# pre-push フックが e2e 実接続テストで push をブロックする問題を直す

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-prek-pre-push-e2e-block
- Polished: {YYYY-MM-DD}

## 目的

`prek.toml` の pre-push フックが e2e-tests の実 Sora 接続テストを含むため、テスト環境を用意していない開発者や canary リリースの push をブロックする問題を解消する。

## 現状

`prek.toml` の pre-push フックは `cargo test --workspace` を実行するが、`e2e-tests` のテストは `TEST_SIGNALING_URLS` 未設定時に skip ではなく panic (`e2e-tests/src/lib.rs` の `signaling_urls()` がエラーを返し、各テストの `expect` で失敗) する。

`e2e-tests/.env` は gitignore されており、CI の secrets 相当の設定がないローカル開発者は実 Sora のテスト環境を用意しない限り push できない。また `canary.py` の `git push` / tag push でも同じフックが走るため、リリース操作自体がローカル環境の E2E 接続可否に依存する。

## 設計方針

- pre-push のテスト対象から e2e-tests を除外する (例: `cargo test --workspace --exclude e2e-tests` か、sora_sdk / sumomo / pbt のみを対象にする)
- または、`TEST_SIGNALING_URLS` 未設定時に e2e テストが skip されるようにする (テスト側の変更は別 issue の範囲とし、本 issue はフック側の修正に絞る)
- canary.py の push がローカル E2E 環境に依存しないことを確認する

## 完了条件

- `TEST_SIGNALING_URLS` 未設定の環境で pre-push フックが失敗しない
- canary.py の push / tag push がローカル E2E 環境に依存しない
- CI でのテストカバレッジは変わらない (CI の `cargo test --workspace` は従来どおり e2e を含む)
- `cargo test --workspace` が成功する

## 変更対象

- `prek.toml`
