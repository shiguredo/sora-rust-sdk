# WebSocket 接続タイムアウト

## 概要

WebSocket 接続時のタイムアウトを設定できるようにする。

## 背景

C++ SDK では `websocket_connection_timeout` (デフォルト 30 秒) で接続タイムアウトを設定できる。
Rust SDK では TCP 接続に 5 秒のハードコードされたタイムアウトがあるが、TLS ハンドシェイクにはタイムアウトがない。

## 調査結果

### C++ SDK

- フィールド: `websocket_connection_timeout` (`int`, デフォルト 30 秒)
- `connection_timeout_timer_` で管理
- DoConnect() 時とリダイレクト時に適用

### Rust SDK 現状

- `connect_websocket()` の TCP 接続: `Duration::from_secs(5)` でハードコード (client.rs:1771)
- TLS ハンドシェイク: タイムアウトなし
- `connect_signaling_urls()` 全体: タイムアウトなし

## 方針

- `SoraClientBuilder` に `websocket_connection_timeout: Duration` を追加する (デフォルト 30 秒、C++ SDK と合わせる)
- `connect_websocket()` の TCP 接続タイムアウトを置き換える
- TLS ハンドシェイクにもタイムアウトを適用する (TCP + TLS 合計でタイムアウト)
- `connect_signaling_urls()` に渡して各 URL の接続に適用する
- `docs/SORA_CPP_SDK.md` の対応表を更新する

## 解決方法

- `SoraClientBuilder` に `websocket_connection_timeout: Duration` を追加した (デフォルト 30 秒)
- `connect_websocket()` に `timeout` 引数を追加し、`tokio::time::timeout_at` で deadline を共有して TCP 接続と TLS ハンドシェイクの合計時間にタイムアウトを適用した
- `connect_signaling_urls()` にも `timeout` 引数を渡すようにした
- リダイレクト時の再接続にもタイムアウトを適用した
- `Error::TlsConnectTimeout` エラーバリアントを追加した
