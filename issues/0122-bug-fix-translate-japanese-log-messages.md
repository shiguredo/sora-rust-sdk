# プロダクションコードの日本語ログを英語に統一する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-translate-japanese-log-messages
- Polished: {YYYY-MM-DD}

## 目的

AGENTS.md の「ログメッセージは全て英語にすること」に違反するプロダクションコードの日本語ログと、expect / panic メッセージの言語混在を是正する。

## 現状

`src/connection.rs` に日本語の `rtc_log_*` が 8 箇所残っている (ログメッセージの英語統一は過去に対応済みだが、このファイルだけ修正漏れ)。

- `rtc_log_info!` の「接続先:」「リダイレクト先:」「DataChannel '{}' にバイナリメッセージを送信:」「DataChannel '{}' からメッセージを受信:」「HTTP Proxy 経由で接続します:」「接続試行:」「接続成功:」
- `rtc_log_warning!` の「切断待機がタイムアウトしました」

また、プロダクションコードの `expect` メッセージの言語が混在している (日本語: `connection.rs` の「event_handler は new() で設定されている」「response_tx は必ず存在する」等、英語: 「video_sender must exist after is_none check」等)。panic メッセージはコンソールに出力されるため、英語に統一する。

## 設計方針

- `src/connection.rs` の日本語ログ 8 箇所を英語に書き換える
- プロダクションコードの `expect` メッセージを英語に統一する
- テストコード内のメッセージは対象外 (テストのログメッセージは日本語が規約)

## 完了条件

- `src/` 配下のプロダクションコードに日本語ログが残っていない
- プロダクションコードの `expect` メッセージが英語で統一されている
- テストコードのメッセージは変更しない
- `cargo test --workspace` が成功する

## 変更対象

- `src/connection.rs`
