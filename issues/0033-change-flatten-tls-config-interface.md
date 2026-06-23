# `TlsConfig` のフィールド直接アクセスと builder メソッドの二重インターフェースを一本化する

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/flatten-tls-config-interface
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`TlsConfig` の二重インターフェース」分。

## 目的

`src/connection.rs:55-64` の `TlsConfig` は全フィールドが `pub` で公開されており、利用者がフィールドに直接代入できる。同時に `SoraConnectionBuilder` 側 (`src/connection.rs:398-415`) には `insecure()` / `client_cert()` / `ca_cert()` のメソッドが用意されていて、内部でフィールドを書き換えている。

この二重インターフェースには以下の問題がある:

- フィールドを追加するたびに builder メソッドとフィールドの両方を同期する必要があり、漏れたときに 1 経路だけ整備された API が残る
- 利用者がどちらを使えば良いか判断しづらい (rustdoc 上はどちらも有効)
- `#[non_exhaustive]` 化 (親 issue M3) するときに `pub` フィールドのまま `#[non_exhaustive]` を付けると `TlsConfig { insecure: ..., .. }` パターンが書けなくなり、外部利用者の構造体リテラルが破綻する

本 issue では、`TlsConfig` のインターフェースを一本化する。

## 優先度根拠

Medium。

- 正式リリース 2026.1.0 で公開 API として固定すると、後から「フィールドを `pub(crate)` 化する」も「builder メソッドを廃止する」も SemVer 破壊変更になり取り戻せない
- 親 issue M3 (`#[non_exhaustive]` 一斉付与) の前提として、`TlsConfig` の利用パターン (構造体リテラル vs builder) を確定させる必要がある
- 修正は機械的で、テスト追加もメソッド側に揃えれば十分

## 現状

### `TlsConfig` (`src/connection.rs:55-64`)

```rust
/// WebSocket (シグナリング接続) の TLS 設定。
///
/// TURN-TLS の TLS 設定は `SoraConnectionBuilder::turn_tls_insecure()` / `turn_tls_ca_cert()` で行う。
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    /// サーバー証明書の検証をスキップする。
    pub insecure: bool,
    /// クライアント証明書 (PEM 形式)。
    pub client_cert: Option<String>,
    /// クライアント秘密鍵 (PEM 形式)。
    pub client_key: Option<String>,
    /// CA 証明書 (PEM 形式)。
    pub ca_cert: Option<String>,
}
```

### `SoraConnectionBuilder` 側のメソッド (`src/connection.rs:398-415`)

```rust
/// サーバー証明書の検証をスキップする。
pub fn insecure(mut self, value: bool) -> Self {
    self.tls_config.insecure = value;
    self
}

/// クライアント証明書と秘密鍵を設定する (PEM 形式)。
pub fn client_cert(mut self, cert: String, key: String) -> Self {
    self.tls_config.client_cert = Some(cert);
    self.tls_config.client_key = Some(key);
    self
}

/// CA 証明書を設定する (PEM 形式)。
pub fn ca_cert(mut self, cert: String) -> Self {
    self.tls_config.ca_cert = Some(cert);
    self
}
```

- `SoraConnectionBuilder` から `TlsConfig` を組み立てる経路と、利用者が直接 `TlsConfig` を構築する経路の両方が公開されている
- `TlsConfig::default()` で空構造体を作って `TlsConfig { insecure: true, ..Default::default() }` のように構造体リテラルで組み立てる用途は現状あるか不明 (`grep` で確認する)
- `tls_config(TlsConfig)` のような setter は現状無い (要確認)

## 設計方針

### 選択肢 A (推奨): フィールドを `pub(crate)` 化し、builder メソッド一本化

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    pub(crate) insecure: bool,
    pub(crate) client_cert: Option<String>,
    pub(crate) client_key: Option<String>,
    pub(crate) ca_cert: Option<String>,
}

impl TlsConfig {
    pub fn new() -> Self { Self::default() }
    pub fn insecure(mut self, value: bool) -> Self { self.insecure = value; self }
    pub fn client_cert(mut self, cert: String, key: String) -> Self { /* ... */ self }
    pub fn ca_cert(mut self, cert: String) -> Self { /* ... */ self }
}
```

- `SoraConnectionBuilder` 側の `insecure()` / `client_cert()` / `ca_cert()` は維持してよい (内部で `self.tls_config = self.tls_config.insecure(value)` のように chain する)
- もしくは `tls_config(TlsConfig)` を 1 本化して、`TlsConfig` 自体を builder pattern で組み立てる方式に変える

### 選択肢 B: `pub` フィールド維持 + builder メソッド削除

```rust
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    pub insecure: bool,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub ca_cert: Option<String>,
}
```

`SoraConnectionBuilder::insecure()` / `client_cert()` / `ca_cert()` を削除し、`tls_config(TlsConfig)` を新規に追加して利用者は `TlsConfig` を直接組み立てる。

- 利用者にとっては「`tls_config()` で `TlsConfig` を渡す」だけのシンプル API
- ただし `#[non_exhaustive]` が `pub` フィールドと共存できないため、`TlsConfig { insecure: ..., .. }` 構造体リテラルパターンを諦めるか、`#[non_exhaustive]` を付けないか (親 issue M3 と衝突) の判断が必要

### 推奨

選択肢 A を採用する。理由:

- 親 issue M3 で `#[non_exhaustive]` 一斉付与が決まっている
- builder pattern は `SoraConnectionBuilder` 全体ですでに採用されており、API の一貫性が保てる
- フィールド `pub(crate)` 化はクレート内部からはフィールドアクセスできるため、`pub` フィールドで書かれている内部コードはほぼ無修正で済む

## 完了条件

- `TlsConfig` のフィールドが `pub(crate)` 化されている (or `pub` で `#[non_exhaustive]` を諦める設計判断が記録されている)
- `TlsConfig` 自体が builder pattern で組み立てられる (`new()` + setter メソッド)
- `SoraConnectionBuilder` 側の `insecure()` / `client_cert()` / `ca_cert()` は維持する場合、内部で `TlsConfig` の builder メソッドを呼ぶよう統一する
- 利用者ドキュメント (rustdoc) に「`TlsConfig` を組み立てるには `TlsConfig::new().insecure(true).ca_cert(...)` のように builder メソッドを使う」ことを明記する
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- `cargo doc -D warnings` (親 issue S4 の `cargo doc -D warnings` ジョブ追加分) で warning が出ない

## 解決方法

1. `TlsConfig` の利用箇所を grep で確認する (`grep -nE 'TlsConfig\s*\{' src/ examples/`、`grep -nE '\.insecure\s*=' src/ examples/` など)
2. 構造体リテラル / 直接代入が使われている場合は、builder メソッドへの移行を実装する (本リポジトリ内のサンプル / テストを優先)
3. フィールドを `pub` → `pub(crate)` に変更する
4. `TlsConfig` に `new()` および各フィールドの setter (`insecure(self, bool) -> Self` 等) を追加する
5. `SoraConnectionBuilder::insecure()` / `client_cert()` / `ca_cert()` は維持し、内部で `TlsConfig` の builder メソッドを呼ぶよう書き換える (利用者から見た互換性を保つ)
6. rustdoc に builder pattern の例を追加する
7. テストでは `TlsConfig::new().insecure(true).client_cert(cert, key).ca_cert(ca)` のチェーンが期待した内部状態を作ることを単体テストで確認する (フィールドが `pub(crate)` なのでクレート内テストから状態確認可能)
