# sumomo の --version が出力されない問題を直す

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-version-output
- Polished: {YYYY-MM-DD}

## 目的

`sumomo --version` を実行したときにバージョン文字列が出力されるようにする。

## 現状

`examples/sumomo/src/args.rs` の `parse_args` は `--version` 指定時に `rtc_log_info!` でバージョン文字列を出力する。しかし `rtc_log_info!` は `main` (`examples/sumomo/src/main.rs`) の `log::log_to_debug(Severity::Info)` によるログ初期化より前に実行されるため、libwebrtc のログ既定値 (出力なし) のまま握りつぶされる。ユーザーは `sumomo --version` が exit 0 で成功したのに何も表示されない。

## 設計方針

- `--version` の出力をログ初期化の後に移動する
- または、`--version` の出力を `rtc_log_info!` ではなく `println!` などの直接出力に変更する
- CLI のヘルプ表示 (`--help`) との挙動の整合を確認する

## 完了条件

- `sumomo --version` でバージョン文字列が出力される
- 通常のログ出力の挙動が変わらない
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `examples/sumomo/src/args.rs`
- `examples/sumomo/src/main.rs`
- `CHANGES.md`
