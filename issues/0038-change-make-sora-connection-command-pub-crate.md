# `SoraConnectionCommand` を `pub(crate)` にする

- Priority: High
- Created: 2026-07-02
- Completed: {YYYY-MM-DD}
- Model: DeepSeek V4 Pro
- Branch: feature/change-make-sora-connection-command-pub-crate
- Polished: 2026-07-02

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Must 派生 issue M4。

## 目的

`SoraConnectionCommand` は `SoraConnectionHandle` が内部で `SoraConnection` にコマンドを送信するための列挙型であり、ユーザーが直接利用する型ではない。正式リリース前に `pub(crate)` に変更し、公開 API から隠蔽する。

## 優先度根拠

- 正式リリース後では互換破壊の変更ができなくなる

## 現状

`src/connection.rs:573` で `pub enum SoraConnectionCommand` として公開されている。
`src/lib.rs:18` で `pub use` によりクレートルートから再エクスポートされている。

`src/error.rs:8` で `use crate::connection::SoraConnectionCommand;` としてインポートされており、`src/error.rs:102-105` の `Error::CommandSendFailed` バリアントが `mpsc::error::SendError<SoraConnectionCommand>` を `source` フィールドとして保持している。
このため `Error` （`pub enum`）のフィールドが `pub(crate)` 型を露出する形になり、単に `pub(crate)` にするだけではコンパイルが通らない。

`Error` は `src/error.rs:10` で `#[derive(Debug)]` が適用されており、`CommandSendFailed` の Debug 出力には `SendError<SoraConnectionCommand>` の Debug 出力が含まれる。また `src/error.rs:381` で `std::error::Error::source()` が `SendError<SoraConnectionCommand>` を返している。

## 設計方針

### 1. `SoraConnectionCommand` の可視性変更

`src/connection.rs:573`:

```rust
// Before
pub enum SoraConnectionCommand { ... }

// After
pub(crate) enum SoraConnectionCommand { ... }
```

### 2. `lib.rs` の `pub use` からの削除

`src/lib.rs:17-20` の `pub use` から `SoraConnectionCommand` を削除する。

```rust
// Before
pub use crate::connection::{
    ParsedProxyInfo, SoraConnection, SoraConnectionBuilder, SoraConnectionCommand,
    SoraConnectionHandle, TlsConfig,
};

// After
pub use crate::connection::{
    ParsedProxyInfo, SoraConnection, SoraConnectionBuilder,
    SoraConnectionHandle, TlsConfig,
};
```

### 3. `Error::CommandSendFailed` の型変更

`src/error.rs:102-105` の `source` フィールドの型を `mpsc::error::SendError<SoraConnectionCommand>` から `String` に変更し、フィールド名も `reason` にリネームする。

既存の `Error` 列挙型では、`std::error::Error::source()` で返されるフィールドにのみ `source` という名前が使われている（例: `DnsResolve { source: io::Error }`, `TcpConnect { source: io::Error }`）。
一方、`std::error::Error` を実装しない `String` 型フィールドには `reason` または `message` が使われている（例: `SetRemoteDescriptionFailed { reason: String }`, `TurnTlsCaCert { message: String }`）。
本変更では `source` を `String` にし `source()` から外すため、命名規約に従いフィールド名を `reason` に変更する。

`SendError<SoraConnectionCommand>` の `Display` 出力を `.to_string()` で取り出すことでエラーメッセージを保持する。
これにより `Error` の `Debug` 出力は、`SendError<SoraConnectionCommand>` の構造体ダンプから単なる `reason: "sending on a closed channel"` のような文字列表示に変わるが、`pub(crate)` 型の情報が漏洩することはなくなる。

また `source()` から `CommandSendFailed` アームを削除する。`SendError` 経由のエラーチェーンは失われるが、Mpsc チャネル送信失敗を downcast で検出するユースケースは存在せず、正式リリース前の破壊変更として許容する。

```rust
// Before
CommandSendFailed {
    source: mpsc::error::SendError<SoraConnectionCommand>,
    command: &'static str,
},

// After
CommandSendFailed {
    reason: String,
    command: &'static str,
},
```

#### `Display` の変更

フィールド名が `source` → `reason` に変わるため、`Display` 実装も追従する。
フォーマット文字列は変更不要 （`"{reason}"` で `String` の内容がそのまま表示される）。

`src/error.rs:290-292`:

```rust
// Before
Error::CommandSendFailed { command, source } => {
    write!(f, "コマンドの送信に失敗しました: {command}: {source}")
}

// After
Error::CommandSendFailed { command, reason } => {
    write!(f, "コマンドの送信に失敗しました: {command}: {reason}")
}
```

#### `source()` の変更

`src/error.rs:381` の `CommandSendFailed` アームを削除する。
`String` は `std::error::Error` を実装していないため `source()` で返せない。

```rust
// Before (行 381)
Error::CommandSendFailed { source, .. } => Some(source),

// After
// CommandSendFailed アームを削除。match は _ => None に fallthrough する。
```

#### エラー構築箇所の変更

`src/connection.rs:516-527` の `send_command()` 内:

```rust
// Before (行 524)
.map_err(|source| Error::CommandSendFailed { source, command })?;

// After
.map_err(|e| Error::CommandSendFailed {
    reason: e.to_string(),
    command,
})?;
```

#### `use` 文の整理

```rust
// Before (src/error.rs:6,8)
use tokio::sync::{mpsc, oneshot};
use crate::connection::SoraConnectionCommand;

// After
use tokio::sync::oneshot;
// SoraConnectionCommand の use 文は削除
```

`mpsc` は `CommandSendFailed` の `SendError<SoraConnectionCommand>` 以外で `error.rs` 内で使われていないため、`oneshot` 単独のインポートに変更する。`oneshot` は `CommandResponseMissing` (行 107) で必要。

### 4. テストの修正

`src/connection.rs:2685-2714` の `url_getters_return_send_error_after_run_loop_stops` は `mpsc::unbounded_channel::<SoraConnectionCommand>()` を直接使用している。
`pub(crate)` 化後もクレート内テストのためコンパイルは通る。
テストアサーションでは `Error::CommandSendFailed { command: _, .. }` でパターンマッチしており、フィールド名が `source` から `reason` に変わっても `..` で無視しているため修正不要である。

### 5. ドキュメントの修正

`skills/sora-rust-sdk/SKILL.md:66` の `SoraConnectionCommand` に関する行を以下のように修正する。

```markdown
// Before
| `SoraConnectionCommand` | `SoraConnectionHandle` が内部的に送信するコマンドの enum。通常はユーザーが直接構築しない |

// After
// ↑の行を削除する
```

### 6. 変更履歴

`CHANGES.md` の `## develop` セクションに以下のエントリを追加する。

```markdown
- [CHANGE] `SoraConnectionCommand` の可視性を `pub` から `pub(crate)` に変更し `Error::CommandSendFailed` の型を変更する
  - `SoraConnectionCommand` は公開 API から削除され、外部からの参照は不可になる
  - `Error::CommandSendFailed` の `source` フィールドを `SendError<SoraConnectionCommand>` から `reason: String` に変更し、`std::error::Error::source()` からも除外する
  - @melpon
```

## 完了条件

- `src/connection.rs:573` の `SoraConnectionCommand` が `pub(crate)` になっている
- `src/lib.rs:17-20` の `pub use` から `SoraConnectionCommand` が削除されている
- `src/error.rs:102-105` の `CommandSendFailed` のフィールドが `reason: String` と `command: &'static str` に変更されている
- `src/error.rs:290-292` の `Display` 実装の `source` が `reason` に変更されている
- `src/error.rs:381` の `CommandSendFailed` に対応する `source()` アームが削除されている
- `src/error.rs:6` の `use tokio::sync::{mpsc, oneshot};` から `mpsc` が削除され `use tokio::sync::oneshot;` になっている
- `src/error.rs:8` の `use crate::connection::SoraConnectionCommand;` が削除されている
- `src/connection.rs:524` のエラー構築でフィールド名 `source` が `reason` に、値が `e.to_string()` に変更されている
- `skills/sora-rust-sdk/SKILL.md` の `SoraConnectionCommand` に関する行が削除されている
- `CHANGES.md` の `## develop` に `[CHANGE]` エントリが追加されている
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- `cargo test` が全テスト通過する

## 解決方法

1. `src/connection.rs:573` の `pub enum SoraConnectionCommand` を `pub(crate) enum SoraConnectionCommand` に変更する
2. `src/lib.rs:17-20` の `pub use` リストから `SoraConnectionCommand` を削除する
3. `src/error.rs:102-105` の `CommandSendFailed` の `source: mpsc::error::SendError<SoraConnectionCommand>` を `reason: String` に変更する
4. `src/error.rs:290-292` の `Display` のパターンマッチとフォーマットの `source` を `reason` に変更する
5. `src/error.rs:381` の `CommandSendFailed` に対応する `source()` アームを削除する
6. `src/error.rs:6` の `use tokio::sync::{mpsc, oneshot};` から `mpsc` を削除し `use tokio::sync::oneshot;` に変更する
7. `src/error.rs:8` の `use crate::connection::SoraConnectionCommand;` を削除する
8. `src/connection.rs:524` の `.map_err(|source| Error::CommandSendFailed { source, command })` を `.map_err(|e| Error::CommandSendFailed { reason: e.to_string(), command })` に変更する
9. `skills/sora-rust-sdk/SKILL.md:66` の `SoraConnectionCommand` の行を削除する
10. `CHANGES.md` の `## develop` セクションに設計方針 6 の `[CHANGE]` エントリを追加する
11. `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo test` がすべて通ることを確認する

### 修正ファイル

- `src/connection.rs`
- `src/error.rs`
- `src/lib.rs`
- `skills/sora-rust-sdk/SKILL.md`
- `CHANGES.md`
