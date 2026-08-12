# 切断時に disconnect メッセージを送信する

- Priority: High
- Created: 2026-08-10
- Completed: 2026-08-13
- Model: deepseek-v4-flash
- Branch: feature/fix-send-disconnect-message
- Polished: {YYYY-MM-DD}

## 目的

Sora クライアント要求仕様で定められた切断時の `"type": "disconnect"` メッセージを送信し、サーバー側のセッション残存と切断検知の遅延を防ぐ。

## 現状

`SoraConnectionHandle::disconnect()` に対応する `SoraConnectionCommand::Disconnect` の処理 (`src/connection.rs`) は、オープン中の DataChannel への close 通知と ack 送信後にメインループを break するだけで、`"type": "disconnect", "reason": "NO-ERROR"` を一切送信しない。終了時の `ws.close()` のみで、シグナリングメッセージとしての disconnect は WebSocket 経由・DataChannel 経由のどちらでも送られない。

特に `ignore_disconnect_websocket=true` で WebSocket が既に閉じている構成では、サーバーに切断を通知する手段は DataChannel 経由の disconnect メッセージしかないが、送信されないためサーバーは DataChannel のクローズに気付くまで最大 10 秒ほど切断を認識しない。参照実装 (sora-cpp-sdk) は disconnect メッセージを送信してからクローズする。

## 設計方針

- `Disconnect` コマンド処理で、WebSocket が接続中なら WebSocket 経由、DataChannel シグナリング切替後なら `signaling` ラベルの DataChannel 経由で `"type": "disconnect", "reason": "NO-ERROR"` を送信してから break する
- 送信失敗は切断処理を中断させず、ログ (英語) に残して break する
- 既存の `OutgoingMessage` に `Disconnect` バリアントを追加するか、既存の組み立てパターンに合わせて送信する
- サーバー起因の切断 (close 受信等) では送信不要

## 完了条件

- `SoraConnectionHandle::disconnect()` を呼んだときに disconnect メッセージが送信される
- WebSocket 経由と DataChannel シグナリングの両構成で正しい経路から送信される
- 送信失敗時に接続終了処理が妨げられない
- サーバー起因の切断では送信されない
- モックやスタブを使わずに検証されている (DataChannel 経路は e2e テストで確認する)
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `src/signaling_types.rs`
- `CHANGES.md`

## 解決方法

- `src/signaling_types.rs` の `OutgoingMessage` に `Disconnect` バリアントを追加し、`{"type":"disconnect","reason":"NO-ERROR"}` にシリアライズする `new_disconnect()` を追加した
- `src/connection.rs` の `SoraConnectionCommand::Disconnect` 処理で、DataChannel シグナリング有効時 (`use_datachannel_signaling`) は `signaling` ラベルの DataChannel 経由、それ以外で WebSocket 接続中は WebSocket 経由で disconnect メッセージを送信するようにした
- 送信失敗は接続終了処理を妨げず、英語のログを残して break する
- DataChannel シグナリング時は run ループ終了後のクローズ待機 (`disconnect_wait_timeout`) で DataChannel のクローズを待ち、close コールバックを通知するようにした
- `e2e-tests/tests/send_disconnect_message.rs` を追加し、DataChannel 経由と WebSocket 経由の両構成で disconnect メッセージの送信を検証した
- `CHANGES.md` への記載は依頼により行わない
