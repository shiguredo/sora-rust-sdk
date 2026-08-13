# プロダクションコードの日本語ログと expect メッセージを英語に統一する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-translate-japanese-log-messages
- Polished: 2026-08-14

## 目的

AGENTS.md の「ログメッセージは全て英語にすること」に違反するプロダクションコードの日本語ログと、`expect` メッセージの言語混在を是正する。

## 現状

`src/connection.rs` に日本語の `rtc_log_*` が 7 箇所残っている。うち 6 箇所を本 issue で英語化する（残り 1 箇所の「切断待機がタイムアウトしました」は open issue 0115 が書き直す）。

- `rtc_log_info!` の「接続先:」「リダイレクト先:」「DataChannel '{}' からメッセージを受信:」「HTTP Proxy 経由で接続します:」「接続試行:」「接続成功:」
- `rtc_log_warning!` の「切断待機がタイムアウトしました」（0115 が扱うため本 issue では対象外）

また、プロダクションコードの `expect` メッセージの言語が混在している。

- 日本語: `src/connection.rs` の「event_handler は new() で設定されている」「response_tx は必ず存在する」「notification でない場合は id が存在する」「BUG: proxy 接続後は plain stream のはずです」、`src/rpc.rs` の「result_seen なので必ず存在する」「error_seen なので必ず存在する」
- 英語: 「video_sender must exist after is_none check」等

`panic!` メッセージはプロダクションコードに日本語が存在しないため、本 issue では対象外とする。

## 設計方針

- `src/connection.rs` の日本語ログ 6 箇所を英語に書き換える
- プロダクションコードの `expect` メッセージを英語に統一する（`src/connection.rs` と `src/rpc.rs`）
- 翻訳スタイルは既存の英語ログ・`expect` メッセージ（「Connection failed: {}: {}」等）の文体に合わせる
- テストコード内のメッセージは対象外（テストのログメッセージは日本語が規約）

## 完了条件

- `src/` 配下のプロダクションコードに日本語ログが残っていない（`0115` で書き直される「切断待機がタイムアウトしました」を除く）。`rtc_log_*` のフォーマット文字列に日本語文字が含まれないことを `grep` で確認する
- `src/` 配下のプロダクションコードの `expect` メッセージが英語で統一されている（`src/connection.rs` と `src/rpc.rs`。`examples/sumomo/` などの examples は対象外とする）
- テストコードのメッセージは変更しない
- `cargo test --workspace` が成功する
- `CHANGES.md` の `## develop` に `[FIX]` エントリを追加する

## 変更対象

- `src/connection.rs`
- `src/rpc.rs`
- `CHANGES.md`
