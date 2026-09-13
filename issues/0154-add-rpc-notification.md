# 受信用 RPC 通知を追加する

- Created: 2026-09-04
- Completed: {YYYY-MM-DD}
- Branch: feature/add-rpc-notification
- Polished: {YYYY-MM-DD}

## 目的

サーバー起点の RPC 到達 (要求や通知) を利用者に中継できるようにする。現状は応答待ちの突き合わせに内部消費されるだけで、サーバー起点の到達を利用者へ渡す手段がない。

## 現状

- `SoraConnectionEventHandler` (`src/connection_event_handler.rs`) に RPC 受信用の受け口がない。既存の 12 個のコールバックは `on_signaling_message`、`on_notify`、`on_push`、`on_track`、`on_remove_track`、`on_switched`、`on_websocket_close`、`on_message`、`on_data_channel`、`on_data_channel_open`、`on_data_channel_message`、`on_data_channel_close` である
- `SoraConnection::handle_data_channel_message` (`src/connection.rs`) の `rpc` ラベル分岐は `RpcResponse::parse` (`src/rpc.rs`) で応答として解釈し、`pending_rpc_responses` と突き合わせできたものだけを対応する `send_rpc_request` の呼び出し元へ返す。未知の id、id なし、構文誤り、相関できない protocol violation はメッセージ単位に破棄して warning ログに落とす
- `docs/SORA_CPP_SDK.md` の RPC 表は、C++ SDK がコールバック (`OnRpc`) で送受信両方を扱うのに対し、Rust SDK は async/await (`send_rpc_request`) のみと明記している

## 設計方針

- `SoraConnectionEventHandler` に受信用の RPC 通知 (例: `on_rpc`) を追加する。引数は `on_notify` や `on_push` と同じく JSON 文字列 (`&str`) とする
- 中継対象は応答待ちに突き合わせできない到達 (要求や通知) とする。`pending_rpc_responses` と相関できた応答は従来どおり `send_rpc_request` の呼び出し元へ返し、`on_rpc` へは渡さない
- 受信本文をログやエラーに含めない扱いは `rpc` ラベル分岐の現行方針を維持する

## 完了条件

- サーバー起点の RPC 到達 (要求や通知) が `on_rpc` で受信できる
- 応答と突き合わせできた到達は従来どおり `send_rpc_request` の呼び出し元へ返る
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- コメントは日本語、ログメッセージは英語、テストの assertion message は日本語で書く
- モックやスタブは使用しない
- `CHANGES.md` の develop セクションに `[ADD]` エントリを追記する

## 変更対象

- `src/connection_event_handler.rs` (`SoraConnectionEventHandler` の拡張)
- `src/connection.rs` (`rpc` ラベル分岐の中継処理の追加)
- `docs/SORA_CPP_SDK.md` (RPC 表の更新)
- `CHANGES.md`
