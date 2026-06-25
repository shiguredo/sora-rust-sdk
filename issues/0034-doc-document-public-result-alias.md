# 公開エイリアス `pub type Result<T>` に rustdoc を追加する

- Priority: Low
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/update-public-result-alias
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`Result<T>` エイリアスの扱い」分。本 issue の作業は rustdoc 追加のみであり API 設計変更は伴わないが、S3 の他の項目と同様に公開 API の品質向上を目的とする。

## 背景

`sora_sdk` のエラー型は `Error` であり、`pub type Result<T> = std::result::Result<T, Error>;` が公開 API として提供されている。
このエイリアスは `src/error.rs:489` で定義され、`src/lib.rs:24` でクレートルートに再エクスポートされている。

e2e テストの一部では `use sora_sdk::Result;` の形での import が使われている。

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

- エラー型 `Error` は `src/error.rs` で定義されている (本 SDK 内部の `Error` 型)
- `Error` 型自体への rustdoc は親 issue M6 (公開 API の rustdoc 拡充) で対応予定のため本 issue の範囲外

### `src/lib.rs:24`

```rust
pub use crate::error::{Error, Result};
```

- クレートルートで `Result` を再エクスポートしている (既存の公開 API)
- rustdoc は定義側 (`src/error.rs`) に追加し、`pub use` 経由で docs.rs に表示される挙動に委ねる

## 設計方針

`pub use` による再エクスポートは既存の公開 API であるため本 issue では変更しない。`src/error.rs` の型エイリアス定義に rustdoc を追加するのみとする。

```rust
/// SDK のエラー型 [`Error`] をエラーパラメータに持つ `std::result::Result` の 1 引数エイリアス。
///
/// 通常の利用では `use sora_sdk::Result;` でインポートする。
/// 標準ライブラリの `Result<T, E>` と名前が衝突する場合は
/// `use std::result::Result as StdResult;` で回避する。
///
/// ```
/// use sora_sdk::Result;
///
/// fn example() -> Result<()> {
///     Ok(())
/// }
/// ```
pub type Result<T> = std::result::Result<T, Error>;
```

- 利用者の標準的なインポートパターン (`use sora_sdk::Result;`) を推奨する
- 名前衝突時の回避策も示し、実用上のカバー範囲を広げる
- コード例を入れて利用者がコピー & ペーストで使い始められるようにする

## 完了条件

- `src/error.rs` の `pub type Result<T>` に rustdoc が追加されている
- rustdoc に (1) エイリアスの中身、(2) 推奨される import 方法、(3) `std::result::Result` との名前衝突回避策、が記述されている
- rustdoc 内にコード例が含まれている
- `cargo test --doc` で doc-test が通過する
- `cargo doc -D warnings` で warning が出ない
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/error.rs` の `pub type Result<T>` に rustdoc を追加する (上記設計方針の案文を出発点に文言を磨く)
2. クレートルートのクレートドキュメント (`src/lib.rs`) からのエイリアスへの言及は親 issue M6 で対応する。本 issue では `src/error.rs` 側の rustdoc が `pub use` 経由でクレートルートのドキュメントとしても表示されるため、対応済みとなる
3. 利用者ドキュメント (rustdoc 例) で `use sora_sdk::Result;` の形を推奨し、コード例も含める
