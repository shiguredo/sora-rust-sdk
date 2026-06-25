# `TlsConfig` のフィールド直接アクセスと builder メソッドの二重インターフェースを一本化する

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-flatten-tls-config-interface
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`TlsConfig` の二重インターフェース」分。

## 目的

`src/connection.rs:55-64` の `TlsConfig` は全フィールドが `pub` で公開されており、利用者がフィールドに直接代入できる。同時に `SoraConnectionBuilder` 側 (`src/connection.rs:398-415`) には `insecure()` / `client_cert()` / `ca_cert()` の builder メソッドが用意されていて、内部でフィールドを書き換えている。つまり、`TlsConfig` の構築経路が「構造体リテラルで直接フィールド代入」と「`SoraConnectionBuilder` の builder メソッド」の 2 系統存在する。

この二重インターフェースには以下の問題がある:

- フィールドを追加するたびに builder メソッドとフィールドの両方を同期する必要があり、漏れたときに 1 経路だけ整備された API が残る
- 利用者がどちらを使えば良いか判断しづらい (rustdoc 上はどちらも有効)
- 将来フィールドの可視性を変更しようとしても、`pub` フィールドに依存している外部コードが存在すると SemVer 破壊変更になる

本 issue では、`TlsConfig` のインターフェースを一本化する。具体的には、フィールドを `pub(crate)` 化して構造体リテラル経路を封鎖し、builder pattern による構築に統一する。

## 優先度根拠

Medium。

- 正式リリース 2026.1.0 で公開 API として固定すると、後から「フィールドを `pub(crate)` 化する」も「builder メソッドを廃止する」も SemVer 破壊変更になり取り戻せない
- `TlsConfig` の利用パターン (構造体リテラル vs builder) を確定させないと、他 issue (#0035 `ParsedProxyInfo` 可視性統一、#0037 `Debug` 手書き実装) との API 設計整合が取れず、後続作業で手戻りが発生する
- 修正範囲が `src/connection.rs` 内に限定されており、他モジュールへの副作用リスクが低い

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

### 確認済みの事実

- `TlsConfig { .. }` 構造体リテラルによる構築は本リポジトリ内で使用実績ゼロ (定義行を除く)
- `tls_config(TlsConfig)` のような setter は現状存在しない
- クレート内部からの `TlsConfig` フィールド直接アクセスは以下の 5 箇所:
  - `SoraConnectionBuilder::insecure()` (`src/connection.rs:400`): `self.tls_config.insecure = value`
  - `SoraConnectionBuilder::client_cert()` (`src/connection.rs:406-407`): `self.tls_config.client_cert` / `client_key`
  - `SoraConnectionBuilder::ca_cert()` (`src/connection.rs:413`): `self.tls_config.ca_cert`
  - `build_tls_client_config()` (`src/connection.rs:2189-2227`): `tls_config.insecure` / `client_cert` / `client_key` / `ca_cert`
  - `build_tls_client_config_with_client_auth()` (`src/connection.rs:2238`): `tls_config.client_cert` / `client_key`
- `SoraConnectionBuilder` 内の `tls_config` フィールドは private (`src/connection.rs:119`)
- sumomo (`examples/sumomo/src/main.rs:296-301`) は `SoraConnectionBuilder` の builder メソッド経由でのみ `TlsConfig` を設定しており、構造体リテラル不使用
- `e2e-tests` でも `TlsConfig` は直接構築されておらず、builder メソッド経由のみ
- `TlsConfig` にアクセサ（getter）は存在しない
- `turn_tls_insecure` / `turn_tls_ca_cert` は `SoraConnectionBuilder` の独立フィールドであり、`TlsConfig` 構造体とは別 (命名は似ているが TURN-TLS 用で対象が異なる。`TlsConfig` の docstring にも明記済み)
- 本リポジトリでは `#[non_exhaustive]` の使用実績はゼロ。`shiguredo-rust` スキルは `#[non_exhaustive]` を禁止している

## 設計方針

### 選択肢 A (推奨): フィールドを `pub(crate)` 化し、builder pattern に一本化

```rust
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
    pub fn client_cert(mut self, cert: String, key: String) -> Self {
        self.client_cert = Some(cert);
        self.client_key = Some(key);
        self
    }
    pub fn ca_cert(mut self, cert: String) -> Self { self.ca_cert = Some(cert); self }
}
```

- `#[non_exhaustive]` は付けない。`shiguredo-rust` スキルが禁止しており、`pub(crate)` フィールドにはそもそも外部効果がない。`issues/closed/0019` でも同判断
- `SoraConnectionBuilder` 側の `insecure()` / `client_cert()` / `ca_cert()` は **維持** し、内部実装を `TlsConfig` の builder メソッド呼び出しに置き換える
  - 維持する理由: 既存利用者 (sumomo 等) のコード互換を保つため。`SoraConnectionBuilder` の他メソッドとの一貫性もある
  - 内部実装例: `self.tls_config = self.tls_config.insecure(value);`
- `TlsConfig` の builder メソッドは `pub` で公開する。現時点では `SoraConnectionBuilder` に `tls_config(TlsConfig)` setter がなく、外部利用者が `TlsConfig` 単体で構築しても渡す経路はないが、将来 `tls_config()` setter を追加する際に builder メソッドが既に整備されていることで API 拡張がスムーズになる。また、rustdoc 上に `TlsConfig` の正規の構築手段として builder pattern を明示できる
- `client_cert()` を cert と key をペアで受け取る設計は維持する。両フィールドは論理的に一対であり、片方のみ設定しても `build_tls_client_config()` で `ClientCertKeyIncomplete` エラーになるため、API レベルで対であることを強制する
- `TlsConfig::new()` は `Default` を呼ぶだけだが、`Default` トレイトのみに依存すると「default → setter 呼び出し」のイディオムが `Default::default()` になる。明示的な `new()` を提供することで、利用者が builder pattern を自然に使えるようになる
- アクセサ（getter）は提供しない。`TlsConfig` の用途は設定を構築して `SoraConnectionBuilder` に渡すことのみであり、外部からフィールド値を読み取るユースケースがない。今後必要になれば別 issue で対応する

### 選択肢 B (不採用): `pub` フィールド維持 + builder メソッド削除

`SoraConnectionBuilder` 側のメソッドを削除し、利用者は `TlsConfig { insecure: true, ... }` の構造体リテラルで直接組み立てる。ただし構造体リテラルはフィールド追加時に全利用箇所の修正が必要になり、API として脆弱。また `SoraConnectionBuilder` の他メソッドとスタイルが揃わず一貫性を損なうため不採用。

### 推奨

選択肢 A を採用する。理由:

- フィールド `pub(crate)` 化により構造体リテラル経路を封鎖し、構築経路を builder pattern に一本化できる
- builder pattern は `SoraConnectionBuilder` 全体ですでに採用されており、API の一貫性が保てる
- フィールド `pub(crate)` 化はクレート内部からはフィールドアクセスできるため、`build_tls_client_config()` 等の内部コードは修正不要
- `SoraConnectionBuilder` 側のメソッドを維持するため、既存利用者 (sumomo、e2e-tests) のコードは変更不要

### 関連 issue との依存関係

- **#0037 (Debug 手書き実装)**: 本 issue を先に実施する。#0037 は `#[derive(Debug)]` を外して `impl Debug` を手書きするが、本 issue でフィールドの `pub(crate)` 化が完了していれば、#0037 の Debug 実装内でもフィールドアクセスが同じクレート内で可能なため問題ない
- **#0035 (ParsedProxyInfo 可視性統一)**: 同じ S3 グループ内だが対象型が異なるため独立。両方ともフィールド `pub(crate)` 化の方向で統一される（#0035 は accessor getter を提供、本 issue は accessor を提供しない）

## 完了条件

- `TlsConfig` の全フィールドが `pub(crate)` 化されている
- `TlsConfig` に `new()` および各フィールドの builder setter メソッド (`insecure()` / `client_cert()` / `ca_cert()`) が追加されている
- `SoraConnectionBuilder` 側の `insecure()` / `client_cert()` / `ca_cert()` の内部実装が、`TlsConfig` の builder メソッド呼び出しに置き換わっている
- `build_tls_client_config()` / `build_tls_client_config_with_client_auth()` のフィールド直接アクセスが `pub(crate)` 後も同一ファイル内なので変更不要なことを確認できている
- `TlsConfig` の rustdoc に builder pattern での構築例 (`TlsConfig::new().insecure(true).ca_cert(ca)`) が記載されている
- `src/connection.rs` の `#[cfg(test)] mod tests` に、`TlsConfig` の builder chain が期待する内部状態を生成することを確認する単体テストが追加されている
- `examples/sumomo` および `e2e-tests` が既存の builder メソッド経由で引き続きビルド・動作すること（フィールド直接アクセス不使用のため修正不要、検証のみ）
- `CHANGES.md` に `[CHANGE]` エントリが追加され、`TlsConfig` のフィールド `pub` → `pub(crate)` 化と builder メソッドの追加が記載されている。移行ガイダンス（`TlsConfig { insecure: true, .. }` → `TlsConfig::new().insecure(true)`）も付記する
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- `cargo doc -D warnings` で warning が出ない

## 解決方法

1. `TlsConfig` の 4 フィールド (`insecure` / `client_cert` / `client_key` / `ca_cert`) を `pub` → `pub(crate)` に変更する (`src/connection.rs:57,59,61,63`)。`src/lib.rs` の `pub use crate::connection::TlsConfig` は型自体が `pub` のままなので変更不要であることを確認する
2. `impl TlsConfig` ブロックを追加し、`new()` / `insecure()` / `client_cert()` / `ca_cert()` の 4 メソッドを実装する
   - `new()` は `Self::default()` を呼ぶだけ
   - `client_cert(cert, key)` は `cert` と `key` をペアで受け取る（現行の `SoraConnectionBuilder` API に揃える）
3. `SoraConnectionBuilder::insecure()` / `client_cert()` / `ca_cert()` の内部実装を、`TlsConfig` の builder メソッド呼び出しに置き換える
   - `self.tls_config.insecure = value;` → `self.tls_config = self.tls_config.insecure(value);`
   - `self.tls_config.client_cert = Some(cert); self.tls_config.client_key = Some(key);` → `self.tls_config = self.tls_config.client_cert(cert, key);`
   - `self.tls_config.ca_cert = Some(cert);` → `self.tls_config = self.tls_config.ca_cert(cert);`
4. `build_tls_client_config()` (`src/connection.rs:2187`) と `build_tls_client_config_with_client_auth()` (`src/connection.rs:2234`) のフィールド直接アクセスは同一ファイル内なのでそのまま維持する（変更不要）
5. `TlsConfig` の rustdoc に builder pattern の使用例を追記する (`src/connection.rs:51-64`)
6. `src/connection.rs` の `#[cfg(test)] mod tests` に以下の単体テストを追加する
   - `TlsConfig::new()` の全フィールドがデフォルト値であることの確認
   - builder chain (`new().insecure(true).client_cert(cert, key).ca_cert(ca)`) が期待する内部状態を生成することの確認
   - 複数回呼び出し時の上書き挙動の確認
7. `examples/sumomo` (`src/main.rs:296-301`) および `e2e-tests` は既存の `SoraConnectionBuilder` builder メソッド経由で設定しており、フィールド `pub(crate)` 化の影響を受けないことを確認する（修正不要）
8. `CHANGES.md` の `## develop` に `[CHANGE]` エントリを追加する
   - `TlsConfig` のフィールドを `pub` → `pub(crate)` に変更し、builder メソッド (`new()` / `insecure()` / `client_cert()` / `ca_cert()`) を追加したことを記載
   - 移行方法: `TlsConfig { insecure: true, .. }` → `TlsConfig::new().insecure(true)` を明記
