# `TlsConfig` / `ParsedProxyInfo` / `ProxyInfo` の `Debug` 実装を手書きにして秘密情報をマスクする

- Priority: High
- Created: 2026-06-23
- Completed: 2026-06-25
- Model: Opus 4.7
- Branch: feature/change-redact-secrets-in-debug-impls
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`TlsConfig` / `ParsedProxyInfo` / `ProxyInfo` の `Debug` 機密露出」分。

本 issue は #0035 (`ParsedProxyInfo` のフィールド可視性統一) の後に着手する前提である。#0035 実施後のフィールド可視性を前提として `Debug` 手書き実装を記述している。

## 目的

以下の 3 つの公開構造体で `#[derive(Debug)]` をそのまま使っているため、`println!("{:?}", config)` / `tracing::debug!(?config)` / panic backtrace 等で秘密情報が全文露出する:

- `TlsConfig` (`src/connection.rs:54`): `client_key` (PEM 形式の秘密鍵本文) が露出
- `ParsedProxyInfo` (`src/connection.rs:1933`): `username` / `password` (HTTP プロキシ認証情報) が露出
- `ProxyInfo` (`src/types.rs:54`): `url` (userinfo として `user:pass@` を含む可能性あり) / `username` / `password` が露出

各構造体の `Debug` を `#[derive(Debug)]` から手書き実装に置き換え、秘密情報を `<redacted>` でマスクする。

## 優先度根拠

High。

- PEM 秘密鍵本文や HTTP プロキシ password が debug print / structured logging で本番ログに残ると、ログ集約システムからの情報漏洩につながる
- `#[derive(Debug)]` を使う限り、利用者がコードレビューで気付かないと容易に漏れる
- 一度ログに乗ったものは事後に回収不可能
- 修正は `impl Debug` を 3 件手書きするだけで局所的
- 正式リリース 2026.1.0 の段階で「秘密情報を漏らさない」状態にしておくのが原則

## 現状

### `TlsConfig` (`src/connection.rs:54-64`)

```rust
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    pub insecure: bool,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub ca_cert: Option<String>,
}
```

- `client_key` は PEM 秘密鍵本文が全文露出する（致命的）
- `client_cert` / `ca_cert` は公開証明書であり秘密情報ではないが、PEM 全文が debug ログを圧迫するため `<present>` 表記に留める
- `SoraConnectionBuilder` (同ファイル 68 行目) は `tls_config: TlsConfig` を保持するが `Debug` を derive しておらず、本修正の波及はない

### `ParsedProxyInfo` (`src/connection.rs:1929-1940`)

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

- 上記は #0035 実施後のフィールド可視性 (`pub(crate)`) を前提としている
- `password` が全文露出する（致命的）。`username` も文脈によっては機密扱いのためマスクする
- `host` / `port` / `user_agent` は秘密ではないため露出する
- `SoraConnection` (同ファイル 530 行目) は `proxy: Option<ParsedProxyInfo>` を保持するが `Debug` を derive しておらず、本修正の波及はない

### `ProxyInfo` (`src/types.rs:54-60`)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxyInfo {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub user_agent: Option<String>,
}
```

- `password` が全文露出する（致命的）
- `url` は `http://user:pass@host:port/path` の形式を含む可能性があり、userinfo 部にパスワードが埋め込まれている場合に露出する
  - ただし SDK の正規パス (`ParsedProxyInfo::parse`) は userinfo を含む URL を拒否するため、`url` に userinfo が含まれるのは利用者が `ProxyInfo` を直接構築して Debug print した場合に限られる
- `user_agent` は秘密ではないため露出する

### 外部クレートへの影響

- `e2e-tests/`: `TlsConfig` / `ProxyInfo` を使用しているが、Debug 出力に依存するテストはない（`cargo test` で確認済み）
- `pbt/`: `ProxyInfo` を使用しているが、Debug 出力に依存するテストはない
- `examples/sumomo/`: `TlsConfig` / `ProxyInfo` を使用しているが、Debug 出力に依存していない

## 設計方針

### 共通方針

- 値の存在 / 不在は出力する: `Some(<redacted>)` / `None`
- 長さや先頭数バイトは出さない（サイドチャネル回避）
- フィールド名は出す
- 公開証明書 (`client_cert` / `ca_cert`) は PEM 全文を `Some(<present>)` と表記し、ログ圧迫を避ける
- 秘密情報 (`client_key` / `username` / `password`) は `Some(<redacted>)` でマスクする
- `#[derive(Debug)]` から `Debug` を外し、`impl std::fmt::Debug` を手書きする

### `TlsConfig` の例

```rust
impl std::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConfig")
            .field("insecure", &self.insecure)
            .field("client_cert", &self.client_cert.as_ref().map(|_| "<present>"))
            .field("client_key", &self.client_key.as_ref().map(|_| "<redacted>"))
            .field("ca_cert", &self.ca_cert.as_ref().map(|_| "<present>"))
            .finish()
    }
}
```

### `ParsedProxyInfo` の例

```rust
impl std::fmt::Debug for ParsedProxyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedProxyInfo")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username.as_ref().map(|_| "<redacted>"))
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("user_agent", &self.user_agent)
            .finish()
    }
}
```

### `ProxyInfo` の例

`url` から userinfo を取り除いた表現を出す方式を採る。
URL のパースには `shiguredo_http11` の `Uri` を利用し、userinfo 部が存在する場合は `user:pass@` を `<redacted>@` に置換した文字列を出力する。

```rust
impl std::fmt::Debug for ProxyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let masked_url = mask_url_userinfo(&self.url);
        f.debug_struct("ProxyInfo")
            .field("url", &masked_url)
            .field("username", &self.username.as_ref().map(|_| "<redacted>"))
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("user_agent", &self.user_agent)
            .finish()
    }
}

fn mask_url_userinfo(url: &str) -> Cow<'_, str> {
    // http://user:pass@host:port/path → http://<redacted>@host:port/path
    // http://host:port/path → そのまま
    ...
}
```

### スコープ外

- `ProxyInfo` の `PartialEq` / `Eq` derive に起因する timing attack 問題は本 issue のスコープ外。対応要否は親 issue #0020 の S3 グループ内で別途判断する
- `SoraConnectionBuilder` / `SoraConnection` 等の親構造体への `Debug` 実装は本 issue の対象外。現状 `Debug` を derive していないため露出経路にならないことを確認済み

## 完了条件

- `TlsConfig` / `ParsedProxyInfo` / `ProxyInfo` から `Debug` が derive から外れている
- 各構造体に `impl std::fmt::Debug` が手書きで実装され、秘密情報が `<redacted>` でマスクされている
- `client_cert` / `ca_cert` は `Some(<present>)` と出力される
- 単体テストで以下を確認:
  - `client_key` を設定した `TlsConfig` の `format!("{:?}")` 結果に PEM 秘密鍵本文と `-----BEGIN` / `-----END` 境界マーカーが含まれない
  - `client_cert: None` のときは `None`、`Some("...")` のときは `Some("<present>")` が出力される
  - `password` を設定した `ParsedProxyInfo` の `format!("{:?}")` 結果に password 文字列が含まれない
  - `username: None` / `password: None` のときは `None` が出力される
  - `username: Some("")` の空文字列でも `<redacted>` が出力される (存在情報のみ残し値は出さない)
  - `ProxyInfo.url` が `http://user:pass@host:port` の場合に `http://<redacted>@host:port` と出力される
  - `ProxyInfo.url` が `http://host:port` の場合はそのまま出力される
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- `CHANGES.md` に `[CHANGE]` エントリが追加されている

## 解決方法

1. `src/connection.rs:54` `TlsConfig` の `#[derive(Debug, Clone, Default)]` から `Debug` を外す
2. 同ファイル内に `impl std::fmt::Debug for TlsConfig` を追加する
3. `src/connection.rs:1933` `ParsedProxyInfo` の `#[derive(Debug, Clone)]` から `Debug` を外し、手書き実装を追加する (#0035 実施後のフィールド可視性を前提)
4. `src/types.rs:54` `ProxyInfo` の `#[derive(Debug, Clone, PartialEq, Eq, Default)]` から `Debug` を外し、手書き実装と `mask_url_userinfo` ヘルパー関数を追加する
5. `mask_url_userinfo` は `shiguredo_http11` の `Uri` を用いて userinfo 部を検出・マスクする
6. テストデータとしてダミー PEM 文字列を `tests/` に用意する (例: `"<PRIVATE_KEY_MARKER>\n<DUMMY_KEY_BODY>\n<PRIVATE_KEY_MARKER>"` のようなダミー)
7. 単体テストを追加する:
   - `TlsConfig` のテストは `src/connection.rs` の `#[cfg(test)] mod tests` に追加する
   - `ParsedProxyInfo` のテストも同様に `src/connection.rs` に追加する
   - `ProxyInfo` のテストは `src/types.rs` の `#[cfg(test)] mod tests` に追加する
   - テストコメント / アサーションメッセージは日本語 (AGENTS.md)
8. `CHANGES.md` に `[CHANGE] TlsConfig / ParsedProxyInfo / ProxyInfo の Debug 実装を手書き化し秘密情報をマスクする` エントリを追加する
  - @voluntas

## 解決方法

- TlsConfig/ParsedProxyInfo/ProxyInfo の Debug derive を外し手書き実装
- client_key/username/password を <redacted> でマスク
- ProxyInfo.url の userinfo 部を mask_url_userinfo でマスク

### 修正ファイル
- `src/connection.rs`
- `src/types.rs`
- `CHANGES.md`
