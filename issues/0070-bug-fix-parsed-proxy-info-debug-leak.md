# `ParsedProxyInfo` の手書き `Debug` 実装を追加し、プロキシ password の平文露出を防ぐ

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-parsed-proxy-info-debug-leak
- Polished: {YYYY-MM-DD}
- Updated: 2026-07-24

## 目的

`ParsedProxyInfo` (公開型) が `#[derive(Debug, Clone)]` のまま `password: Option<String>` を保持しており、`println!("{:?}", parsed)` / `tracing::debug!(?parsed)` で HTTP プロキシ認証パスワードが平文露出する。既にクローズ済みの issue 0037 「Debug から機密を redact する」の実装対象から漏れているため、追加で手書き `Debug` を実装する。

## 優先度根拠

High (セキュリティ致命)。issue 0037 は Completed 判定されているのに実装が抜けており、「秘密情報は Debug に出さない」というプロジェクトポリシーと実装が矛盾している。`ProxyInfo` は手書き Debug 実装済みだが、`ParsedProxyInfo` は derive のまま残っている (公開型のためユーザー側のログ経路で機密が漏洩する)。なお `TlsConfig` は Debug 自体を実装していない (`#[derive(Clone, Default)]` のみ) ため実際の漏洩は起きないが、issue 0037 が要求した「`impl std::fmt::Debug for TlsConfig` を追加してマスクする」自体は同様に未達である。

## 現状

`src/connection.rs:1976-1983` に定義がある:

```rust
#[derive(Debug, Clone)]
pub struct ParsedProxyInfo {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) user_agent: String,
}
```

`grep "impl.*Debug.*ParsedProxyInfo" src/` の結果は 0 件で、手書き Debug は存在しない。ParsedProxyInfo は `lib.rs` から `pub` で再エクスポートされている。

一方、issue 0037 の解決方法として実装された `types.rs::ProxyInfo` (types.rs:83 付近) は手書き `impl Debug` で `<redacted>` にマスクされている。`TlsConfig` (src/connection.rs:55) は `#[derive(Clone, Default)]` のままで Debug 自体を実装していない (`grep 'impl.*Debug.*TlsConfig' src/` は 0 件)。漏洩は起きないが、issue 0037 の完了条件 (`impl std::fmt::Debug for TlsConfig` を追加してマスクする) は未達のままである。本 issue のスコープ (`ParsedProxyInfo`) と重ならないため `TlsConfig` の Debug 追加は本 issue では扱わないが、issue 0037 の追加漏れとして別途対応を検討する必要がある。

## 設計方針

1. `#[derive(Debug, Clone)]` から `Debug` を外し、`Debug, Clone)` を `Clone)` にする。
2. 手書き `impl std::fmt::Debug for ParsedProxyInfo` を追加し、以下のようにマスクする:
   - `username`: `Some(_)` なら `"<redacted>"`、`None` なら `None`
   - `password`: 同上
   - `host` / `port` / `user_agent` はそのまま出力
3. 単体テスト `parsed_proxy_info_debug_masks_credentials` を追加し、`format!("{:?}", parsed)` に実際の password 文字列が含まれないことを検証する。
4. issue 0037 の実装 (types.rs::ProxyInfo) と揃った出力形式にする。

## 完了条件

- `ParsedProxyInfo` の Debug 出力に `username` / `password` の実値が含まれない。
- 単体テストで redact が検証されている。
- issue 0037 の Completed 条件を実質的に満たす。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
