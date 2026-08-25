# TURN-TLS CA 証明書指定の実装

## 概要

TURN-TLS の CA 証明書指定を実装する。

## 背景

webrtc-rs に `PeerConnectionDependencies::set_tls_cert_verifier()` と `SSLCertificateVerifier` API が追加されている。
sora-rust-sdk ではまだこの API を利用していない。

## webrtc-rs の API

- `PeerConnectionDependencies::set_tls_cert_verifier(SSLCertificateVerifier)` (`src/api/peer_connection.rs:1323`)
- `SSLCertificateVerifier` (`src/rtc_base/ssl_certificate.rs`)

## 対応内容

- `SSLCertificateVerifier` を使ってカスタム CA 証明書で TURN-TLS の検証を行えるようにする
- `SoraClientBuilder` に TURN-TLS CA 証明書設定を追加する
- `docs/SORA_CPP_SDK.md` の対応表を更新する

## 解決方法

- `SoraClientBuilder` に `turn_tls_ca_cert: Option<Vec<u8>>` フィールドと `turn_tls_ca_cert(der: Vec<u8>)` セッターを追加した
- `TurnTlsCaCertVerifier` 構造体を作成し、`SSLCertificateVerifierHandler` を実装した
- `webpki::anchor_from_trusted_cert` で CA 証明書からトラストアンカーを生成し、`webpki::EndEntityCert::verify_for_usage` でチェーン検証を行う
- `SoraClient::new()` で CA 証明書が設定されている場合に `PeerConnectionDependencies::set_tls_cert_verifier()` を呼び出すようにした
- `Error::TurnTlsCaCert` エラーバリアントを追加した
- `rustls-webpki` を依存関係に追加した
- `docs/SORA_CPP_SDK.md` の TURN-TLS CA 証明書指定を「未実装」から「o」に更新した
