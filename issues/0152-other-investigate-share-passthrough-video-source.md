# MP4 パススルーの `VideoTrackSource` を複数 PeerConnection で共有できるかを検討する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/add-share-passthrough-video-source
- Polished: {YYYY-MM-DD}

## 目的

MP4 パススルーで、同じ `Mp4VideoCapturer::video_source()` を複数の PeerConnection の映像トラックに渡せるようにするかを検討し、方針を確定する。

これは libwebrtc では普通の使い方（1 つの `VideoTrackSource` を複数 sink へ fan-out する）である。現状の SDK では debug ビルドで abort し、release でも非対応とドキュメントされている。abort の直接原因は `mp4.rs` の局所バグではなく、次の組み合わせである。

- パススルーがエンコード済みサンプルを `kNative` の `VideoFrameBuffer` に載せて運ぶ
- `shiguredo_webrtc` の native handler がバッファ 1 個を 1 スレッドに固定する
- libwebrtc の `VideoBroadcaster` が同じバッファ実体を複数 encoder スレッドへ配る

方針は 1 つに決めない。`sora-rust-sdk` だけで閉じる案、`shiguredo_webrtc` 側の契約を変える案、共有しないと決めてドキュメントに理由を残す案を並べ、採用するものを後から確定する。

## 優先度根拠

低くてよい。issue 0150 が `Mp4SampleReader` の共有と PeerConnection ごとの `Mp4VideoCapturer` を用意し、同じファイルを独立再生する経路は既にある。sumomo も capturer は接続 1 つにつき 1 つである。

残るのは「1 つの再生を複数 PeerConnection に載せる」という、カメラでは普通だがパススルーでは意味が違う使い方である。回避策があるため、実装の緊急度は高くない。

## 現状

### 公開 API 上の制約

`Mp4VideoCapturer::video_source` は `VideoTrackSource` を clone して返す。`VideoTrackSource::clone` は参照カウントの共有であり、別ソースにはならない。

同じ戻り値を複数 PeerConnection の映像 encoder に渡してはならない、と rustdoc と `docs/INPUT_MP4.md` に書いてある。debug ビルド (`debug_assertions`) では次のメッセージで abort する。

```text
assertion `left == right` failed: video_frame_buffer callback called from multiple threads
```

release ではこの assertion は無い。`VideoFrameBuffer::as_native_ref` の safety コメントは同一実体への同時アクセスを禁止したままなので、release でも非対応である。

回避は PeerConnection ごとに `Mp4VideoCapturer` を分け、`Mp4SampleReader` だけ `clone()` して共有することである。issue 0150 の対象はこちらで、本 issue の対象ではない。

### パススルーのデータ経路

`src/video_codecs/mp4.rs` の経路は次のとおりである。

1. `Mp4VideoCapturer` の feeder スレッドが `Mp4SampleReader::get_sample` で `Mp4EncodedSample` を得る
2. `VideoFrameBuffer::new_with_handler` に載せ、`AdaptedVideoTrackSource::on_frame` する
3. `Mp4PassthroughEncoder::encode` が `VideoFrameBuffer::as_native_ref::<Mp4EncodedSample>` で取り出し、`EncodedImage` にする

`Mp4EncodedSample` は `pub(crate)` の内部型である（issue 0077）。`VideoFrameBufferHandler` として `width` / `height` を返し、`to_i420` は常に `None` である。画素ではない。再エンコードせず bitstream を運ぶための `kNative` である。

`Mp4PassthroughEncoder::get_encoder_info` は `has_trusted_rate_controller(true)` を立てるが、`supports_native_handle` は立てない。既定は false である。

### abort の発生箇所

SDK が固定する `shiguredo_webrtc` は `~0.152.1-canary.0` である。`VideoFrameBufferHandlerState` はバッファ 1 個につき `callback_thread` を持ち、C callback（`type` / `width` / `height` / `ToI420` / `CropAndScale`）から `assert_video_frame_buffer_handler_thread` を呼ぶ。検査は `#[cfg(debug_assertions)]` 付きである。

`VideoFrameBufferHandler` は `Send` であり `Sync` ではない。`to_i420` と `crop_and_scale` は `&mut self` である。C callback は `user_data` を `&mut` で取る。任意の Rust handler の並行呼び出しを安全とみなせないため、最初に触れたスレッドへ固定している。libwebrtc 本体の assert ではない。

`as_native_ref` はこの検査を通らない。`Mp4PassthroughEncoder::encode` だけを見ると abort 理由には見えない。実際に複数スレッドから先に当たるのは、encoder パイプラインが呼ぶ `type` / `width` / `height` である。

### libwebrtc としては共有が普通

同じ `VideoTrackSource` から `SoraConnectionContext::create_video_track` でトラックを複数作り、各 PeerConnection に載せるのは libwebrtc の想定どおりである。`AdaptedVideoTrackSource::on_frame` は内部の `VideoBroadcaster::OnFrame` に入り、登録された全 sink へ同じ `VideoFrame`（同じ `VideoFrameBuffer` の参照カウント）を渡す。各 `VideoSendStream` は自分の encoder キューで `width` / `type` / `Encode` する。

I420 を出す capturer（sumomo の fake、libcamera の非 native 出力）はこの fan-out で問題にならない。画素バッファは libwebrtc 組み込みで、Rust の `VideoFrameBufferHandler` callback を通らない。

`kNative` は libwebrtc が外部実装向けに用意した経路である（`api/video/video_frame_buffer.h`）。カメラのテクスチャや DMA-BUF を encoder までゼロコピーで渡す用途が本命で、エンコード済みサンプルを載せるのはその転用である。native 実装の `width` / `ToI420` / `CropAndScale` は複数スレッドから呼ばれ得る。`shiguredo_webrtc` の単一スレッド固定は、その前提より厳しい。

確認した fan-out の性質（`media/base/video_broadcaster.cc`）:

- 通常フレームは全 sink に同じ `VideoFrame` を渡す（バッファを sink ごとに複製しない）
- `UpdateWants` は `max_pixel_count` と `max_framerate_fps` を全 sink の最小値にする
- 1 つの sink が厳しい wants を出すと、ソース側の `adapt_frame` が全接続に効く

`video/video_stream_encoder.cc` は、crop が必要で、かつバッファが `kNative` でないか `supports_native_handle` が false のとき、`CropAndScale` / `Scale` を呼ぶ。失敗するとそのフレームを落とす。パススルーは `supports_native_handle` を立てず、`Mp4EncodedSample::to_i420` は `None`、`crop_and_scale` も未実装である。解像度が一致して crop が 0 なら単一 PC では通る。共有して wants がずれ crop が付くと、スレッド検査とは別にフレームが落ちる。

### 同じ制約に当たる他経路

本 issue の動機は MP4 パススルーである。同じ `VideoFrameBuffer::new_with_handler` を使う経路は他にもある。

- `src/libcamera.rs` の `LibcameraNativeFrameBuffer`（`native_frame_output`）。カメラは 1 ソースを複数 PeerConnection に載せるのが本命ユースケースに近い。`crop_and_scale` は新しい handler を返すが、元バッファの `width` / `height` callback は残る
- `src/video_codecs/v4l2.rs` の encoder は `as_native_ref::<LibcameraNativeFrameBuffer>` で DMA-BUF を読む。native ソースを複数 encoder に渡すと、パススルーと同じ abort になり得る

I420 に変換して出す経路は本 issue の対象外である。

### 混同してはいけない 2 つの要求

同じ `video_source()` を共有することと、同じ MP4 ファイルを複数接続へ送ることは別である。

| 要求 | 再生時計 | フレーム | wants | 現状 |
|---|---|---|---|---|
| 同じファイルを独立に送る | capturer ごと | 別バッファ | 接続ごと | issue 0150。reader 共有 + capturer 分離 |
| 1 つの再生を複数接続へ載せる | 1 つ | 同じバッファを fan-out | ソースで集約 | 未対応。本 issue の主題 |

zakuro-rs の `--vcs N` × `--input-mp4` は、前者（独立した仮想クライアント）に近い。issue 0150 は汎用 SDK に負荷試験専用 API を置かないと決めた。後者は Web の `MediaStreamTrack` を複数 `RTCPeerConnection` に addTrack する形に近い。

パススルーで後者をやると、次の意味上の問題がスレッド検査とは独立に残る。

- 1 つの `adapt_frame` が全接続のフレームドロップを決める。encoded bitstream は欠けるとデコードが壊れる
- エンコード済みサンプルは `CropAndScale` できない。接続ごとに解像度を変えるカメラ共有とは違う
- キーフレーム要求や再送を無視するパススルー制約が、全接続に同時に効く

「abort さえ消せばカメラと同じ共有でよい」とは限らない。

## 設計方針

未確定。以下は採用候補であり、本 issue の実装着手前に 1 つへ絞る。複数 crate にまたがる案は、`shiguredo_webrtc` 側に別 issue を切ってからでもよい。本リポジトリの issue は傘として残す。

### 候補 A: `shiguredo_webrtc` の native handler を並行読み可能にする

C callback の `type` / `width` / `height` を `&self` 相当にし、単一スレッド検査を緩める。`to_i420` / `crop_and_scale` は Mutex にするか、handler に `Sync` を要求する。`as_native_ref` の safety も並行読みを許すなら書き換える。

- カメラの native 共有（libcamera DMA-BUF）にも効く
- `Mp4EncodedSample` のフィールドは読み取り専用に近い。`to_i420` は `None` で変異しない。データとしては並行読みできる余地がある
- 任意 handler が本当に `Sync` かは分からない。トレイト境界を変えると破壊的変更になり得る
- assertion だけ外して `&mut` callback を並行で残すと、Rust のエイリアシング上は未定義動作のままである
- `sora-rust-sdk` だけでは完結しない。SDK が固定する `shiguredo_webrtc` のバージョン更新が要る

issue 0150 は panic 回避のためにこの検査を緩めないと明示して範囲外にした。

### 候補 B: SDK 側でソースを fork する（feeder 1 本、バッファは接続ごと）

1 つの `Mp4VideoCapturer` が N 個の `AdaptedVideoTrackSource` を持ち、同じサンプルから **別の** `VideoFrameBuffer` を作って各 `on_frame` する。webrtc-rs の検査はバッファ 1 個ごとなので、実体を分ければ abort しない。

- webrtc-rs を変えずに「1 再生を N encoder へ」が実現できる
- サンプルバイトを接続数だけコピーする（`Arc<[u8]>` にしても handler / `VideoFrameBuffer` は別インスタンスが要る）
- 公開 API をどうするかが問題になる。`video_source()` の clone は今も参照カウント共有なので、呼ぶたびに別ソースを返すよう変えると既存の意味が壊れる
- 明示 API（接続ごとにソースを取る）は、issue 0150 が `fork_video_source` として不採用にした形に近い。不採用理由は、開始時刻のずれやファイルが複数ある普通の使い方が例外になる、汎用 SDK としては持たない、であった

「`video_source()` を複数 PC に渡せるようにする」と「fork API を足す」は利用者から見ると違う。前者は libwebrtc の常識、後者は SDK 独自である。

### 候補 C: エンコード済みサンプルの運び方を変える

`kNative` に載せない。dummy の I420 を `on_frame` し、サンプルは timestamp 等で encoder へ別チャネルで渡す、など。

- I420 ソースの共有は今でも動くので、スレッド検査を回避できる
- timestamp 衝突、`adapt_frame` で落ちたフレームとのずれ、encoder 初期化前のサンプル、を自前で扱う
- issue 0077 が `Mp4EncodedSample` を内部化した「native バッファで 3 部品を繋ぐ」設計と逆行する
- パススルー専用の副作用が大きく、libcamera native の共有には効かない

encoded を `FrameTransformer` で注入する案は、現行の transformer が受信トラック向けであり、capturer → encoder の代替にはなっていない。

### 候補 D: 共有しないと決めて、理由をドキュメントに残す

実装しない。issue 0150 の回避（reader 共有 + capturer 分離）を正式解とする。

- 独立再生はパススルーの普通の使い方に合う。接続ごとの時計・`adapt_frame`・欠落しない bitstream を維持できる
- 「`VideoTrackSource` が共有できない」はカメラ / I420 の常識と食い違うので、docs に **なぜパススルーだけ共有しないか**（native の単一スレッド契約、wants 集約、encoded を scale できないこと）を書く。今の docs は abort の話が中心で、意味上の理由が弱い
- libcamera native の共有は別 issue にする。カメラは fan-out が本命なので、候補 A の動機は残る

### 候補を絞るときの観点

1. 利用者が欲しいのは独立再生か、1 再生の fan-out か。前者なら D で足り、後者だけ A か B が要る
2. libcamera native まで同じ解にするか。するなら A が効き、B は MP4 専用になる
3. `shiguredo_webrtc` を今触るか。触るなら A を先にそちらで検討する。SDK だけで閉じるなら B か D
4. `supports_native_handle` と `CropAndScale` を共有と同時に扱うか。abort だけ消しても crop でフレームが落ちる

本 issue で実装に進む場合でも、0150 の「capturer ごとの独立再生」は残す。共有は追加の使い方であり、既存の 1 対 1 と reader 共有を壊さない。

## 完了条件

- 「同じファイルの独立再生」と「1 再生の fan-out」のどちらを本 issue で扱うかを本文に確定してある
- 候補 A / B / C / D（またはその組み合わせ）から採用方針が 1 つに決まっている
- 採用が A を含む場合、`shiguredo_webrtc` 側の対応 issue が切られているか、本 issue にそのリポジトリでの変更範囲が書かれている
- 採用が「共有できるようにする」（A / B / C）の場合:
  - 同じ `Mp4VideoCapturer::video_source()` を複数 PeerConnection の映像 encoder に渡しても、debug ビルドで `video_frame_buffer callback called from multiple threads` が起きない
  - release でも同一 native バッファへの同時アクセスが safety コメントと矛盾しない
  - 既存の 1 対 1 と、reader 共有 + capturer 分離が壊れない
  - I420 を出す capturer のソース共有が壊れない
  - `docs/INPUT_MP4.md` と `Mp4VideoCapturer::video_source` の rustdoc から「共有してはならない」が外れ、新しい使い方が書かれている
  - 実 MP4 fixture で、1 つの `video_source()` から複数トラックがフレームを供給できる回帰がある
- 採用が D の場合:
  - `docs/INPUT_MP4.md` に、abort だけでなくパススルーでソース共有しない意味上の理由が書かれている
  - 実装コードは変えない
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功する（ドキュメントのみの場合は既存が通ること）
- コメントは日本語、ログメッセージは英語、テストの assertion message は日本語で書く
- モックやスタブは使用しない
- コードを変える場合は `CHANGES.md` の develop セクションにエントリを追記する

## 変更対象（方針確定後）

方針未定のため、着手時に次から選ぶ。

- `src/video_codecs/mp4.rs`（`Mp4VideoCapturer` / `Mp4PassthroughEncoder` / `Mp4EncodedSample`）
- `docs/INPUT_MP4.md`
- `Mp4VideoCapturer::video_source` / `Mp4SampleReader` の rustdoc
- `skills/sora-rust-sdk/SKILL.md`（共有手順を書いてある場合）
- `CHANGES.md`（実装する場合）
- `shiguredo_webrtc` の `VideoFrameBufferHandler` / `VideoFrameBufferHandlerState` / `as_native_ref`（候補 A）
- `src/libcamera.rs`（native 共有まで範囲に含める場合。含めないなら別 issue）

## 関連

- issue 0150: `Mp4SampleReader` の共有。`video_source()` の複数 encoder への配給は非対応のままにし、本 issue へ先送りした
- issue 0077: `Mp4EncodedSample` を公開 API から外し、native `VideoFrameBuffer` 経由の内部受け渡しに固定した
- `shiguredo_webrtc` 0.152.1-canary.0 の `VideoFrameBufferHandlerState::callback_thread`
