# libcamera 停止時のバッファ寿命を保証する

- Priority: High
- Created: 2026-07-29
- Completed: 2026-08-01
- Model: GPT-5
- Branch: feature/fix-iroiro3
- Polished: 2026-07-29

## 目的

libcamera の停止完了を確認してから capture session のリソースを破棄し、native frame には独立した DMA-BUF の所有権を持たせることで、停止時のクラッシュや無効な file descriptor 参照を防ぐ。

## 優先度根拠

High。停止失敗を無視したまま libcamera のリソースを破棄する経路と、下流が借用した DMA-BUF file descriptor より allocator が先に破棄される経路がある。
前者は libcamera worker による解放済み FrameBuffer へのアクセス、後者は V4L2 の非同期変換による閉じた file descriptor の利用につながる。

## 現状

`run_libcamera_loop` は `camera.stop()` の結果を破棄する。
その直後に `Request`、`FrameBufferAllocator`、`Camera`、`CameraManager` が破棄されるため、停止失敗時は native callback や queued request の終了を保証できない。

`shiguredo_libcamera` 2026.1.0 の `FrameBuffer` は allocator が所有するバッファへの非所有参照であり、plane の file descriptor も借用値として返す。
`LibcameraNativeFrameBuffer` と、その clone を保持する V4L2 converter は raw file descriptor と requeue token だけを保持しており、allocator の破棄後まで DMA-BUF の寿命を延長できない。

また、capture thread は `run_libcamera_loop` のエラーをログだけにして破棄し、公開 `LibcameraVideoCapturer::stop()` も `()` を返して join error を破棄する。
このため、現状の API では利用者が停止失敗を検出できない。

## 設計方針

### Native frame の DMA-BUF 所有権

- allocator から取得した各 DMA-BUF file descriptor を `F_DUPFD_CLOEXEC` 相当で 1 回複製し、`OwnedFd` として所有する
- `NativeFrameInfo` と `LibcameraNativeFrameBuffer` は raw `i32` ではなく `Arc<OwnedFd>` を保持する
- `LibcameraNativeFrameBuffer::fd()` は、保持する `OwnedFd` の `as_raw_fd()` を返す既存シグネチャを維持する
- `crop_and_scale()` と V4L2 converter へ渡す clone は同じ `Arc<OwnedFd>` を保持し、最後の clone が破棄されるまで DMA-BUF を閉じない
- file descriptor の複製に失敗した場合は native frame の初期化を失敗させ、借用した raw file descriptor へ fallback しない

Linux の DMA-BUF は file の参照カウントによって backing storage の寿命を管理するため、複製した file descriptor が残る間は allocator が元の file descriptor を閉じても buffer は解放されない。
この所有権により、公開され得る native frame の返却を `stop()` が無期限に待つ必要はなくなる。

### Capture session の停止

- `CameraManager`、`Camera`、request completion callback、`CameraConfiguration`、`Request`、`FrameBufferAllocator`、captured buffer 情報を、停止処理まで一体で所有する private capture session にまとめる
- capture session は `NotStarted`、`Running`、`StoppedSafely`、`Quarantined` の内部状態を持ち、resource を通常の field drop に任せない fail-closed guard として実装する
  - `camera.start()` の成功後にだけ `Running` へ遷移する
  - `NotStarted` での設定、DMA-BUF 複製、allocate、request 構築、`camera.start()` の失敗は、未 queue の request、allocator、captured buffer 情報、configuration を破棄してから `camera.release()` を呼び、camera、manager の順に通常どおり破棄する
  - `Running` の session が panic で unwind した場合は custom `Drop` が resource を破棄せず、session 全体を `Quarantined` 相当としてプロセス終了まで隔離保持する
  - `camera.stop()` の成功直後に `StoppedSafely` へ遷移し、この状態でだけ依存 resource の明示的な Drop を許可する
- shutdown の順序を次に固定する
  1. stop flag を検出して新規 frame dispatch と requeue を終了する
  2. request、allocator、callback を保持したまま `camera.stop()` を同期的に呼ぶ
  3. `camera.stop()` が成功した場合だけ request と allocator を破棄する
  4. `camera.release()` を呼ぶ
  5. camera と manager を破棄する
- `camera.stop()` が失敗した場合は、非同期アクセスの終了を仮定しない
  - `camera.release()` を呼ばない
  - session の構成要素を unwind で破棄しない
  - manager、camera、callback、request、allocator を含む session 全体をプロセス終了まで隔離保持する fail-closed 経路へ移す
  - stop error を利用者へ返す
- capture loop と `camera.stop()` の両方が失敗した場合は、安全性に直結する stop error を返し、capture loop error は英語でログ出力する
- `camera.stop()` が成功した場合は、`camera.release()` error を capture loop error より優先する
  - 両方が失敗した場合は release error を返し、capture loop error を英語でログ出力する

stop 失敗後の隔離保持は回復処理ではなく、解放済み native resource へのアクセスを防ぐ最終安全策とする。
正常な stop では隔離や resource leak を発生させない。
panic は capture thread の stack が unwind した後に join 側で検出されるため、join error の変換ではなく capture session 自身の custom `Drop` で隔離を保証する。

### 公開 API へのエラー伝播

- capture thread を `JoinHandle<Result<()>>` とし、thread 内で `run_libcamera_loop` のエラーを破棄しない
- `LibcameraVideoCapturer::stop(&mut self) -> Result<()>` に変更し、次を利用者へ返す
  - capture loop、`camera.stop()`、`camera.release()` のエラー
  - capture thread が panic した場合の専用エラー
- capture thread が存在しない状態での `stop()` は `Ok(())` とする
- `Drop` は同じ安全な停止処理を呼ぶがエラーを返せないため、失敗を英語でログ出力する
- `LibcameraVideoCapturer` は `Idle`、`Running`、`Poisoned` の状態を保持する
  - `start()` の thread spawn が成功したときだけ `Running` へ遷移する
  - 明示的な `stop()` が成功したときだけ `Idle` へ戻る
  - capture thread が `Err` を返した場合または panic した場合は `Poisoned` へ遷移する
  - `Poisoned` 後の `start()` と `stop()` は専用エラーを返し、再 start や二重 shutdown を行わない
- public API の破壊的変更として `CHANGES.md` の develop セクションへ `[CHANGE]` を追記する

## 変更対象

- `src/libcamera.rs`
- `src/error.rs`
- `e2e-tests/tests/libcamera_video_capturer.rs`
- `e2e-tests/tests/v4l2_video_codec.rs`
- `CHANGES.md`

## 完了条件

- `camera.start()` の成功後は、`camera.stop()` の成功前に request、allocator、callback、camera、manager のいずれも破棄されない
- `camera.start()` より前の初期化失敗と `camera.start()` 自体の失敗では、未 queue の依存 resource を破棄してから camera を release し、camera、manager の順に通常どおり破棄する
- `camera.stop()` の失敗時は session 全体が隔離保持され、`camera.release()` や通常の Drop が実行されない
- `Running` 中の panic unwind でも session 全体が隔離保持され、通常の Drop が実行されない
- 明示的な `LibcameraVideoCapturer::stop()` が capture thread の停止結果を返し、失敗を成功として扱わない
- capture thread の `Err` または panic 後は capturer が `Poisoned` になり、以後の `start()` と `stop()` が専用エラーを返す
- `Drop` 時の停止失敗が英語でログ出力される
- native frame とその全 clone が同じ `Arc<OwnedFd>` を保持し、allocator が所有する raw file descriptor を直接所有したものとして扱わない
- 複製元の file descriptor を閉じた後も、最後の native frame clone を破棄するまで複製 file descriptor が有効であることを、実 OS file descriptor を使う単体テストで確認する
- `crop_and_scale()` と V4L2 converter の非同期 callback value が `Arc<OwnedFd>` の寿命を維持する
- production で利用する純粋な状態遷移・エラー優先順位 helper を単体テストし、`Idle`、`Running`、`Poisoned` の全遷移と複数エラーの優先順位を確認する
- fail-closed guard が `Running` のまま Drop された場合は内部 resource を通常 Drop せず、`StoppedSafely` の場合だけ定義済みの順序で明示的に Drop する構造であることをコードレビューで確認する
- モック、スタブ、Drop probe、`catch_unwind` は使わない
- Raspberry Pi の実機上で `libcamera,v4l2` feature を有効にし、`native_frame_output(true)` と実 V4L2 converter / encoder を使う
- 接続と送信を継続したまま同じ capturer の `stop()` と `start()` を 20 回繰り返し、各回で次を確認する
  - `stop()` が `Ok(())` を返す
  - `start()` が `Ok(())` を返す
  - stop 後に stats が増加しなくなった値を baseline とする
  - 再 start 後に outbound-rtp の `framesEncoded` と `packetsSent` が baseline から 30 以上増加する
  - inbound-rtp の `framesDecoded` と `packetsReceived` が baseline から 30 以上増加する
- 最終反復後にも明示的な `stop()` が `Ok(())` を返す
- 全 call site が `stop()` の `Result` を明示的に検証する
- 必要な実機機能がない場合に skip や成功扱いにせず、テストを失敗させる
- 通常環境で `cargo test --workspace` が成功する
- Raspberry Pi 実機で次が成功する
  - `cargo test -p sora_sdk --features libcamera,v4l2`
  - `cargo test -p e2e-tests --features libcamera,v4l2 --test libcamera_video_capturer --test v4l2_video_codec`

## 解決方法

対応範囲を検討した結果、設計方針のうち「Native frame の DMA-BUF 所有権」のみを実装し、「Capture session の停止」と「公開 API へのエラー伝播」は対応しないで closed にした。

- 各 DMA-BUF fd を `F_DUPFD_CLOEXEC` で複製し、`Arc<OwnedFd>` として所有する
- `NativeFrameInfo` と `LibcameraNativeFrameBuffer` は複製した fd を保持し、`crop_and_scale()` や V4L2 converter の clone が同じ fd を共有する
- allocator が元の fd を閉じても、最後の native frame clone が破棄されるまで複製 fd が有効なままである
- 実 OS fd を使う単体テストで、元の fd を閉じた後に複製 fd が有効なままであることを確認する

「Capture session の停止」と「公開 API へのエラー伝播」を対象外にした理由は次のとおり。

- `camera.stop()` 失敗時の終了保証は原理的に不可能で、libcamera に強制停止 API はない。失敗後の安全な遷移は隔離かリトライのみであり、fail-closed の隔離は回復処理ではなく稀な失敗経路の最終安全策に過ぎない
- `stop()` が失敗した時点で capturer は実質死亡しており、失敗を利用者へ返しても取れる行動がない。public API の破壊的変更と二重の状態機械を導入するコストに見合わない
- 通常の停止では `camera.stop()` の成功後に resource を破棄する順序が既に維持されており、fd の寿命延長と組み合わせることで停止時のクラッシュは防げる
