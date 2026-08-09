# switched 後の WebSocket I/O エラーで DataChannel シグナリングを継続する

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-ignore-websocket-io-errors-after-switched
- Polished: {YYYY-MM-DD}

## 目的

`ignore_disconnect_websocket=true` で switched 受信済みの状態では、DataChannel シグナリングが健全な限り WebSocket の I/O エラーで接続全体を終了させない。

## 現状

`SoraConnection::run` のメインループ (`src/connection.rs`) では、switched 後の WebSocket 終了を吸収する処理が経路ごとに非対称になっている。

- `flush_ws_output` のエラーは `switched_ignore_disconnect_websocket && !websocket_closed` が成立するなら吸収される
- 一方、`stream.read` の `UnexpectedEof` 以外のエラー (ECONNRESET 等) は `return Err` で run() が終了する
- `ws.feed_recv_buf` のエラー (壊れたフレーム・不正 UTF-8 等の WebSocket プロトコルエラー) も `?` で run() が終了する

RST 切断やプロトコルエラーは `ignore_disconnect_websocket=true` の主要シナリオ (モバイル回線の切断等) で現実的に発生し得る。

## 設計方針

- switched 受信済みかつ `switched_ignore_disconnect_websocket=true` の状態では、`stream.read` のエラーと `feed_recv_buf` のエラーを吸収し、`websocket_closed = true` として DataChannel シグナリングを継続する
- 吸収の条件は `flush_ws_output` 側の既存条件と揃え、切替成立前のエラーは従来どおりエラーとして伝播する
- 吸収時は英語で警告ログを出力する

## 完了条件

- switched 後に ECONNRESET 等の read エラーが発生しても run() が終了しない
- switched 後に WebSocket プロトコルエラーが発生しても run() が終了しない
- 切替成立前の同種エラーは従来どおりエラーになる
- 通常の WebSocket のみ構成の挙動が変わらない
- production log は英語、コメントとテストの assertion message は日本語にする
- `cargo test --workspace` が成功する

## 変更対象

- `src/connection.rs`
- `CHANGES.md`
