# TURN-TLS のサーバー証明書検証に SubjectAltName / CN 検証を追加する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-turn-tls-san-verification-missing
- Polished: {YYYY-MM-DD}

## 目的

`TurnTlsCaCertVerifier::verify_chain` は `webpki::EndEntityCert::verify_for_usage` でチェーン信頼のみ検証し、`verify_is_valid_for_subject_name` を呼んでいないため SubjectAltName / CN 検証がスキップされている。RFC 6125 §6, §7 未準拠。TURN サーバーの host 名を verifier に渡し、SAN / CN 検証を追加する。

## 優先度根拠

High (セキュリティ致命)。`turn_tls_ca_cert` に公開 CA (例: Let's Encrypt) を指定すると、その CA が発行した **任意ホスト証明書** で TURN サーバーになりすまし可能。TURN 経由の映像・音声が第三者に復号される。ユーザーが「private CA 専用の API」と認識しないまま公開 CA を渡す運用ミスは十分に起こり得る。

## 現状

`src/connection.rs:612-650` に verifier 実装がある:

```rust
ee.verify_for_usage(
    webpki::ALL_VERIFICATION_ALGS,
    &self.trust_anchors,
    &intermediates,
    time,
    webpki::KeyUsage::server_auth(),
    None,   // CRL
    None,
).is_ok()
```

rustls-webpki の `verify_for_usage` は API doc で明示的に「subject name は検証しない」と告知しており、SAN / CN 検証は別 API `verify_is_valid_for_subject_name` が担当する。現状はチェーン信頼だけ通し、subject name はノーチェック。

さらに `TurnTlsCaCertVerifier` は TURN サーバーの host 名を受け取っておらず、SAN 検証を追加しようとしても実装できない構造になっている。

## 設計方針

1. `TurnTlsCaCertVerifier` に TURN サーバーの host 名を渡せるようにする (`IceServerConfig::urls` から host を抽出する経路を追加)。
2. `verify_chain` の中で `verify_for_usage` の直後に `ee.verify_is_valid_for_subject_name(&SubjectNameRef::try_from_ascii_str(host)?)` を呼ぶ。
3. CRL / OCSP は TURN では慣行上省略が許容されているため、そのまま省略で良い (rustdoc で明記)。
4. 短期対応として、README または SKILL.md に「`turn_tls_ca_cert` は TURN サーバー専用の private CA のみを指定すること。公開 CA を指定するとホスト名検証されないため、SDK の SAN 検証実装が完了するまでは避けること」を強く警告する。
5. shiguredo_webrtc 側で SAN 検証が別途行われているかを事前に調査し、行われているなら SDK 側の実装は不要 (verifier ドキュメント整備のみ)。

## 完了条件

- TURN サーバー証明書検証で SAN / CN 検証が実施され、host 名不一致で拒否される。
- 単体テストで「host 名不一致の証明書は拒否される」ことを検証している。
- rustdoc / README に TURN-TLS の検証範囲が明記されている。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
