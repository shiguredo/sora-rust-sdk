# `SoraConnectionCommand` を `pub(crate)` にする

- Priority: High
- Created: 2026-07-02
- Completed: {YYYY-MM-DD}
- Model: DeepSeek V4 Pro
- Branch: feature/change-make-sora-connection-command-pub-crate
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Must 派生 issue M4。

## 目的

`SoraConnectionCommand` は `SoraConnectionHandle` が内部で `SoraConnection` にコマンドを送信するための列挙型であり、ユーザーが直接利用する型ではない。正式リリース前に `pub(crate)` に変更し、公開 API から隠蔽する。

## 優先度根拠

- 正式リリース後では互換破壊の変更ができなくなる
- `/review-code` の致命的指摘の一つ

## 現状

`src/connection.rs:573` で `pub enum SoraConnectionCommand` として公開されている。
`src/lib.rs:18` で `pub use` によりクレートルートから再エクスポートされている。

また `src/error.rs:102-105` の `Error::CommandSendFailed` バリアントが `mpsc::error::SendError<SoraConnectionCommand>` を `source` フィールドとして公開しているため、単に `pub(crate)` にするだけではコンパイルが通らない。

## 設計方針

1. `SoraConnectionCommand` の可視性を `pub` から `pub(crate)` に変更する
2. `lib.rs` の `pub use` から `SoraConnectionCommand` を削除する
3. `Error::CommandSendFailed` の `source` フィールドの型を `mpsc::error::SendError<SoraConnectionCommand>` から公開可能な形式に変更する

## 完了条件

- `SoraConnectionCommand` が `pub(crate)` になっている
- `lib.rs` の `pub use` から `SoraConnectionCommand` が削除されている
- `Error::CommandSendFailed` が `pub(crate)` 型を公開していない
- 全テストが通る
