# `TlsConfig` / `ParsedProxyInfo` / `ProxyInfo` の `Debug` 実装を手書きにして秘密情報をマスクする

- Priority: High
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/redact-secrets-in-debug-impls
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`TlsConfig` / `ParsedProxyInfo` / `ProxyInfo` の `Debug` 機密露出」分。

## 目的

以下 3 つの公開構造体で `#[derive(Debug)]` をそのまま使っているため、`println!("{:?}", config)` / `tracing::debug!(?config)` / panic backtrace 等で秘密情報がフル露出する:

- `TlsConfig` (`src/connection.rs:54`): `client_cert` / `client_key` (PEM 形式の証明書本文 / 秘密鍵本文)
- `ParsedProxyInfo` (`src/connection.rs:1933`): `username` / `password` (HTTP プロキシ認証情報)
- `ProxyInfo` (`src/types.rs:54`): `username` / `password` / `url` (URL に password が embed されている場合あり)

本 issue では各構造体の `Debug` 実装を手書きに置き換え、秘密情報を `<redacted>` 等でマスクする。

## 優先度根拠

High。

- 秘密鍵 (PEM 形式の `-----BEGIN ...-----` / `-----END ...-----` 境界マーカーで囲まれた本文) や HTTP プロキシ password が、利用者の何気ない debug print / structured logging で本番ログに残ると、SIEM / ログ集約システムからの情報漏洩につながる
- `#[derive(Debug)]` を使う限り、利用者がコードレビューで気付かないと容易に漏れる
- 一度ログに乗ったものは事後に「マスクするように修正した」と言っても回収不可能
- 修正は `impl Debug` を 3 件手書きするだけ
- 正式リリース 2026.1.0 後でも追加可能 (`Debug` 出力フォーマットは SemVer 上「安定とは見なされない」のが一般的) だが、`2026.1.0` の段階で「秘密情報を漏らさない」状態にしておくのが原則

## 現状

### `TlsConfig` (`src/connection.rs:54-64`)

```rust
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    pub insecure: bool,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,   // <- PEM 秘密鍵が全文露出
    pub ca_cert: Option<String>,
}
```

### `ParsedProxyInfo` (`src/connection.rs:1929-1940`)

```rust
#[derive(Debug, Clone)]
pub struct ParsedProxyInfo {
    pub host: String,
    pub port: u16,
    username: Option<String>,
    password: Option<String>,   // <- HTTP プロキシ password が全文露出
    user_agent: String,
}
```

### `ProxyInfo` (`src/types.rs:54-60`)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxyInfo {
    pub url: String,            // <- url に password が含まれる可能性
    pub username: Option<String>,
    pub password: Option<String>,  // <- HTTP プロキシ password が全文露出
    pub user_agent: Option<String>,
}
```

`PartialEq` / `Eq` は別問題 (timing attack の余地。`password` のような秘密値の `==` は constant-time でない `str::eq` を使うため、本来 `Eq` derive は避けるのが望ましい) だが、本 issue では `Debug` のマスクに絞る。`PartialEq` / `Eq` 廃止は別 issue として切り出すかは追加検討する。

## 設計方針

### `Debug` 手書き実装の共通方針

- 値の存在 / 不在は出力する: `Some(<redacted>)` / `None`
- 長さや先頭数バイトは出さない (サイドチャネル回避)
- フィールド名は出す (`f.debug_struct("TlsConfig").field("insecure", &self.insecure).field("client_key", &"<redacted>").finish()` のように)
- マスク用のヘルパー型を 1 つ用意する案もある (例: `struct Redacted<'a, T>(&'a Option<T>)` で `Debug` 実装をひとつにまとめる)

### `TlsConfig` の例

```rust
impl std::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConfig")
            .field("insecure", &self.insecure)
            .field("client_cert", &self.client_cert.as_ref().map(|_| "<redacted>"))
            .field("client_key", &self.client_key.as_ref().map(|_| "<redacted>"))
            .field("ca_cert", &self.ca_cert.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}
```

`ca_cert` は公開証明書なので機密ではないが、PEM 文字列の冗長さで debug ログを圧迫する観点では `<present>` 表記が望ましい。一律マスクを優先するか、可読性とのバランスは実装フェーズで判断する。

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

- `host` / `port` / `user_agent` は秘密ではないので露出
- `username` も特定の文脈で機密扱いなので一律 `<redacted>`

### `ProxyInfo` の例

`url` は `http://user:pass@host:port/path` の形式を含む可能性があるため、`url` 自体も `<redacted>` 化が無難。ただし `url` が秘密でないケース (`http://host:port/`) で利便性が落ちる。

選択肢:

- (A) `url` をパースしてホスト / ポート部のみ出し、user info は伏せる
- (B) 一律 `<redacted>` (実装が単純)
- (C) `url` から userinfo を取り除いた表現を出す

(A) または (C) を採るのが利用者体験的に望ましい。実装フェーズで判断する。

## 完了条件

- `TlsConfig` / `ParsedProxyInfo` / `ProxyInfo` から `#[derive(Debug)]` が外れている
- 各構造体に `impl Debug` が手書きで実装され、秘密情報 (`client_key` / `password` / `username` / 認証情報を含む `url`) が `<redacted>` 等でマスクされている
- マスク後の `Debug` 出力に「フィールドが存在するか」は残しつつ「値自体」が出ない
- 単体テストで以下を確認:
  - `client_key` を設定した `TlsConfig` を `format!("{:?}", config)` した結果に PEM 本文 (PEM の `-----BEGIN ...-----` 境界マーカーで始まる秘密鍵本文) が含まれない
  - `password` を設定した `ParsedProxyInfo` / `ProxyInfo` を `format!("{:?}", info)` した結果に password 文字列が含まれない
  - `client_cert: None` / `password: None` のときは `None` (もしくは類似の不在表現) が出る
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/connection.rs:54` `TlsConfig` の `#[derive(Debug, ...)]` から `Debug` を外す
2. 同ファイル内に `impl std::fmt::Debug for TlsConfig` を追加する (上記設計方針参照)
3. `src/connection.rs:1933` `ParsedProxyInfo` の `#[derive(Debug, ...)]` から `Debug` を外し、手書き実装を追加する
4. `src/types.rs:54` `ProxyInfo` の `#[derive(Debug, ...)]` から `Debug` を外し、手書き実装を追加する
5. `ProxyInfo` の `url` の扱いは選択肢 (A) / (C) のいずれかを採用し、`url` 内の userinfo (`user:pass@`) が含まれていればマスクする
6. 単体テストを `src/connection.rs` の `#[cfg(test)] mod tests` (もしくは `src/types.rs` 側) に追加する
   - 「`client_key` の PEM 本文がマスクされる」「`password` がマスクされる」「`None` のときの表示」をそれぞれ検証する
   - テストコメント / アサーションメッセージは日本語 (AGENTS.md)
7. 親 issue の `Polished` / closed コミット時に、関連箇所 (CHANGES.md の `[CHANGE]` エントリ) を `shiguredo-changelog` 規約に従って更新する
8. `PartialEq` / `Eq` derive の扱い (timing attack 観点) は本 issue のスコープ外。必要があれば別 issue として `#0020` 親 issue 側に追加する
