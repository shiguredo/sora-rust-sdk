# 公開エイリアス `pub type Result<T>` に rustdoc を追加する

- Priority: Low
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/document-public-result-alias
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`Result<T>` エイリアスの扱い」分。

## 背景

### `pub type Result<T>` は Rust エコシステムで一般的なイディオムである

`pub type Result<T> = std::result::Result<T, Error>;` を公開するパターンは、標準ライブラリをはじめとする多くの著名 crate で採用されている:

| crate | エイリアス |
|---|---|
| `std::io` | `pub type Result<T> = Result<T, io::Error>;` |
| `anyhow` | `pub type Result<T> = Result<T, anyhow::Error>;` |
| `reqwest` | `pub type Result<T> = Result<T, reqwest::Error>;` |
| `serde_json` | `pub type Result<T> = Result<T, serde_json::Error>;` |
| `tokio::io` | `pub type Result<T> = Result<T, io::Error>;` |

標準ライブラリ自体が `std::io::Result<T>` でこのパターンを確立しており、事実上のイディオムである。Rust 開発者は `use some_crate::Result;` でその crate の `Result` をインポートする慣習に慣れており、標準ライブラリの `Result<T, E>` との名前衝突が問題になるケースは稀である（衝突時はフルパスや `use std::result::Result as StdResult;` で回避する）。

したがって `sora_sdk` が `pub type Result<T>` を公開し、クレートルートから再エクスポートすること自体は妥当な API 設計である。

### 問題点

rustdoc コメントが付いていないため、docs.rs 上で「これは何の Result か」が分からない。

## 目的

`src/error.rs` の `pub type Result<T>` に rustdoc を追記し、利用ガイドを明確にする。

## 優先度根拠

Low。

- 利用者の体験上の小さな不便にとどまる (rustdoc が無いため docs.rs での可読性が落ちる)
- 機能上は問題なく、SemVer 上の制約も「rustdoc 追加」自体は破壊変更ではない

## 現状

### `src/error.rs:489`

```rust
pub type Result<T> = std::result::Result<T, Error>;
```

- rustdoc コメント無し
- エラー型 `Error` は `src/error.rs` で定義されている (本 SDK 内部の `Error` 型)

### `src/lib.rs:24`

```rust
pub use crate::error::{Error, Result};
```

- クレートルートで `Result` を再エクスポート

## 設計方針

`pub use` は維持し、rustdoc を追加するのみとする。

```rust
/// SDK 内のエラー型 [`Error`] を組み合わせた `std::result::Result` の 1 引数エイリアス。
///
/// `crate::Result<T>` は `Result<T, sora_sdk::Error>` のショートハンドである。
/// 標準ライブラリの `Result<T, E>` と名前が衝突するため、利用箇所では
/// `sora_sdk::Result<T>` の形でモジュールパス込みで参照することを推奨する。
pub type Result<T> = std::result::Result<T, Error>;
```

- `pub type Result<T>` の公開は Rust エコシステムで一般的なイディオムであり、削除する理由はない
- 公開 API の利便性を維持しつつ、rustdoc で誤用を抑制できる
- API 互換性を維持できる

## 完了条件

- `src/error.rs` の `pub type Result<T>` に rustdoc が追加されている
- rustdoc に (1) エイリアスの中身、(2) `std::result::Result` との名前衝突への注意、(3) 推奨される import / フルパス利用、が記述されている
- `cargo doc -D warnings` で warning が出ない (親 issue S4 ジョブが入っていればそちらで)
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/error.rs` に rustdoc を追加する (上記設計方針の例文を出発点に文言を磨く)
2. クレートルートの `Result` のドキュメント (`src/lib.rs` のクレートドキュメント) からもエイリアスへの言及をたどれるようにする (親 issue M6 (lib.rs クレートドキュメント拡充) と整合させる)
3. 利用者ドキュメント (rustdoc 例) で `fn foo() -> sora_sdk::Result<()> { ... }` の形を推奨する
