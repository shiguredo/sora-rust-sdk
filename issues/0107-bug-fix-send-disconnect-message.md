# 切断時に disconnect メッセージを送信する

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
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
