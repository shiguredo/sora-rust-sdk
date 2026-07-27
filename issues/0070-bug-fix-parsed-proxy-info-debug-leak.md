# `ParsedProxyInfo` の手書き `Debug` 実装を追加し、プロキシ password の平文露出を防ぐ

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-parsed-proxy-info-debug-leak
- Polished: 2026-07-27
- Updated: 2026-07-24

## 目的

`ParsedProxyInfo`（公開型）が `#[derive(Debug, Clone)]` のまま `password: Option<String>` を保持しており、`println!("{:?}", parsed)` / `tracing::debug!(?parsed)` で HTTP プロキシ認証パスワードが平文露出する。既にクローズ済みの issue 0037 で `ProxyInfo` には手書き Debug が実装済みだが、`ParsedProxyInfo` は実装漏れのまま残っている。

## 優先度根拠

High（セキュリティ致命）。`ProxyInfo` は手書き Debug 実装済みだが、`ParsedProxyInfo` は derive のまま残っている（公開型のためユーザー側のログ経路で機密が漏洩する）。issue 0037 が対象とした 3 型のうち `ParsedProxyInfo` だけ未対応であり、「秘密情報は Debug に出さない」というプロジェクトポリシーと実装が矛盾している。

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

`grep "impl.*Debug.*ParsedProxyInfo" src/` の結果は 0 件で、手書き Debug は存在しない。`ParsedProxyInfo` は `lib.rs` から `pub` で再エクスポートされている。

一方、`src/types.rs:83-93` の `ProxyInfo` は手書き `impl Debug` で `username` / `password` を `Some("<redacted>")` にマスク済みであり、このパターンを `ParsedProxyInfo` にも適用する。

## 設計方針

1. `#[derive(Debug, Clone)]` から `Debug` を外し、`#[derive(Clone)]` にする。
2. 手書き `impl std::fmt::Debug for ParsedProxyInfo` を追加し、`ProxyInfo` と同様のマスク方式を適用する:
   - `username`: `Some(_)` なら `Some("<redacted>")`、`None` なら `None`
   - `password`: 同上
   - `host` / `port` / `user_agent` はそのまま出力
3. `format!("{:?}", parsed)` に password 文字列が含まれないことを検証する単体テスト `parsed_proxy_info_debug_masks_credentials` を追加する。テストの配置先は `src/connection.rs` 内の `#[cfg(test)] mod tests`。
4. `ProxyInfo` の Debug 実装で採用されているマスク方式と同一のパターンとする。

## 完了条件

- `ParsedProxyInfo` から `Debug` が derive から外れ、手書き `impl Debug` が追加されている。
- `ParsedProxyInfo` の Debug 出力に `username` / `password` の実値が含まれない。
  - `username: Some("x")` のとき `Some("<redacted>")` が出力される。
  - `username: None` のとき `None` が出力される。
  - `password` についても同様。
- `host` / `port` / `user_agent` はそのまま出力される。
- 単体テスト `parsed_proxy_info_debug_masks_credentials` が追加されている。
- `cargo test --workspace --all-features` が通る。
- `cargo fmt --all -- --check` が通る。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
