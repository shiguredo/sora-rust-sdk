# `ParsedProxyInfo` のフィールド可視性を統一する

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/align-parsed-proxy-info-field-visibility
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`ParsedProxyInfo` のフィールド可視性整合」分。

## 目的

`src/connection.rs:1929-1940` の `ParsedProxyInfo` は `pub struct` でありながら、フィールドの可視性が `host` / `port` のみ `pub` で、`username` / `password` / `user_agent` は private になっている。

```rust
#[derive(Debug, Clone)]
pub struct ParsedProxyInfo {
    pub host: String,
    pub port: u16,
    username: Option<String>,
    password: Option<String>,
    user_agent: String,
}
```

外部利用者は `host` / `port` だけ読み取れて、それ以外は (accessor が無いため) 取り出せない。型自体は `pub` で `lib.rs:18` から再エクスポートされており、外から見える状態が不整合になっている。

rustdoc には「PBT 等の検証目的を主用途として公開している型のため、通常の利用者がこの型を直接構築する必要はなく、`ParsedProxyInfo::parse` 経由で取得する」と書かれており、検証目的の公開と一般利用の境界が曖昧。本 issue では可視性をどちらかに揃え、公開 API の意図を明確にする。

## 優先度根拠

Medium。

- 正式リリース 2026.1.0 で公開された後にフィールド可視性を変えるとほぼすべての操作が破壊変更になる (フィールド非公開化、accessor 追加、`pub` 取り下げ、いずれもユーザー側ビルドを壊す)
- 現状の不整合は機能的なバグではないが、API 設計上のノイズで利用者の混乱を招く
- 修正は機械的で局所
- `Debug` derive で秘密情報 (`password`) が露出する別問題 (本親 issue S3 の Debug 機密露出案件) と直交するが、本 issue で「フィールド可視性をどうするか」を確定させた後、Debug の出力 (見えるフィールドだけマスク不要、隠したフィールドはマスク要) を整える順序になるため、先に本 issue を解決するのが望ましい

## 現状

### `ParsedProxyInfo` 定義 (`src/connection.rs:1929-1940`)

```rust
/// `ProxyInfo` を解析し、HTTP プロキシ接続に必要な情報に正規化した結果。
///
/// PBT 等の検証目的を主用途として公開している型のため、通常の利用者がこの型を
/// 直接構築する必要はなく、`ParsedProxyInfo::parse` 経由で取得する。
#[derive(Debug, Clone)]
pub struct ParsedProxyInfo {
    pub host: String,
    pub port: u16,
    username: Option<String>,
    password: Option<String>,
    user_agent: String,
}
```

### 公開状況

- `src/lib.rs:18` で `pub use crate::connection::{ParsedProxyInfo, ...};` で再エクスポート済
- `ParsedProxyInfo::parse` (`src/connection.rs:1947` 付近) は `pub fn` で、検証目的での生成入口

### 問題

- `host` / `port` のみフィールド `pub`、残り 3 件は private
- accessor (`fn username(&self) -> Option<&str>` 等) は存在しない (要確認: grep 必要)
- 外部から `username` / `password` / `user_agent` を読む手段が無い (再構築 → `parse` 経由しか無い)

## 設計方針

### 選択肢 A: フィールドを全て `pub(crate)` に下げ、accessor を 5 件公開する

```rust
#[derive(Clone)]
#[non_exhaustive]
pub struct ParsedProxyInfo {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) user_agent: String,
}

impl ParsedProxyInfo {
    pub fn host(&self) -> &str { &self.host }
    pub fn port(&self) -> u16 { self.port }
    pub fn username(&self) -> Option<&str> { self.username.as_deref() }
    pub fn password(&self) -> Option<&str> { self.password.as_deref() }
    pub fn user_agent(&self) -> &str { &self.user_agent }
}
```

- 一般的な公開構造体パターンで、将来内部表現を変えても accessor 互換は維持しやすい
- `#[non_exhaustive]` も付けやすい (フィールド `pub` で破綻しない)
- 秘密情報 (`password`) を返す accessor の公開可否を別 issue (Debug 機密露出案件) で判断する余地が残る

### 選択肢 B: 型を `pub(crate)` に下げ、PBT 専用の `pub(crate)` シンボルとして提供する

```rust
#[cfg(any(test, feature = "_pbt"))]
pub struct ParsedProxyInfo { ... }

#[cfg(not(any(test, feature = "_pbt")))]
pub(crate) struct ParsedProxyInfo { ... }
```

- `pbt` 外部クレート (親 issue closed の `#0019` で抽出済) からは feature gate 経由で参照する
- 利用者からは見えなくなる
- `lib.rs:18` の再エクスポートも feature gate 化

### 推奨

選択肢 A を採る。理由:

- 構造体自体は `pub` で残し、accessor で読みやすくするのが Rust の公開 API の通常パターン
- rustdoc に書いてある「PBT 等の検証目的」は外部 PBT クレートが本 SDK の `pub` を参照することを想定している
- `#[non_exhaustive]` (親 issue M3) と組み合わせると、将来フィールド追加で破壊変更にならない
- 秘密情報 (`password`) を accessor で公開することの是非は別 issue (Debug 機密露出案件) で扱う。本 issue では「全 accessor 公開」を基本線とし、accessor 公開の可否は連動して判断する

## 完了条件

- `ParsedProxyInfo` の全フィールドが `pub(crate)` に揃っている (もしくは型自体が `pub(crate)` に下がっている)
- 必要な accessor (`host()` / `port()` / `username()` / `password()` / `user_agent()` の 5 件) が `pub fn` として用意されている (選択肢 A) もしくは型公開自体を取りやめている (選択肢 B)
- rustdoc が「公開意図 (PBT 検証目的 / 一般利用)」「accessor の利用方法」を明示している
- `#[non_exhaustive]` (親 issue M3) と整合している
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `ParsedProxyInfo` の `pub` フィールド (`host` / `port`) を `pub(crate)` に下げる
2. private フィールド (`username` / `password` / `user_agent`) もそのまま `pub(crate)` で記述する (実質変化なし)
3. accessor を `impl ParsedProxyInfo` に 5 件追加する
4. `src/connection.rs` 内で `proxy.host` / `proxy.port` のような直接アクセスを accessor 呼び出しに置き換える
5. `#[non_exhaustive]` 付与は親 issue M3 と一括で行うため、本 issue 内ではコメントで言及するのみ
6. rustdoc を更新し、「PBT 検証目的の公開」「accessor 経由でフィールドを取得」「`parse` で構築」を整理する
7. テストでは accessor が想定どおりの値を返すことを確認する単体テストを `src/connection.rs` の `#[cfg(test)] mod tests` に追加する
8. 秘密情報の `Debug` 出力に関する対応は本 issue とは別建てとし、別 issue (Debug 機密露出案件 = `#0037` 想定) で扱う
