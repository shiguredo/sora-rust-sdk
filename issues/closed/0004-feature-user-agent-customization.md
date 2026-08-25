# User-Agent カスタマイズ

## 概要

WebSocket 接続時の User-Agent ヘッダをカスタマイズできるようにする。

## 背景

C++ SDK では User-Agent をカスタマイズする機能がある。
Rust SDK では未実装。

## 対応内容

- `SoraClientBuilder` に User-Agent カスタマイズ設定を追加する
- WebSocket 接続時のハンドシェイクに User-Agent ヘッダを設定する
- `docs/SORA_CPP_SDK.md` の対応表を更新する

## 解決方法

- `SoraClientBuilder` に `user_agent: Option<String>` フィールドと `user_agent()` setter メソッドを追加した
- 未設定の場合はデフォルト値 `"Sora Rust SDK {version}"` (`get_sora_client_name()`) を使用する
- WebSocket 接続時 (初回接続・リダイレクト時の両方) で `ClientConnectionOptions` に `User-Agent` ヘッダを設定するようにした
- `docs/SORA_CPP_SDK.md` の対応表を更新した
