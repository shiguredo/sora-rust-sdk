# sumomo の --version を --help と同様に stdout へクリーン出力する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-version-output
- Polished: 2026-08-14

## 目的

`sumomo --version` の出力を `--help` と同じ形式（stdout へのログプレフィックスなしの直接出力）に統一し、バージョン文字列がパイプ処理やツール連携で利用できるようにする。

## 現状

`examples/sumomo/src/args.rs` の `parse_args` は `--version` 指定時に `rtc_log_info!` でバージョン文字列を出力して `exit(0)` する。`rtc_log_info!` は libwebrtc のログ経路（`shiguredo_webrtc::rtc_base::logging`）で出力されるため、Linux では stderr に `(sumomo::args.rs:150): sumomo 0.0.0` のようなログプレフィックス付きで出る。

一方、`--help` は `parse_args` 内の `args.finish()` で得たヘルプ文字列を `print!` で **stdout** にログプレフィックスなしで出力して `exit(0)` する（`examples/sumomo/src/args.rs` の `noargs::HELP_FLAG.take_help` の後）。`--video-codec-list` も同様に直接出力する。

また、libwebrtc の stderr 出力はプラットフォーム・ビルド種別に依存し、macOS の release ビルドでは `WEBRTC_MAC && NDEBUG` 分岐で stderr へのログ出力が無効化され得る。その環境では `--version` が何も表示されないことになる。このため「ログ経由で出る」「ログ経由だと出力されない」のどちらも環境依存の挙動となる。

## 設計方針

- `--version` の出力を `rtc_log_info!` から `println!`（stdout への直接出力・末尾改行付き）に変更する。`print!` は `std::process::exit(0)` 時に stdout バッファがフラッシュされないため使わない
- `--help`（stdout への `print!` 直接出力）と同じ形式に揃え、ログプレフィックス・タイムスタンプ・スレッド名を含めない
- `--version` の処理位置は現状のまま `parse_args` 内で `exit(0)` とする（`--version` と他の早終了オプションの同時指定時は `--version` を最優先する。現状どおり）
- 出力するバージョン文字列は `env!("CARGO_PKG_NAME")` / `env!("CARGO_PKG_VERSION")` のままとする（sumomo は `examples/sumomo/Cargo.toml` で `version = "0.0.0"`）
- `--list-devices` も同様にログ経由で出力しているが、本 issue では `--version` のみを対象とする（`--list-devices` は対象外）

## 完了条件

- `sumomo --version` が stdout に `sumomo 0.0.0` と末尾改行付きで出力し（ログプレフィックスなし・タイムスタンプ/スレッド名なし）、exit 0 する
- stderr にはログプレフィックス付きの出力をしない
- `--help` の出力形式と整合している
- 通常のログ出力（接続時等）の挙動が変わらない
- 回帰テストを追加する（`CARGO_BIN_EXE_sumomo` を child process として起動し、`--version` が stdout に `sumomo 0.0.0` を出力して stderr には出力しないこと、exit 0 であることを検証する）
- `cargo test --workspace` と `cargo clippy --workspace -- -D warnings` が成功する（`args.rs` の `rtc_log_info` import が未使用になり clippy エラーになるため、import の削除も行う）
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `examples/sumomo/src/args.rs`（`--version` 出力の `println!` 化と、未使用になる `rtc_log_info` import の削除）
- `examples/sumomo/tests/`（`--version` の回帰テストを追加）
- `CHANGES.md`（`[FIX]` エントリを追加）
