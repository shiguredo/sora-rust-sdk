# sumomo の不完全な client 認証設定を拒否する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-sumomo-client-auth
- Polished: 2026-08-06

## 目的

client certificate と private key の片側だけが指定された場合に起動を拒否し、利用者の意図に反する認証なし接続を防ぐ。

## 優先度根拠

High。認証を設定したつもりの利用者が警告なしで認証なし接続へ移行し、接続先の設定次第では意図しない接続が成立する。

## 現状

sumomo の argument parser は certificate と key を独立して受理する。
argument validation は pair 条件を検査せず、builder 設定は両方が存在する場合だけ適用される。
SDK 本体の `client_cert` は certificate と key をペアで受け取るため、片側指定は SDK 側の検証に到達せず、警告なしで認証なし接続になる。

## 設計方針

- certificate と key の両方指定または両方未指定だけを許可する
- 片側指定は `validate_args` で接続開始前にエラーにする
  - SDK 本体の検証は sumomo 経由では発火しないため
- エラーは既存の `validate_args` と同じ英語 + オプション名の形式にする
  - 例: `--client-cert and --client-key must be specified together`
  - SDK 本体の同規則のエラーは日本語表示かつオプション名形式ではないため再利用しない
  - `parse_args` の既存の cert/key 読み込みエラー（日本語）は本 issue の変更対象外
- error とログに certificate や key の内容を含めない
  - private key の実値が診断エラーや CI ログへ流出するのを防ぐ

## 変更対象

- `examples/sumomo/src/args.rs`
  - `validate_args` への pair 条件の検査追加
  - `--client-cert` / `--client-key` の help 文言へ両方指定が必要な旨を追記
- `examples/sumomo/src/tests.rs`
  - argument validation のテスト追加
- `CHANGES.md`
- SDK 本体 (`src/`) と `examples/sumomo/src/main.rs` は変更しない

## 完了条件

- certificate だけ、key だけの指定が接続開始前に失敗する
  - `--video-codec-list` モードは main が `validate_args` より先に return するため構造上対象外（`validate_args` 側の分岐は不要）
  - `--list-devices` モードは Args の `client_cert` / `client_key` が None 固定のため検査を自然に通過する
- 両方指定と両方未指定は従来どおり動作する
- error とログに certificate や key の内容が含まれない
  - `validate_args` はログを出力しないため、ログ側はテスト対象外
- argument validation のテストで、片側指定の拒否（エラーが英語 + オプション名形式であることも検証する）・両方指定の受理・両方未指定の受理・エラーに certificate や key の内容が含まれないことを検証する
- `--client-cert` / `--client-key` の help 文言に両方指定が必要な旨が記載されている
- 新規に追加する error は英語、新規に追加するテストのコメントと assertion message は日本語にする（既存の記述の書き換えは含まない）
- `CHANGES.md` の develop セクションへ `[FIX]` と担当者 `@voluntas` を追記する
