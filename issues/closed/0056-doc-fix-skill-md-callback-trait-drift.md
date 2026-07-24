# SKILL.md の callback trait 化への追随漏れを全面修正する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-24
- Model: Opus 4.7
- Branch: feature/fix-skill-md-callback-trait-drift
- Polished: {YYYY-MM-DD}

親 issue: [`0051-doc-prepare-readme-and-docs.md`](./0051-doc-prepare-readme-and-docs.md) の「High 相当（正式リリース blocker）」として認定され、`#0051` 側では「別 issue に切り出す」対象と明記された SKILL.md drift 全体を扱う。兄弟 issue: [`0055-fix-readme-sendonly-sample-compile.md`](./0055-fix-readme-sendonly-sample-compile.md)（README sendonly サンプル）。

## 目的

`skills/sora-rust-sdk/SKILL.md` を、`#0044`（callback trait 化、2026-07-08 closed）で確定した現行公開 API に完全追随させる。SKILL.md は Claude / Cursor / Codex 等の他エージェントが sora-rust-sdk を扱うときの一次リファレンスで、記述が実装と食い違うとエージェントが誤ったコードを生成し続ける。

## 優先度根拠

High。正式リリース 2026.1.0 の blocker。

- SKILL.md はエージェント向けの一次リファレンスで、間違いが波及した先で新たな誤コードを再生産する
- 現状はコンパイル不能な `SoraConnection::builder(...)` 4 引数呼び出しが 4 箇所、`SoraConnectionBuilder` に存在しないメソッドとして紹介されたトレイトメソッドが少なくとも 3 箇所、コールバック表全体（12 項目）が「Builder メソッド」の見出しで置かれている
- README sendonly は目視即発見できるが、SKILL.md は分量が多く drift の総量が把握しづらい。ここを塞がずに正式版を出すと、リリース後に「エージェントが古い API を提案する」問題が長く残る

## 現状

事実として確定した項目のみを載せる（`skills/sora-rust-sdk/SKILL.md` を実際に読み、`src/connection.rs` および `src/connection_event_handler.rs` の実装と突き合わせて確認）。行番号は 2026-07-24 時点。

### コンパイル不能な `SoraConnection::builder(...)` 4 引数呼び出し（4 箇所）

`src/connection.rs:658-664` は 5 引数（`context`, `signaling_urls`, `channel_id`, `role`, `event_handler`）だが、SKILL.md では以下がすべて 4 引数のまま:

- `SKILL.md:68` 型シグネチャ記述: `` `SoraConnection::builder(context, signaling_urls, channel_id, role) -> SoraConnectionBuilder` ``
- `SKILL.md:292-297` sendrecv 例: `Role::SendRecv` を 4 引数目で終了
- `SKILL.md:432-437` 複数クライアント例: `sora_sdk::Role::RecvOnly` を 4 引数目で終了
- `SKILL.md:469-471` 複数 URL レース例: `Role::SendRecv` を 4 引数目で終了

なお `SKILL.md:360` の DataChannel メッセージング例は `SoraConnection::builder(/* ... */)` と省略表記のため、引数数の drift 対象ではないが、直後の `.on_message(|label, data| ...)` が Builder メソッド誤扱いに該当する（次節）。

### Builder メソッド誤扱い（`SoraConnectionEventHandler` トレイトメソッドを Builder メソッドとして紹介）

`src/connection_event_handler.rs:15-` で `pub trait SoraConnectionEventHandler: Send` として定義されているメソッドを、SKILL.md では `SoraConnectionBuilder` のチェーン可能メソッドとして紹介している:

- `SKILL.md:70-87` コールバック表: 12 項目すべてが「メソッド」列で列挙され、直前の見出しは `#### コールバック設定`（`SKILL.md:70`）で、章の位置（`### 接続ビルダー` 直下）とあわせて Builder メソッド一覧と読める。実装ではすべて `SoraConnectionEventHandler` トレイトのメソッド（`src/connection_event_handler.rs:23,35,41,47,52,56,62,70,75,…`）で、`SoraConnectionBuilder` には存在しない
- `SKILL.md:263`（`.on_message(Fn(&str, &[u8]))` として登場）: 該当節を要確認。実装は `SoraConnectionEventHandler::on_message`（`src/connection_event_handler.rs:70`）
- `SKILL.md:300` sendrecv 例内 `.on_track(|transceiver| ...)`: Builder ではなくトレイトメソッド（`src/connection_event_handler.rs:47`）
- `SKILL.md:299` sendrecv 例内 `.on_notify(|text| ...)`: Builder ではなくトレイトメソッド（`src/connection_event_handler.rs:35`）
- `SKILL.md:361` DataChannel 例内 `.on_message(|label, data| ...)`: Builder ではなくトレイトメソッド

drift の起源: `#0044`（`issues/closed/0044-change-callback-to-trait.md`, 2026-07-08 closed）。当時 README（sendrecv / recvonly）と `src/lib.rs` の `//!` 例は追随済みだったが、SKILL.md への追随が漏れた。

## 設計方針

- `#0044` で確定した API 面（`SoraConnection::builder` 第 5 引数 `event_handler: impl SoraConnectionEventHandler + 'static`、コールバックは全てトレイトメソッド）を SKILL.md 側の正とする
- サンプルコードは README `recvonly`（`README.md:181-207`）と粒度・スタイルを揃える
  - `use` 節に `SoraConnectionEventHandler` を追加
  - 例ごとに空 impl の `struct MyEventHandler;` を用意し、必要なメソッドだけオーバーライドする形にする
  - Builder チェーン側からは `.on_notify` / `.on_track` / `.on_message` などのコールバック系呼び出しを削除する
- `SKILL.md:70-87` の「コールバック設定」表は、`SoraConnectionBuilder` の下位節から切り離し、「イベントハンドラ設定」として独立節にする（あるいは節タイトルを `#### イベントハンドラ (SoraConnectionEventHandler トレイト)` に改める）。表の見出しは「メソッド」→「トレイトメソッド」に改め、章の冒頭でトレイト実装型を Builder の第 5 引数として渡す旨を明示する
- `SoraConnectionBuilder` 側に残る本物のチェーンメソッド（`sender_audio_track` / `sender_video_track` / `audio` / `video` / `client_cert` / `ca_cert` / `turn_tls_ca_cert` / `proxy` 等）と、トレイトメソッド（`on_*`）の境界を、節見出しではっきり分ける
- 動作変更ゼロ。あくまで表記の正確化のみ
- 「軽く直す」ではなく、`grep -n "SoraConnection::builder\|on_notify\|on_track\|on_message\|on_signaling_message\|on_push\|on_switched\|on_websocket_close\|on_data_channel\|on_remove_track\|on_data_channel_open\|on_data_channel_message\|on_data_channel_close" skills/sora-rust-sdk/SKILL.md` で全出現を洗ってから、残らず現行 API に揃える

## 完了条件

- `skills/sora-rust-sdk/SKILL.md` 全体で以下が達成されている
  - `SoraConnection::builder(...)` の全出現が `src/connection.rs:658-664` の 5 引数と一致（第 5 引数 `event_handler` を必ず伴う）
  - 各サンプルは `SoraConnectionEventHandler` を `use` に含み、`struct MyEventHandler;` + 空 impl を最低限持つ
  - `SoraConnectionBuilder` のチェーンとして書かれていた `on_*` 系呼び出しはすべてトレイトメソッド側に移動、あるいはトレイトメソッドである旨が明記されている
  - `SKILL.md:70-87` の 12 コールバック表が「`SoraConnectionEventHandler` トレイトのメソッド」として位置付けられている
- 修正後、`grep` で `SoraConnection::builder` の全出現が 5 引数呼び出しか省略記法（`(/* ... */)` 等）のいずれかであることを確認する（4 引数呼び出しがゼロ）
- `grep` で `\.on_notify\|\.on_track\|\.on_message\|\.on_push\|\.on_signaling_message\|\.on_switched\|\.on_websocket_close\|\.on_data_channel` の Builder チェーン誤用が SKILL.md 内にゼロ、または全てトレイト実装ブロック内の記述（`impl SoraConnectionEventHandler for ...` の中）に限定されている
- `#0051` の完了条件を満たすため、本 issue の番号は `#0051` の子 issue 欄に明示的に反映されている（`issues/0051-doc-prepare-readme-and-docs.md:152`）
- `CHANGES.md` への追記は不要（`shiguredo-changelog` 規約で `.md` 変更は変更履歴に反映しない）

## 解決方法

1. `grep -nE "SoraConnection::builder|SoraConnectionBuilder::on_|\.on_[a-z_]+\(|SoraConnectionEventHandler|SoraConnectionBuilder" skills/sora-rust-sdk/SKILL.md` で drift の全出現を洗い出し、修正対象を確定した
2. `SKILL.md:68` の型シグネチャ記述を 5 引数（`context, signaling_urls, channel_id, role, event_handler`）に修正し、`event_handler` の型（`impl SoraConnectionEventHandler + 'static`）を明示した
3. `SKILL.md:70-87` の 12 コールバック表を「`SoraConnectionEventHandler` トレイト」の節に置き換えた。節見出しを `#### イベントハンドラ (SoraConnectionEventHandler トレイト)` に改め、表の列を「トレイトメソッド」「シグネチャ」「説明」に変更し、実際のトレイトメソッド署名（`fn on_*(&mut self, ...)` 形式）に合わせた。`Send`（`Sync` 不要）である旨と、実装型のインスタンスを Builder 第 5 引数に渡す旨をイントロに追加した
4. `SKILL.md:263` の `SoraConnectionBuilder::on_message` 誤扱いを `SoraConnectionEventHandler::on_message` に修正した
5. sendrecv 例（`SKILL.md:280-`）、DataChannel メッセージング例（`SKILL.md:359-`）、複数クライアント例（`SKILL.md:424-`）、複数 URL レース例（`SKILL.md:469-`）の 4 例をすべて `struct MyEventHandler;` + `impl SoraConnectionEventHandler for MyEventHandler { ... }` パターンに書き直し、`SoraConnection::builder(...)` 呼び出しを 5 引数化した
6. 修正後に `grep -nE "^\s*\.on_" skills/sora-rust-sdk/SKILL.md` で Builder チェーンとしての `.on_*` 誤用が 0 件になっていることを確認した
7. `cargo check --workspace` を実行し、SKILL.md 変更が周辺コードのコンパイルに影響を与えていないことを確認した（22.66s、エラーなし）
8. `CHANGES.md` への追記は不要と判断した（`shiguredo-changelog` 規約で `.md` 変更は変更履歴に反映しない）
9. ユーザーの明示的許可のもと、`feature/fix-skill-md-callback-trait-drift` ブランチは切らず `develop` に直接コミットした

## 対象ファイル

- `skills/sora-rust-sdk/SKILL.md`

## 対象外

- `README.md` sendonly サンプル修正は #0055 で扱う
- `skills/sora-rust-sdk/SKILL.md` の callback trait 化以外の drift（例: 新 API 追加分の網羅性、非同期化の記述、feature フラグ表の最新化）は本 issue のスコープ外。必要なら別 issue で扱う
- SKILL.md をエージェント配布用に再インストール／同期する運用（`gh skill install` 等）は本 issue では扱わない。修正後の同期はメンテナーの通常運用に任せる
