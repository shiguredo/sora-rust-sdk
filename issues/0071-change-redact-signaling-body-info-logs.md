# シグナリング / DataChannel 本文の info ログを削減または redact し、機密情報の平文流出を防ぐ

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-redact-signaling-body-info-logs
- Polished: 2026-07-27

## 目的

シグナリング / DataChannel の送受信本文を `rtc_log_info!` で **本文丸ごと** 出力している箇所が複数あり、以下の機密情報が info レベルで流出する:

- `connect` メッセージの `metadata` (認証トークンをアプリが載せる想定)
- `connect` メッセージの `signaling_notify_metadata`
- offer / re-offer の `config.iceServers.credential` (TURN パスワード) と `username`
- notify / push メッセージ (認証結果、参加者メタデータ)
- RPC の `params` / `result` / `error.data` (任意 JSON)

info レベルでの本文垂れ流しを止め、ログ集約先への平文流出を防ぐ。

## 優先度根拠

High (セキュリティ致命)。ログ集約先 (CloudWatch / Loki / ELK 等) に流れれば本番運用でセキュリティインシデントに直結する。SDK 利用者は `on_signaling_message` / `on_message` などのハンドラで自前ログを持てるため、SDK 側の info ダンプは重複でありセキュリティ観点でも不要。

## 現状

該当箇所:

- `src/connection.rs:1064` `rtc_log_info!("[WebSocket] Received text message: {}", text);` — 受信した全テキストメッセージ（offer / re-offer / notify / push / ping / redirect / close 等）を全文ログ。offer の `iceServers.credential` (TURN パスワード) や notify / push のメタデータが平文出力される
- `src/connection.rs:1158-1161` `rtc_log_info!("[WebSocket] バイナリメッセージを受信しました: {} bytes", data.len());` — 本文は出力しておらず（`data.len()` のみ）、機密情報の平文流出には該当しない。ログ規約違反（日本語）の是正のみ必要
- `src/connection.rs:1723` `rtc_log_info!("Sent message to DataChannel '{}': {}", label, &message);` — シグナリング（`send_signaling_message` 経由）/ stats（`send_stats_message` 経由）/ RPC の DataChannel 送信本文を全文ログ
- `src/connection.rs:1774` `rtc_log_info!("DataChannel '{}' からメッセージを受信: {}", label, String::from_utf8_lossy(&message_bytes));` — 全 DataChannel ラベル（`"signaling"`（ReOffer の TURN credential を含む）/ `"push"` / `"notify"` / `"rpc"` / `"#xxx"` 等のユーザー定義ラベル）の受信本文を全文ログ
- `src/connection.rs:2567` `rtc_log_info!("[WebSocket] Sent text message: {}", text);` — `send_text` 関数内で呼ばれ、WebSocket 経由の全送信テキスト（初回 connect / answer / reanswer / ユーザー任意送信シグナリング（`SoraEvent::SignalingMessage` 経由））を全文ログ。connect の `metadata` / `signaling_notify_metadata` を含むすべての送信テキストが平文出力される

## 設計方針

**方針: 案 A（本文ログをすべて削除）を採用する。** ユーザーが必要なら `on_signaling_message` / `on_message` ハンドラで同等の情報を取得できるため、SDK 側の info ダンプは重複でありセキュリティリスクに対して不要。

他案の評価:

- **案 B**: `rtc_log_info!` → `rtc_log_debug!`（もしくは `trace`）に降格し、message type と長さだけを info で残す。運用者が info で気軽に監視できる利点はあるが、debug レベルであっても平文出力が残るため、機密漏洩リスクを完全には除去できない。
- **案 C**: JSON パース後に `metadata` / `signaling_notify_metadata` / `credential` / `params` を `<redacted>` に置換した文字列を組み立ててからログする。実装コストが高く、フィールド追加時に追従漏れのリスクがある。`connection.rs` のログ層で JSON パースによる完全 redact は過剰。

なお、`src/connection.rs:1158-1161` のバイナリメッセージログは本文を出力しておらず、機密漏洩対象外のため、削除ではなくログ言語を英語に修正する。
同様に `src/connection.rs:1745-1749`（`send_data_channel_message` 内の日本語バイナリ送信ログ）も本文は出力しておらず、機密漏洩対象外のため、本 issue では扱わない。

`Ping` / `Pong` / `StateChanged` などの低頻度制御メッセージのログは本 issue の対象外とし、別 issue で整理する。

## 完了条件

- シグナリング / DataChannel 本文が info レベルで平文出力されない（該当行 1064, 1723, 1774, 2567 の `rtc_log_info!` を削除することで達成する）。
- ユーザーが必要なら `on_signaling_message` / `on_message` / `on_notify` / `on_push` / `on_data_channel_message` のハンドラで同等の情報を取得できることを `skills/sora-rust-sdk/SKILL.md` および `SoraConnectionEventHandler` トレイトの rustdoc に明記する。
- `cargo test --workspace --all-features` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
