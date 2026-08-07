# sumomo の capture format と変換処理を一致させる

- Priority: Medium
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-sumomo-capture-format
- Polished: 2026-08-07

## 目的

video device が選択する pixel format を sumomo の変換可能な形式（NV12 / YUY2 / I420）でカバーし、変換できない format が選択された場合に全 frame が無言で破棄される状態を防ぐ。

## 優先度根拠

Medium。バックエンドと device に依存するが、Windows (Media Foundation) では NV12 / YUY2 の双方が非対応の device で I420 が選択され得る。PipeWire（feature 有効時）のネゴシエーションも I420 を含む。
現状の callback は I420 を変換せず無言で破棄するため、該当する環境では映像が送信されない。

## 現状

`VideoDeviceCapturer::new` は `pixel_format: None` を指定し、device 側の選択に任せる。
capture callback は NV12 と YUY2 だけを変換し、それ以外の format を通知なしで破棄する。
また NV12 / YUY2 の変換関数が false を返した場合（バッファ不足等）も、無言で `return` して破棄する。

バックエンドの既定ネゴシエーション（shiguredo_video_device 2026.2.0。sumomo は default features で利用）:

- V4L2（Linux 既定）: None 時は NV12 → YUY2 のフォールバックのみ。どちらも不可の device は `VideoCapture::new` で開始前に失敗する。I420 は選択されない（YUV420 専用 device は本 issue の対応後も `VideoCapture::new` で失敗し続ける。無言破棄ではないため対象外）。
- Media Foundation（Windows）: None 時は NV12 → YUY2 → I420 を試し、すべて不可なら native format にフォールバックする。I420 は現状の callback で破棄される。native が NV12 / YUY2 / I420 以外の場合は `VideoCapture::new` で失敗する（callback には届かない）。
- AVFoundation（macOS）: None 時は NV12 に正規化される。
- PipeWire: sumomo は pipewire 系 feature を有効化していないため現構成では到達不能。有効化した場合（shiguredo_video_device の default features から `default-v4l2` を外して `default-pipewire` を有効にした場合。既定バックエンドは 1 つだけ選ぶ）、ネゴシエーションは NV12 / YUY2 / I420 の choice で行われ、I420 が選択され得る。
- MJPEG は sumomo の構成では選択されない（`mjpeg` feature が無効）。

## 設計方針

- 変換可能な format を NV12、YUY2、I420 に定め、選択され得る全 format を変換実装がカバーする
  - I420 は Y プレーンと連結 UV（U プレーン + V プレーン）を、`frame.uv_data` を `stride_uv * ceil(height / 2)` で U / V に分割し、`shiguredo_webrtc::i420_copy`（U / V を別引数で受ける）で `I420Buffer` へコピーする
  - 連結 UV のレイアウト前提は `shiguredo_video_device` の `frame_math::i420_plane_sizes` の式（U 連結 V、U / V とも `stride_uv * ceil(height / 2)`）と整合する。MF は `capture_mf.rs` の `process_sample` が stride = width / stride_uv = ceil(width / 2) の packed 形へ正規化してから配信し、AVF も `video_avf.m` の I420 分岐が同じ連結 UV バッファを作る（どちらも `Some(I420)` 明示時のみ。sumomo の `None` 構成では MF だけが I420 を配信し得る）。同関数は `pub(crate)` で sumomo からは参照できないため、helper は式を自前実装する。helper は `frame.stride_uv` を使う（AVF の padding で ceil(width / 2) と一致しない場合があるため、width から再計算しない）
  - PipeWire は「連結 UV の実バイト数やストライドの解釈がこの式と一致しない場合がある」と同 crate が明言しており、現構成では到達不能のため、PipeWire 有効化時には変換 helper を実 device で検証し直す（本 issue では PipeWire の検証は対象外）
  - `uv_data` が `None` の I420 frame は変換失敗として扱う。YUY2 は packed 形式のため `uv_data` が常に `None` であり、これは正常系（`uv_data` なしで変換する）
- `VideoCaptureConfig::pixel_format` は None のまま、バックエンドの既定ネゴシエーションに任せる
  - V4L2 / MF / AVF は各バックエンドの固定ネゴシエーションリスト（現状セクション参照）に合致する format を返すか、合致しない device は `VideoCapture::new` で失敗する（shiguredo_video_device 2026.2.0 の内部挙動であり、sumomo の変更では制御しない。依存バージョン更新時はこの前提が変わっていないか再確認する）
  - PipeWire は変換可能な raw format の choice を返すか、`start()` で失敗する
  - `Some(Nv12)` への固定は YUY2 のみ対応の device を回帰させ、PipeWire では choice が常に YUY2 / I420 を含むため I420 選択も防げない。採用しない
- capture callback は NV12、YUY2、I420 を変換し、それ以外の format と変換失敗は英語のエラーログを出して破棄する（無言破棄をやめる）
  - callback は FFI 境界（V4L2 / AVF / PipeWire）または Rust thread（MF）から呼ばれる。panic すると FFI 越えの abort や capture thread の永続喪失になるため、本 issue で追加する変換 helper はパニックしない（境界チェック済みの total な関数）こと。既存の callback 内処理（`adapt_frame` 等）も同様に FFI 境界上で動くが、本 issue では変更しない（panic 危険は既存のまま。helper だけを total にしても callback 全体の panic 耐性は完成しない点に注意）
  - エラーログには pixel format 名と幅・高さに加え、変換失敗時はバッファの実測長と期待長を含める（原因特定のため）。`PixelFormat::Unknown` は生値（FourCC）も出す（`.name()` は `"Unknown"` を返すだけで原因特定にならないため）。非対応 format のエラーログは防御的な分岐であり、現構成の全バックエンドでは callback 到達前に format が除去されるため実動では発火しない（将来の shiguredo_video_device 更新時の保険）
  - エラーログ（非対応 format と変換失敗の両方）は連続で出ないようレート制限する（2 秒ごとに 1 回を既定値とする。「1 回目だけ」は恒久的な失敗で無言破棄に戻るため不採用。2 秒はログ量と失敗兆候の検知速度のバランスで決めた値）。間隔は定数（例: `LOG_RATE_LIMIT_MS`）として 1 箇所に定義する。非対応 format と変換失敗は同じカウンタを共有してよい（どちらも capture 異常の兆候）。callback は `Fn` クロージャ（`FnMut` ではない）のため、レート制限の状態は `Arc<AtomicI64>` などの interior mutability で持つ（callback へ move して capture と同じ寿命にする）。レート制限の判定（現在時刻・前回ログ時刻・間隔から出力可否を返す）は pure 関数に切り出して単体テストする。クロックは `shiguredo_webrtc::time_millis()`（i64 ミリ秒）を使い、前回ログ時刻の初期値は 0（未ログのセンチネル）とし、pure 関数は `last == 0`（未ログ）のときは常に出力する（`now - last >= interval` の判定だけでは、初回の失敗が interval 未満の時刻で起きた場合に抑圧されるため。実クロックは十分大きな正値だが、単体テストの小さな `now` でも保証が成立するようセンチネルを特別扱いする）。これにより初回の失敗は必ずログする（恒久的な失敗を無言破棄に戻さない）
  - callback 内の他の無言破棄（`shared_clone.lock()` 失敗、`adapt_frame` の `!applied`）は本 issue の対象外
- frame 変換を callback から pure helper に切り出し、単体テスト可能にする
  - helper は `fn convert_frame(&shiguredo_video_device::VideoFrame<'_>) -> Result<shiguredo_webrtc::I420Buffer, ConvertError>` の単一関数とし、`pixel_format` を内部で match して非対応 format を `Err` で拒否する（非対応 format と変換失敗を区別する error variant を持つ）
  - 変換失敗 variant は実測長・期待長・`PixelFormat` 値・失敗箇所（Y プレーン / UV プレーン / オーバーフロー）を保持し、callback はログ出力時にこれを使う（callback 側で期待長を再計算しない。`PixelFormat` 値を持たせれば `Unknown` の FourCC もログに出すことができ、失敗箇所でどちらのプレーンが短いかも分かる）。オーバーフロー失敗時は長さ（実測長・期待長）が計算できないため、代わりに失敗した次元値（width / height / stride）を保持する（オーバーフロー variant を分離するか、長さを `Option` にする）
  - total 性のため、`I420Buffer::new` の前に width / height / stride が正の値であることを検証し、非正の値は `Err` にする（`I420Buffer::new` は不正な次元で panic し得るため）。stride_uv の正値検証は NV12 / I420 分岐のみで行う（YUY2 は packed 形式のため stride_uv == 0 が契約）。変換前の長さ検証は `checked_mul` / `checked_add` で行い、`nv12_to_i420` / `yuy2_to_i420` / `i420_copy` が内部で使う必要長と同じ式（`stride * (rows - 1) + row_bytes`、libyuv.rs の `has_required_len` と同じ基準）で Y プレーン・UV プレーンの不足とオーバーフローを判定して `Err` にし、`split_at` ではパニックしない。I420 の U / V 分割は各 `stride_uv * ceil(height / 2)` で行う。`uv_data` は分割前に全体長（`2 * stride_uv * ceil(height / 2)`）でも検証する（libyuv の必要長式は無 padding（stride_uv == ceil(width / 2)）のときだけ分割境界と一致し、padding のある stride_uv では分割境界より小さい値を返すため、`split_at` の安全は分割境界の検証で保証する）。NV12 の UV 長は `stride_uv * ceil(height / 2)`、YUY2 は `frame.stride * frame.height` を基準にする。packed 式（`stride * rows`）を基準にすると奇数幅 NV12 のように libyuv 内部検証にだけ引っかかるケースで「実測 = 期待」の無意味なログになるため、変換失敗 variant の期待長は、失敗した検証に対応する必要長（I420 の分割境界または libyuv の必要長式）を使う。長さ検証が実測バッファとの整合を保証するため、`I420Buffer::new` は実 device 由来の次元では panic しない。`nv12_to_i420` / `yuy2_to_i420` / `i420_copy` が false を返した場合も `Err` に変換する。I420 の変換パターンは `src/video_codecs/v4l2.rs` の `build_i420_frame` を参照する（checked arithmetic の実装パターンのみを参照する。分割式は本 issue の方針どおり `frame.stride_uv` ベースであり、`build_i420_frame` の `stride.div_ceil(2)` による再計算は使わない）
  - 奇数幅 NV12 は長さ検証（`2 * ceil(width / 2)` が stride_uv（== width）を超えるため、libyuv の必要長式で UV 不足になる）により変換失敗になる既知のケースであり、`Err` を返すことを単体テストで固定する（uv_data が無 padding の `stride_uv * ceil(height / 2)` である前提。この前提は MF / V4L2 / PipeWire の正規化に基づく。本 issue の変更で無言破棄からログ付き破棄に変わる）
  - Nv12 の `uv_data` が `None` の場合は現状と同じく変換失敗として扱う
  - helper は `#[cfg(feature = "media-device")]` でゲートし、単体テストは video_device.rs 内の `#[cfg(all(test, feature = "media-device"))]` モジュールに置く（shiguredo-rust 規約: private 実装のテストはモジュール内に書く）

## 変更対象

- `examples/sumomo/src/video_device.rs`
  - capture callback（I420 変換の追加、エラーログ）
  - frame 変換の pure helper（新規）とその `#[cfg(all(test, feature = "media-device"))]` 単体テスト
- `CHANGES.md`

media-device のテストは CI に job が無いためローカル検証のみ（Linux では `--features media-device` のビルドに `libpulse-dev` が必要。`libpipewire-0.3-dev` は sumomo の構成では不要）。CI job の追加は本 issue の対象外とする。

## 完了条件

- 現構成で選択され得る capture format（NV12 / YUY2 / I420）のすべてを変換実装がカバーする（PipeWire は現構成で到達不能のため対象外）
- 変換できない format や変換失敗は無言で破棄せず、英語のエラーログを出す（レート制限付き）
- frame 変換の pure helper の単体テストで、NV12 / YUY2 / I420 の変換、非対応 format の拒否、変換失敗（バッファ不足・`uv_data` なし）を検証する
  - 変換後の `I420Buffer` の U / V プレーン内容を assert する（Y / U / V を区別できる定数値で入力を作り、U / V 分割の反転や `ceil` 切り上げの誤りを検出する）
  - 奇数幅・奇数高さの I420 が変換できること（`i420_copy` の chroma 切り上げと helper の分割式が一致すること）を検証する
  - 奇数幅 NV12 は変換失敗（`Err`）になることを検証する
- レート制限の判定を pure 関数に切り出し、単体テストで「初回の失敗は必ず出力され、間隔内の連続失敗は抑圧され、間隔経過後に再び出力される」ことを検証する
  - `ConvertError` は `video_device.rs` 内の private 型とし、`Copy` 可能な形（`PixelFormat` は Copy、長さは数値）にする。非対応 format variant と変換失敗 variant の両方が `PixelFormat` 値を保持する
- 実 device で、利用するバックエンド（開発マシンの OS に応じたバックエンド）の少なくとも 1 つの format（NV12 / YUY2）を手動確認する。手順は `--video-input-device <device-id>` を指定して送信し、NV12 / YUY2 の変換が従来どおり動作する（映像が届く）ことで回帰がないことを確認する。カメラや video device が無い環境では、Windows の I420 と同様に単体テスト + code review で代替する
- Windows 実機で I420 がネゴシエーションされる device がある場合は、変換成功（エラーログが出ず映像が送られる）ことを確認する。I420 をネゴシエーションできる device が無い場合は、単体テスト + code review で代替する（I420 は現構成の MF 固有のため、pure helper の単体テストで担保する）。helper のレイアウト前提が `capture_mf.rs` / `video_avf.m` の連結 UV 正規化契約（U 連結 V、`frame.stride_uv` を width から再計算しない）に依存する旨は、実機確認の有無に関わらず production comment に明記する（将来の依存更新や他 device で壊れ得る恒久的な依存であるため）
- callback のエラーログ・レート制限・helper 呼び出しの wiring は、pure 関数と helper の単体テストでは検証できないため、code review で確認する旨を PR に明記する
- `cargo test -p sumomo --features media-device` と `cargo clippy -p sumomo --features media-device --all-targets -- -D warnings` が成功する
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `cargo fmt --all --check` が成功する
- `CHANGES.md` の develop セクションへ `[FIX]` と担当者 `@voluntas` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
