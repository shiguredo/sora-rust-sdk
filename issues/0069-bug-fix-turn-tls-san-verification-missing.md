# TURN-TLS のサーバー証明書検証に SubjectAltName 検証を追加する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-turn-tls-san-verification-missing
- Polished: 2026-07-27

## 目的

`TurnTlsCaCertVerifier::verify_chain` は `webpki::EndEntityCert::verify_for_usage` でチェーン信頼のみ検証し、`verify_is_valid_for_subject_name` を呼んでいないため SubjectAltName (SAN) のホスト名検証がスキップされている。RFC 9525 §6 (旧 RFC 6125) に未準拠。TURN サーバーの host 名を verifier で受け取り、SAN 検証を追加する。

CN (Common Name) は RFC 9525 §6.3 で非推奨であり、webpki も CN 検証 API を提供していないため、CN 検証は対象外とする。

## 優先度根拠

High (セキュリティ致命)。`turn_tls_ca_cert` に公開 CA (例: Let's Encrypt) を指定すると、その CA が発行した任意ホスト証明書で TURN サーバーになりすまし可能。TURN 経由の映像・音声が第三者に復号される。ユーザーが「private CA 専用の API」と認識しないまま公開 CA を渡す運用ミスは十分に起こり得る。

## 現状

`src/connection.rs:612-650` に verifier 実装がある:

```rust
struct TurnTlsCaCertVerifier {
    trust_anchors: Vec<TrustAnchor<'static>>,
}

impl SSLCertificateVerifierHandler for TurnTlsCaCertVerifier {
    fn verify_chain(&mut self, chain: SSLCertChainRef<'_>) -> bool {
        // ... 証明書チェーンの抽出 ...
        ee.verify_for_usage(
            webpki::ALL_VERIFICATION_ALGS,
            &self.trust_anchors,
            &intermediates,
            time,
            webpki::KeyUsage::server_auth(),
            None,   // CRL
            None,
        ).is_ok()
        // ↑ verify_is_valid_for_subject_name が呼ばれていない
    }
}
```

rustls-webpki の `verify_for_usage` は API doc で明示的に「subject name は検証しない」と告知しており、SAN 検証は別 API `verify_is_valid_for_subject_name` が担当する。現状はチェーン信頼だけ通し、subject name はノーチェック。

さらに `TurnTlsCaCertVerifier` は TURN サーバーの host 名を受け取っておらず、SAN 検証を追加しようとしても実装できない構造になっている。

### 実装上の制約

- verifier 生成は `SoraConnection::new()` 内 (`src/connection.rs:731-742`) で行われ、その直後に `PeerConnection::create()` が呼ばれるため、verifier の所有権は PeerConnection に移り後から差し替えられない
- ICE サーバー情報 (`IceServerConfig`) はシグナリングの offer/re-offer 受信時 (`src/connection.rs:1494`) に初めて届くため、verifier 生成時点では TURN ホスト名は未知
- `verify_chain` のシグネチャは `fn verify_chain(&mut self, chain: SSLCertChainRef<'_>) -> bool` であり、接続先ホスト名は引数として渡されない
- re-offer (`src/connection.rs:1098`) で ICE サーバーが差し替わる可能性がある
- `turn_tls_insecure = true` 時は `set_tls_cert_policy(TlsCertPolicy::InsecureNoCheck)` (`src/connection.rs:1429-1430`) が設定され、verifier 自体がバイパスされる可能性がある（shiguredo_webrtc 側の挙動に依存するため、実装時に実際のバイパス有無を確認する）

## 設計方針

### 1. 共有状態によるホスト名の遅延注入

`TurnTlsCaCertVerifier` に `Arc<Mutex<Vec<String>>>` で許可ホスト名リストを持たせる。verifier 生成時点では空リストで作成し、同じ `Arc` のクローンを `SoraConnection` のフィールドとして保持する（`apply_pc_configuration()` が `impl SoraConnection` のメソッドであるため。`SoraConnectionBuilder` は `SoraConnection::new()` で消費されるため保持先にできない）。

`Mutex` を選択する理由: `verify_chain` は shiguredo_webrtc の同期コールバック（戻り値 `bool`）であり、`tokio::sync::mpsc` の `.await` が使えない。`std::sync::mpsc` を使う場合もコールバック内でブロッキング `recv()` が必要になり TLS 検証スレッドを不必要にブロックする。ロック範囲は `Vec<String>` の読み取りまたは差し替えのみで最小限であり、競合は re-offer 時と TLS ハンドシェイク時に限られる。これらの理由により shiguredo-rust スキルの「共有状態を Mutex で保護する設計を安易に選ばないこと」規約におけるチャネル方式の検討を経たうえで Mutex を選択する。

`handle_offer()` → `apply_pc_configuration()` (`src/connection.rs:1415-1444`) の先頭で、`IceServerConfig::urls` から `turns:` スキームの URL を抽出し、ホスト名を許可リストに設定する。re-offer 時は新しいリストを先に構築してからアトミックに差し替える（lock → 新 `Vec` で置き換え → unlock）。`apply_pc_configuration()` 先頭の `if servers.is_empty() { return Ok(()); }` の早期 return により、re-offer で ICE サーバーが省略された場合は許可リストを更新せず維持する（既存の ICE サーバー設定が維持される挙動と整合させる）。

`turn_tls_ca_cert` が `None` の場合は `Arc` が生成されないため、`apply_pc_configuration()` 内のホスト名抽出処理を `Option<Arc<Mutex<Vec<String>>>>` のガード付きで実装する（`if let Some(ref hosts) = self.allowed_hosts { ... }`）。

### 2. verify_chain 内での SAN 検証

`verify_chain` 内で `verify_for_usage` 通過後、許可リストをロックして全ホスト名を走査し、以下のパターンで SAN 検証を行う:

```rust
for host in hosts.lock().unwrap_or_else(|poison| poison.into_inner()).iter() {
    if let Ok(subject_name) = SubjectNameRef::try_from_ascii_str(host) {
        if ee.verify_is_valid_for_subject_name(&subject_name).is_ok() {
            return true;
        }
    }
}
false
```

`verify_chain` は shiguredo_webrtc の C++ 側から呼ばれる同期コールバックであるため、Rust のパニックが FFI 境界を越えると未定義動作になる。`unwrap()` ではなく `unwrap_or_else(|poison| poison.into_inner())` を使い、Mutex が poison された場合でも内部データを復旧して検証を継続する。poison が発生し得る典型的なシナリオは `apply_pc_configuration` がロック保持中にアサーション失敗等でパニックした場合だが、発生確率はきわめて低い。

`try_from_ascii_str` の失敗は、TURN URL から正しくホスト名が抽出されていれば通常発生しない。ただし false 返却によりフェイルセーフに動作する（実装上の防御として `expect()` ではなく false 返却を選択する）。

検証失敗時はログ（`rtc_log_warning!`）に失敗した証明書の subject 情報と期待ホスト名を出力する。

### 3. CRL / OCSP の扱い

CRL / OCSP は TURN では慣行上省略が許容されているため、引き続き省略する。rustdoc でその旨を明記する。

### 4. `turn_tls_insecure` との関係

`turn_tls_insecure = true` 時は `TlsCertPolicy::InsecureNoCheck` により verifier がバイパスされる可能性が高いが、shiguredo_webrtc 側の実際の挙動を実装時に確認し、両方が指定された場合の優先順位を rustdoc と README に明記する。

### 5. TURN URL からのホスト名抽出

`IceServerConfig::urls` に含まれる `turns:` スキームの URL からホスト名を抽出する。抽出手順は以下のとおり:

1. URL 文字列からスキーム `turns:` を除去する
2. ホスト名部分を切り出す（例: `turns:host.example.com:443?transport=tcp` → `host.example.com`）
3. IPv6 アドレスはブラケット表記 `[2001:db8::1]` に対応し、ブラケットを外して取り出す
4. ポート番号（`:443` 等）は除去する
5. クエリパラメータ（`?transport=...`）は除去する

`turn:` スキーム（非 TLS）の URL は対象外とする。複数の異なるホスト名が存在する場合はすべてのホスト名を許可リストに追加する（fallback 先の TURN サーバー証明書も正しく検証される）。同一ホスト名の重複除去は不要（検証結果に影響しないため）。

URL パースが複雑になる場合は、既存の依存に URL パース用のクレートを追加するか、簡易な文字列操作で実装するかを実装時に判断する。

## 完了条件

- TURN サーバー証明書検証で SAN (SubjectAltName) のホスト名検証が実施され、host 名不一致で拒否される。
- re-offer で ICE サーバーが差し替わった場合も、新しいホスト名に対して正しく検証が行われる。
- `turn_tls_insecure = true` と `turn_tls_ca_cert` 併用時の挙動が rustdoc と README に明記されている。
- rustdoc / README に TURN-TLS の検証範囲（チェーン信頼 + SAN、CRL/OCSP は省略）が明記されている。
- `cargo test --workspace` と `cargo clippy --workspace --all-targets --all-features -- -D warnings` が通る。
