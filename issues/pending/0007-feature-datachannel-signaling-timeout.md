# DataChannel シグナリングタイムアウト

## 概要

DataChannel シグナリングへの切り替え時のタイムアウトを設定できるようにする。

## 背景

C++ SDK では `data_channel_signaling_timeout` (デフォルト 180 秒) が定義されている。
Rust SDK では未実装。

## 調査結果

### C++ SDK

- フィールド: `data_channel_signaling_timeout` (`int`, デフォルト 180 秒)
- フィールドは定義されているが、現在の実装では使用されていない

### Rust SDK 現状

- Switched メッセージ受信後、全 DataChannel がオープンするまで待機 (client.rs:1031-1045)
- DataChannel が全てオープンしてから `ws_disconnect_delay` (10 秒ハードコード) 後に WebSocket をクローズ
- DataChannel オープンまでの待機にタイムアウトがない

## 方針

- C++ SDK でも未使用のため、pending にする
