# `ParsedProxyInfo` のフィールド可視性を統一する

- Priority: Medium
- Created: 2026-06-23
- Completed: 2026-06-25
- Model: Opus 4.7
- Branch: feature/change-align-parsed-proxy-info-field-visibility
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`ParsedProxyInfo` のフィールド可視性整合」分。

## 目的

`src/connection.rs:1929-1940` の `pub struct ParsedProxyInfo` は、フィールド `host` / `port` が `pub` である一方、`username` / `password` / `user_agent` はデフォルト可視性 (private) になっている。
外部利用者は `host` / `port` だけ読み取れて、それ以外は accessor が無いため取り出せない。型自体は `pub` で `lib.rs:18` から再エクスポートされており、一部のフィールドだけが公開された不整合状態である。

全フィールドを `pub(crate)` に統一し、代わりに pub accessor を提供することで、フィールド可視性を揃え、後方互換のないフィールド直接アクセスからの移行を完了させる。

## 優先度根拠

Medium。

- 正式リリース 2026.1.0 で公開された後にフィールド可視性を変えると破壊変更になる (フィールド非公開化、accessor 追加、いずれもユーザー側ビルドを壊す)
- 現状の不整合は機能的なバグではないが、API 設計上のノイズで利用者の混乱を招く
- 修正は機械的で局所
- `Debug` 手動実装による秘密情報のマスク (親 issue S3 の別案件 #0037) と直交するが、先にフィールド可視性を確定させた後、Debug 実装を整える順序になる

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

### `pub use` 再エクスポート (`src/lib.rs:18`)

```rust
pub use crate::connection::{
    ParsedProxyInfo, ...
};
```

再エクスポートは既存の公開 API であるため本 issue では変更しない。
変更後も `ParsedProxyInfo` はクレートルートから参照可能で、accessor 経由でフィールド値を取得する形になる。

### フィールド直接アクセス箇所

`src/connection.rs` 内の直接アクセス (全 15 箇所):

| 行 | アクセス内容 |
|---|---|
| 718 | `proxy.host` |
| 719 | `proxy.port` |
| 720 | `proxy.username` |
| 721 | `proxy.password` |
| 722 | `proxy.user_agent` |
| 2260 | `proxy.host` |
| 2261 | `proxy.port` |
| 2263 | `proxy.host`, `proxy.port` |
| 2343 | `proxy.user_agent` |
| 2344 | `proxy.username` |
| 2345 | `proxy.username`, `proxy.password` |
| 2346 | `proxy.password` |
| 2571 | `parsed.user_agent` (テスト) |
| 2582 | `parsed.user_agent` (テスト) |
| 2593-2599 | 構造体リテラル構築 (テスト) |
| 2616-2622 | 構造体リテラル構築 (テスト) |

`pbt/tests/prop_connection.rs` 内の直接アクセス:

| 行 | アクセス内容 |
|---|---|
| 19 | `parsed.host` |
| 20 | `parsed.port` |

`pbt` クレートは `sora_sdk` の外部クレートであるため、フィールドの `pub(crate)` 化でこれらのアクセスはコンパイルエラーになる。accessor 呼び出しへの置き換えが必要。

## 設計方針

`host` / `port` の `pub` フィールドを `pub(crate)` に下げ、全 5 フィールドの可視性を `pub(crate)` に統一する。
代わりに全フィールドの pub accessor を追加し、外部利用者は accessor 経由で値を取得する形に移行する。

```rust
#[derive(Clone)]
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

- `#[derive(Debug)]` は本 issue では手を付けず、別 issue #0037 で手動実装に置き換える
- re-export は既存の公開 API であるため維持する。フィールド可視性の統一後も利用者は `sora_sdk::ParsedProxyInfo` から accessor 経由でアクセスできる
- `password` accessor はフィールド可視性統一の一貫性のため他の accessor とともに公開する。パスワード生値の露出抑制は #0037 で対応する

## 完了条件

- `ParsedProxyInfo` の全フィールドの可視性が `pub(crate)` に統一されている
- 5 件の accessor (`host()` / `port()` / `username()` / `password()` / `user_agent()`) が `pub fn` として追加されている
- `src/connection.rs` 内の全フィールド直接アクセスが accessor 呼び出しに置き換えられている
- `pbt/tests/prop_connection.rs:19-20` の `parsed.host` / `parsed.port` が `parsed.host()` / `parsed.port()` に置き換えられている
- 単体テスト `build_proxy_connect_request_*` 内のフィールド読み取りも accessor 呼び出しに置き換えられている (`parsed.user_agent` → `parsed.user_agent()` 等)
- accessor の動作確認をする単体テストが `tests/test_connection.rs` に追加されている (全 accessor の正常系・`username` / `password` の `None` ケースを含む)
- `CHANGES.md` に `[CHANGE] ParsedProxyInfo のフィールド可視性を pub(crate) に統一し accessor を追加する` のエントリが追加されている
  - @voluntas
- `skills/sora-rust-sdk/SKILL.md` の `ParsedProxyInfo` のフィールド可視性に関する記述が更新されている (フィールド直接アクセス不可、accessor 経由で取得、に差し替え)
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- `cargo test --test test_connection` で新規単体テストが通過する
- PBT (`cargo test -p pbt`) が通過する

## 解決方法

1. `src/connection.rs:1934-1940`: 全フィールドを `pub(crate)` に変更する。`#[derive(Debug)]` は維持する
2. `src/connection.rs`: `impl ParsedProxyInfo` ブロックに 5 件の pub accessor を追加する
3. `src/connection.rs`: 内部の全フィールド直接アクセス (上記「現状」表の 15 箇所) を accessor 呼び出しに置き換える。構造体リテラルで構築している箇所はそのまま (同一クレート内で `pub(crate)` フィールドに直接代入できるため)
4. `pbt/tests/prop_connection.rs:19-20`: `parsed.host` → `parsed.host()`, `parsed.port` → `parsed.port()` に置き換える
5. `ParsedProxyInfo` の docstring (rustdoc) を更新し、accessor の存在と利用方法を追記する
6. `tests/test_connection.rs` を新規作成し、accessor の単体テストを追加する (全 5 件の正常系 + `username()` / `password()` が `None` を返すケース)
7. `CHANGES.md` に `[CHANGE]` エントリを追加する
8. `skills/sora-rust-sdk/SKILL.md` の `ParsedProxyInfo` に関する記述を、フィールド直接アクセス不可・accessor 経由で取得、という内容に更新する

## 解決方法

- 全フィールドを pub(crate) 化し accessor メソッド追加
- 内部アクセス・pbt・SKILL.md 更新

### 修正ファイル
- `src/connection.rs`
- `pbt/tests/prop_connection.rs`
- `skills/sora-rust-sdk/SKILL.md`
- `tests/test_connection.rs`
- `CHANGES.md`
