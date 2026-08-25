# TURN-TLS insecure モードの実装

## 概要

TURN-TLS の SSL 検証スキップ (insecure モード) を実装する。

## 背景

webrtc-rs に `TlsCertPolicy::InsecureNoCheck` と `IceServer::set_tls_cert_policy()` API が追加されている。
sora-rust-sdk ではまだこの API を利用していない。

## webrtc-rs の API

- `TlsCertPolicy::InsecureNoCheck` (`src/api/peer_connection.rs:480`)
- `IceServer::set_tls_cert_policy()` (`src/api/peer_connection.rs:542`)

## 対応内容

- ICE サーバー設定時に `TlsCertPolicy::InsecureNoCheck` を設定できるようにする
- `SoraClientBuilder` に TURN-TLS insecure 設定を追加する
- `docs/SORA_CPP_SDK.md` の対応表を更新する

## 解決方法

- `SoraClientBuilder` に `turn_tls_insecure: bool` フィールドとセッターメソッドを追加した
- `apply_ice_servers()` で `turn_tls_insecure` が `true` の場合に `IceServer::set_tls_cert_policy(TlsCertPolicy::InsecureNoCheck)` を呼び出すようにした
- `shiguredo_webrtc` の import に `TlsCertPolicy` を追加した
- `docs/SORA_CPP_SDK.md` の TURN-TLS insecure モードを「未実装」から「o」に更新した
