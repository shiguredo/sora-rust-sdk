# シグナリング / DataChannel 本文の info ログを削減または redact し、機密情報の平文流出を防ぐ

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-redact-signaling-body-info-logs
- Polished: {YYYY-MM-DD}

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

- `src/connection.rs:1064` `rtc_log_info!("[WebSocket] Received text message: {}", text);` — offer 応答が全文ログ、TURN credential も出る
- `src/connection.rs:1158-1161` `rtc_log_info!("[WebSocket] バイナリメッセージを受信しました: {} bytes", data.len());` (ログ規約違反もあり、issue 0110 と重複対応)
- `src/connection.rs:1723` `rtc_log_info!("Sent message to DataChannel '{}': {}", label, &message);` — RPC params 全文
- `src/connection.rs:1774` `rtc_log_info!("DataChannel '{}' からメッセージを受信: {}", label, String::from_utf8_lossy(&message_bytes));` — notify/push/RPC 全文
- `src/connection.rs:2567` `rtc_log_info!("[WebSocket] Sent text message: {}", text);` — 初回 connect メッセージ (metadata / signaling_notify_metadata が入る)

## 設計方針

以下のいずれかを選択:

- **案 A**: 上記の本文ログをすべて **削除** する (ユーザーは `on_signaling_message` / `on_message` で自前ログ)。
- **案 B**: `rtc_log_info!` → `rtc_log_debug!` (もしくは `trace`) に降格し、message type と長さだけを info で残す。
- **案 C**: JSON パース後に `metadata` / `signaling_notify_metadata` / `credential` / `params` を `<redacted>` に置換した文字列を組み立ててからログする。

**推奨は案 A**。ユーザーが必要ならハンドラで取れるため、SDK 側の info ダンプは削除。案 B は運用者が info で気軽に監視できる利点があるが、機密漏洩リスクとのトレードオフ。案 C は実装コストが高い。

`Ping` / `Pong` / `StateChanged` などの低頻度制御メッセージのログは、issue 0071 とは別に整理する (削除候補 IMP-71 相当)。

## 完了条件

- シグナリング / DataChannel 本文が info レベルで平文出力されない。
- ユーザーが必要ならハンドラで取れる旨を SKILL.md / rustdoc に明記する。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
