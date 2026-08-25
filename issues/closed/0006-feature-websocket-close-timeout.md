# WebSocket 閉じるタイムアウト

## 概要

WebSocket を閉じる際のタイムアウトを設定できるようにする。

## 背景

C++ SDK では `websocket_close_timeout` (デフォルト 3 秒) で WebSocket クローズのタイムアウトを設定できる。
Rust SDK では WebSocket クローズ処理にタイムアウトがない。

## 調査結果

### C++ SDK

- フィールド: `websocket_close_timeout` (`int`, デフォルト 3 秒)
- `closing_timeout_timer_` で管理
- 通常切断時、リダイレクト時、他の接続完了時に適用
- タイムアウト時は `ws_->Cancel()` を呼び、クローズコード 4999 を返す

### Rust SDK 現状

- `ws.close()` 後に `flush_ws_output()` で出力を送信するが、クローズ完了のタイムアウトはない
- メインループの `ws.state() == ConnectionState::Closed` で検出して break する
- redirect 時 (client.rs:994)、DataChannel 切替時 (client.rs:1044)、shutdown 時 (client.rs:1058) でクローズ

## 方針

- `SoraClientBuilder` に `websocket_close_timeout: Duration` を追加する (デフォルト 3 秒、C++ SDK と合わせる)
- WebSocket クローズ後、メインループ内で `ConnectionState::Closed` になるまでの待機にタイムアウトを適用する
- タイムアウト時はストリームを強制切断する
- `docs/SORA_CPP_SDK.md` の対応表を更新する

## 解決方法

- `SoraClientBuilder` に `websocket_close_timeout: Duration` を追加した (デフォルト 3 秒)
- メインループ脱出後の WebSocket クローズ処理に `tokio::time::timeout` を適用した
- タイムアウト時はログを出力して強制切断する
