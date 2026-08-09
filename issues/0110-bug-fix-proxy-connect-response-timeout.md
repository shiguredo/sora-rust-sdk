# HTTP プロキシの CONNECT 応答待ちにタイムアウトを設ける

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-proxy-connect-response-timeout
- Polished: {YYYY-MM-DD}

## 目的

CONNECT 応答を返さない HTTP プロキシで `SoraConnection::run` が無限にハングしないようにする。

## 現状

`connect_http_proxy_tunnel` (`src/connection.rs`) の CONNECT 応答待ちループは `stream.read` にタイムアウトがなく、`websocket_connection_timeout` の期限を一切参照しない。TCP 接続は成立するが CONNECT 応答を返さないプロキシに接続した場合、応答待ちで永久にブロックする。

さらに `connect_signaling_urls` は全 URL の失敗時に `join_set.join_next()` で全タスクの完了を待つため、1 つのタスクが応答待ちでハングすると複数 URL 構成でも run() 全体がハングする。`SoraConnectionBuilder::websocket_connection_timeout` のドキュメント (「WebSocket 接続が確立するまでの待機時間の上限」) に反する。

## 設計方針

- `connect_http_proxy_tunnel` に `deadline` を渡し、CONNECT 応答待ちの read を `tokio::time::timeout_at` で囲む
- タイムアウト時は既存のエラー種別に倣った具体的なエラー (`Error` に新規バリアントまたは既存のタイムアウト系バリアント) を返す
- `connect_signaling_urls` の全タスク完了待ちが、残りタスクのハングで無限に続かないことを確認する (タイムアウト導入により各タスクが必ず終了する)

## 完了条件

- CONNECT 応答を返さないプロキシに対し、`websocket_connection_timeout` 以内にエラーが返る
- タイムアウト時に具体的なエラー内容がエラーメッセージに含まれる
- 正常なプロキシ経由接続の挙動が変わらない
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `src/error.rs`
- `CHANGES.md`
