# ロガー API を追加する

- Created: 2026-10-05
- Completed: 2026-10-05
- Branch: feature/add-logger-api
- Polished: {YYYY-MM-DD}

## 目的

sora_sdk と libwebrtc のログを利用側から取得できるようにする。各言語 SDK (Swift / Kotlin / Python) がアプリのログ機構 (os.Logger / logcat / logging) へ転送して、実機で接続トラブル (ICE / TLS / シグナリング) の原因を追えるようにする。

## 現状

- sora_sdk のログは shiguredo_webrtc の `rtc_log_info!` / `rtc_log_warning!` / `rtc_log_error!` などのマクロで出力しており、libwebrtc のログ機構に乗っている
- shiguredo_webrtc には `log::LoggingConfig` (最低レベル / debug レベル / タイムスタンプ / スレッド名 / stderr 出力 / プレフィックス)、`log::LogSink` と `log::LogSinkHandler` (ログ 1 行ごとのコールバック)、`log::initialize_logging` が公開されており、webrtc-rs 側の追加実装は不要
- sora_sdk はこれらを公開 API にしておらず、ログレベルや出力先を設定する手段が無い
- 各言語 SDK も `initialize_logging` を呼んでおらず、ログ sink も登録していない。実機ではログが実質見えず、Kotlin は自前で生成したエラーメッセージだけを logcat へ出力している
- Sora Swift SDK のロードマップには「ロガー (Rust コアのログを Swift へ転送する仕組み)」が未着手で残っている

## 設計方針

- sora_sdk にログ設定とログ受け取りの公開 API を追加する (shiguredo_webrtc の `log` モジュールをラップする)
  - ログレベル (最低レベル) の指定
  - ログ 1 行ごとに呼ばれるコールバック (sink) の登録
- ログ 1 行の情報は sora_sdk の型として定義し、公開 API に shiguredo_webrtc の型は出さない
- libwebrtc のログ sink は signaling / worker / network などの複数スレッドからロックを取らずに呼ばれる。コールバックのスレッドセーフと、呼び出し側が重い処理をしない前提をドキュメントに明記する
- ログ設定は `initialize_logging` と同じプロセス単位とする (接続単位ではない)
- 未設定時の既定挙動を定義する (現状の stderr 出力を維持するなど)
- 機密情報の redact (signaling URL の query 除去など、実装済みの内容) は維持する
- 各言語 SDK への橋渡し (os.Logger / logcat / logging モジュール) は各 SDK 側の作業とし、この issue では扱わない

## 完了条件

- ログレベルを指定できる
- ログ 1 行ごとにメッセージ / severity / タイムスタンプなどをコールバックで受け取れる
- sora_sdk のログと libwebrtc のログ (`RTC_LOG`) の両方が同じ経路で受け取れる
- 未設定時の既定挙動が定義されている
- `skills/sora-rust-sdk/SKILL.md` に API が追記されている

## 変更対象

- `src/logging.rs` (新規) と `src/lib.rs` (公開)
- `skills/sora-rust-sdk/SKILL.md`

## 解決方法

実装を行わない。目的・現状・設計方針の前提が成立していないと判断し、closed にする。

### 前提 1: sora_sdk 利用者にログ設定・取得の手段が既にある

- `shiguredo_webrtc` 0.154.1-canary.3 (本リポジトリの Cargo.toml で依存) は `log` モジュールを公開している (`src/rtc_base/logging.rs` の `LoggingConfig` / `Severity` / `LogSink` / `LogSinkHandler` / `initialize_logging` / `print`。`src/lib.rs` が `pub use rtc_base::{…, log, …}` しているため `shiguredo_webrtc::log::*` で利用できる)。
- sora_sdk の利用者は公開 API で `shiguredo_webrtc` の型 (例: `RtpTransceiver` / `AudioDeviceModule`) を受け取るため、必ず `shiguredo_webrtc` に依存する (`skills/sora-rust-sdk/SKILL.md`「利用側の `Cargo.toml` に `shiguredo_webrtc` を追加する必要がある」)。
- 実際に、本リポジトリの `examples/sumomo/src/main.rs` と、sora_sdk の実利用者である zakuro-rs の `src/main.rs` は `log::LoggingConfig::new()` / `log::initialize_logging()` を直接呼んでおり、ログレベル・タイムスタンプ・スレッド ID の設定と sink 登録が既に可能である。
- したがって「sora_sdk は…ログレベルや出力先を設定する手段が無い」という現状認識は成立しない。

### 前提 2: 各言語 SDK が sora_sdk の API を経由する想定が実状と異なる

- Swift (sora-ios-sdk): sora-rust-sdk を利用していない。`Sora.swift` に `Sora.setWebRTCLogLevel(_: RTCLoggingSeverity)` (`RTCCallbackLogger`) が既にあり、libwebrtc のログを取得できる。バイナリは webrtc-rs の C API (libwebrtc_c.xcframework) へ移行する方針で、ロガーは C API の `webrtc_LogSink` / `webrtc_LoggingConfig_AddSink` で実装すると移行 issue 0070 に明記されている。「Sora Swift SDK のロードマップに…未着手で残っている」という記述は sora-ios-sdk の実状態と整合しない。
- Kotlin (sora-android-sdk): sora-rust-sdk を利用していない。`SoraLogger` (Kotlin) で自 SDK のログを logcat へ出力しており、libwebrtc-c への書き換えは調査段階 (issue 0061) である。
- Python (sora-python-sdk): sora-rust-sdk + PyO3 へ移行中だが、移行設計 (`docs/migration-to-sora-rust-sdk.md`) はログの受け口として `shiguredo_webrtc::log` を直接バインディングすると明記しており、sora_sdk へのラッパー API 追加を必要としていない。

### 補足: 設計方針が現行 API 方針と矛盾

- 「公開 API に shiguredo_webrtc の型は出さない」は現行慣行と異なる。`SoraConnectionEventHandler::on_track` は `shiguredo_webrtc::RtpTransceiver` を直接受けるなど、公開 API に `shiguredo_webrtc` の型が多数出ている。

以上より、この issue が解決を目指していた問題は現行実装ですでに解消可能か、前提が成立しないものであり、対応は不要と判断した。
