# ロガー API を追加する

- Created: 2026-10-05
- Completed: {YYYY-MM-DD}
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
