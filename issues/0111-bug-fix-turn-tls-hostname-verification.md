# TURN-TLS の CA 証明書検証にサーバー名検証を追加する

- Priority: High
- Created: 2026-08-10
- Completed: 2026-08-13
- Model: deepseek-v4-flash
- Branch: feature/fix-turn-tls-hostname-verification
- Polished: {YYYY-MM-DD}

## 目的

`turn_tls_ca_cert` で独自 CA を設定した TURN-TLS 接続のサーバー名 (ホスト名) 検証を追加し、同一 CA が発行する別名証明書による中間者攻撃を防ぐ。

## 現状

`TurnTlsCaCertVerifier::verify_chain` (`src/connection.rs`) は `webpki::verify_for_usage` で証明書チェーンの検証・有効期限・サーバー認証 EKU のみを行い、サーバー名の検証を行わない。`webpki::verify_for_usage` にはサーバー名パラメータが存在せず、サーバー名検証は呼び出し側の責務だが本実装では呼び出していない。

`SSLCertificateVerifierHandler` のインターフェースは `fn verify_chain(&mut self, chain: SSLCertChainRef<'_>) -> bool` のみでサーバー名を受け取らないため、現状のインターフェースでは SDK 側でホスト名検証を追加できない構造的な制約がある。`turn_tls_insecure` は明示的な仕様だが、`turn_tls_ca_cert` は「CA で検証する」と謳いながら身元 (identity) 検証が欠落している。

## 設計方針

- 検証対象のホスト名を `TurnTlsCaCertVerifier` へ渡せるようにする。shiguredo-webrtc 側のインターフェース拡張が必要な場合は、その拡張を前提に設計を確定する (インターフェース変更ができない場合は、TURN URL のホスト名を SDK 側で検証する代替手段を検討する)
- 検証は TURN URL のホスト名 (IP アドレス指定時は IP SAN) に対して行う
- 検証失敗時は接続を拒否する
- ドキュメント (`turn_tls_ca_cert` の doc) に検証内容を明記する

## 完了条件

- `turn_tls_ca_cert` 設定時にサーバー名検証が行われる
- ホスト名不一致の証明書は拒否される
- IP アドレス直指定時の検証が正しく行われる
- `turn_tls_insecure` の挙動は変わらない
- 検証ロジックの単体テストがある (モックやスタブは使わない)
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `src/error.rs` (必要に応じて)
- 依存クレート (shiguredo-webrtc) のインターフェース (必要に応じて)
- `CHANGES.md`

## 解決方法

**修正不要として closed にする。** 本 issue は `issues/closed/0069-bug-fix-turn-tls-san-verification-missing.md` と同一内容の重複であり、0069 の調査で以下の結論が得られているため対応しない。

libwebrtc が TURN-TLS 接続時に OpenSSL の `X509_check_host()` で SAN 検証を既に行っている。カスタム `SSLCertificateVerifier` を設定しても、`BasicPacketSocketFactory::CreateClientTcpSocket()` (`basic_packet_socket_factory.cc:214`) が `ssl_adapter->StartSSL(remote_address.hostname())` でホスト名を SSL アダプタに渡し、`OpenSSLAdapter::ContinueSSL()` (`openssl_adapter.cc:396`) が `SSLPostConnectionCheck()` を呼んで `openssl::VerifyPeerCertMatchesHost(ssl, host) && cert_verified` の両方を要求する (`openssl_adapter.cc:773-774`)。`VerifyPeerCertMatchesHost()` (`openssl_utility.cc:215`) が `X509_check_host()` で SAN を検証し、ホスト名検証はバイパスされない。

`src/connection.rs` の `TurnTlsCaCertVerifier::verify_chain` は当時の調査から変更がなく、上記の結論は現状のコードにも依然として適用される。
