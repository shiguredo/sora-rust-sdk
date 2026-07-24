# README の sendonly サンプルをコンパイル可能な状態に戻す

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-24
- Model: Opus 4.7
- Branch: feature/fix-readme-sendonly-sample-compile
- Polished: {YYYY-MM-DD}

親 issue: [`0051-doc-prepare-readme-and-docs.md`](./0051-doc-prepare-readme-and-docs.md) の「High 相当（正式リリース blocker）」として認定された sendonly サンプル破綻部分を切り出したもの。#0051 の完了条件により、本 issue の番号は #0051 のマージ用 PR 本文冒頭で参照される。

## 目的

`README.md` の sendonly サンプルを、公開 API の実態（`src/connection.rs:658-664` の 5 引数 `builder`）に合わせてそのままコンパイルできる状態に戻す。crates.io から `sora_sdk 2026.1.0` を導入する利用者が最初の 10 分でつまずかないようにする。

## 優先度根拠

High。正式リリース 2026.1.0 の blocker。

- 破綻箇所は README「sendonly で接続する」節（最短の送信例）で、SDK 導入直後にほぼ確実に踏むサンプルコード
- サンプルがコンパイル不能な状態で crates.io に到着すると、利用者は「そもそも SDK が壊れている」という一次判断に至る
- 対照的に `sendrecv`（`README.md:122-128`）と `recvonly`（`README.md:195-201`）はすでに 5 引数で正しく書かれており、sendonly だけが `#0044`（callback trait 化, 2026-07-08 closed）への追随漏れで残っている

## 現状

事実として確認済み（`README.md:147-173` を実際に読み、`src/connection.rs:655-672` の実装と突き合わせて確定）:

- `use` 節（`README.md:147`）が `use sora_sdk::{Role, SoraConnection, SoraConnectionContext};` のみで、`SoraConnectionEventHandler` が含まれていない
- `struct MyEventHandler;` および `impl SoraConnectionEventHandler for MyEventHandler {}` の定義が節内に存在しない
- `SoraConnection::builder(...)` 呼び出し（`README.md:160-165`）が以下の 4 引数のみで、第 5 引数 `event_handler` が抜けている

  ```
  SoraConnection::builder(
      context,
      vec!["wss://sora.example.com/signaling".to_string()],
      "your-channel-id".to_string(),
      Role::SendOnly,
  )
  ```

- 一方で `src/connection.rs:658-664` の実装は 5 引数を要求する

  ```
  pub fn builder(
      context: Arc<SoraConnectionContext>,
      signaling_urls: Vec<String>,
      channel_id: String,
      role: Role,
      event_handler: impl SoraConnectionEventHandler + 'static,
  ) -> SoraConnectionBuilder
  ```

- したがって `README.md` sendonly サンプルをそのまま貼り付けるとコンパイルエラーになる
- `sendrecv`（`README.md:122-128`）と `recvonly`（`README.md:195-201`）はすでに 5 引数で書かれており、`recvonly` は `MyEventHandler` の struct / impl も同節内に含んでいる（`README.md:181-201`）

drift の起源は `#0044`（`issues/closed/0044-change-callback-to-trait.md`、2026-07-08 closed）で導入された `SoraConnection::builder` 第 5 引数化。sendrecv / recvonly は同時に追随済みで、sendonly だけが漏れていた。

## 設計方針

- `recvonly` サンプル（`README.md:181-207`）と同一のスタイルに揃える
  - `use` 節に `SoraConnectionEventHandler` を追加する
  - `#[tokio::main] async fn main` の直前に `struct MyEventHandler;` と `impl SoraConnectionEventHandler for MyEventHandler {}`（空実装）を置く
  - `SoraConnection::builder(...)` に第 5 引数として `MyEventHandler` を渡す
- 空実装で意味が伝わりにくくならないよう、`recvonly` 側と同じく `on_notify` の最小オーバーライド例を入れるかどうかは、実装時に `recvonly` サンプルと粒度を揃える方向で判断する（現状 `recvonly` は `on_notify` を 1 メソッド上書きしている）
- 節内の文言・見出し・コメント（`// AudioTrack を作成する` など）は変更しない。あくまでコンパイル可能化のための最小修正に留める

## 完了条件

- `README.md` の sendonly サンプル節（現行 `L142-174`）が、`src/connection.rs:658-664` の 5 引数 `builder` に整合する形に修正されている
- サンプル節をそのままローカルの `main.rs` に貼り付けて `cargo check` が通ることを、手元で 1 度検証する
- `#0051` の完了条件を満たすため、本 issue の番号は `#0051` の子 issue 欄に明示的に反映されている（`issues/0051-doc-prepare-readme-and-docs.md:151`）
- `CHANGES.md` への追記は不要（`shiguredo-changelog` 規約で `.md` 変更は変更履歴に反映しない）

## 解決方法

1. `README.md:142-174` の sendonly 節を、`recvonly` 節と同じスタイル（`use` 節に `SoraConnectionEventHandler` を追加、`struct MyEventHandler;` + `impl SoraConnectionEventHandler for MyEventHandler { fn on_notify(&mut self, text: &str) { println!("notify: {text}"); } }`、`SoraConnection::builder(...)` 呼び出しに第 5 引数 `MyEventHandler` を追加）に修正した
2. 修正後のサンプルを scratch プロジェクトにそのまま貼り付け、`sora_sdk` を path 依存で参照した状態で `cargo check` がクリーンに通ることを確認した（24.94s、警告・エラーなし）
3. `CHANGES.md` への追記は不要と判断した。`shiguredo-changelog` 規約に「`.rst` / `.md` ファイルの変更は変更履歴に反映しないこと」と明記されており、本 issue の変更は `README.md` のみのため対象外
4. ユーザーの明示的許可のもと、`feature/fix-readme-sendonly-sample-compile` ブランチは切らず `develop` に直接コミットした

## 対象ファイル

- `README.md`（sendonly 節のみ）

## 対象外

- `skills/sora-rust-sdk/SKILL.md` の drift 全面追随は #0056 で扱う
- `README.md` の他節（前提条件、対応プラットフォーム、構成図、MP4 無変換送信の音声制約など）は #0051 の Medium 側で継続対応する
- `src/lib.rs` の `//!` に埋め込まれた sendrecv 最小例は #0051 の対象ファイル一覧に含まれるため、本 issue では触らない
