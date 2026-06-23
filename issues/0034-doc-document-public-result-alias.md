# 公開エイリアス `pub type Result<T>` に rustdoc を追加する (もしくは公開を取りやめる)

- Priority: Low
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/document-public-result-alias
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`Result<T>` エイリアスの扱い」分。

## 目的

`src/error.rs:489` で `pub type Result<T> = std::result::Result<T, Error>;` が定義され、`src/lib.rs:24` で `pub use crate::error::{Error, Result};` で再エクスポートされている。`Result` は 1 引数のエイリアスだが、標準ライブラリの `Result` (2 引数) と同名で、利用者がインポートしたときに混乱しやすい。さらに rustdoc コメントが付いていないため、docs.rs 上で「これは何の Result か」が分からない。

本 issue では rustdoc を追記し、利用ガイドを明確にする (もしくは `pub use` から外して `sora_sdk::error::Result<T>` 経由に限定する)。

## 優先度根拠

Low。

- 利用者の体験上の小さな不便にとどまる (型推論で混乱する程度)
- 機能上は問題なく、SemVer 上の制約も「rustdoc 追加」自体は破壊変更ではない
- ただし `pub use` から外す方針を選んだ場合は破壊変更になるため、2026.1.0 のタイミングで決着させたい

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
- 利用者は `use sora_sdk::Result;` で 1 引数 `Result<T>` を取り込めるが、標準ライブラリの `Result<T, E>` と名前が衝突する

## 設計方針

### 選択肢 A (推奨): rustdoc を追加し、`pub use` は維持する

```rust
/// SDK 内のエラー型 [`Error`] を組み合わせた `std::result::Result` の 1 引数エイリアス。
///
/// `crate::Result<T>` は `Result<T, sora_sdk::Error>` のショートハンドである。
/// 標準ライブラリの `Result<T, E>` と名前が衝突するため、利用箇所では
/// `sora_sdk::Result<T>` の形でモジュールパス込みで参照することを推奨する。
pub type Result<T> = std::result::Result<T, Error>;
```

- 利用者の混乱は減らせる
- API 互換性を維持できる

### 選択肢 B: `pub use crate::error::Result` から外す

`src/lib.rs:24` を `pub use crate::error::Error;` のみにし、`Result` は `sora_sdk::error::Result<T>` 経由でのみ参照可能にする。

- 標準ライブラリの `Result` との衝突を完全に避けられる
- 既存利用者 (主にサンプル `examples/sumomo`) が `sora_sdk::Result` で参照しているなら破壊変更
- 2026.1.0 前なら入れられるが、利用感が落ちる可能性がある

### 推奨

選択肢 A を採る。理由:

- 公開 API の利便性を維持しつつ、rustdoc で誤用を抑制できる
- 選択肢 B の「ショートハンドを完全に廃止」は利便性とトレードオフで、本 SDK の他の builder API のシンプルさと整合しない

## 完了条件

- `src/error.rs:489` の `pub type Result<T>` に rustdoc が追加されている
- rustdoc に (1) エイリアスの中身、(2) `std::result::Result` との名前衝突への注意、(3) 推奨される import / フルパス利用、が記述されている
- (選択肢 B を採る場合) `src/lib.rs:24` から `Result` の再エクスポートが外れている
- `cargo doc -D warnings` で warning が出ない (親 issue S4 ジョブが入っていればそちらで)
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/error.rs:489` に rustdoc を追加する (上記選択肢 A の例文を出発点に文言を磨く)
2. クレートルートの `Result` のドキュメント (`src/lib.rs` のクレートドキュメント) からもエイリアスへの言及をたどれるようにする (親 issue M6 (lib.rs クレートドキュメント拡充) と整合させる)
3. 選択肢 B を採る場合は `src/lib.rs:24` を `pub use crate::error::Error;` に変更し、`Result` 利用箇所を `sora_sdk::error::Result` か関数内 `use crate::error::Result;` に書き換える (本リポジトリ内のサンプル / テストを優先)
4. 利用者ドキュメント (rustdoc 例) で `fn foo() -> sora_sdk::Result<()> { ... }` の形を推奨する
