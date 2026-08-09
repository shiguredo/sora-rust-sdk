# e2e テストの run task エラーを失敗時に表示する

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-e2e-run-task-error-visibility
- Polished: {YYYY-MM-DD}

## 目的

e2e テストで接続が早期に失敗した場合、run task が保持する真のエラーをテスト失敗メッセージに表示し、原因の切り分けを可能にする。

## 現状

`e2e-tests/src/test_connection.rs` の `SoraTestConnection::connect` は `tokio::spawn(async move { connection.run().await })` で run task を起動するだけである。接続が早期に失敗した場合:

- `wait_for_event` は「タイムアウトしました」としか報告せず、run task の `Result` を参照しない
- `disconnect_and_wait` は先に `disconnect().await?` を実行するため、run task が先に死んでいると command 送信が `CommandSendFailed` で先に失敗し、真のエラーに到達できない

この結果、シグナリングエラー・認証失敗・接続拒否などの真因が隠れ、SDK バグの切り分けが困難になる。

## 設計方針

- run task の `JoinHandle<Result<()>>` を保持し、各待機 API のタイムアウト時・失敗時に run task の結果を参照して真のエラーを表示する
- `disconnect_and_wait` が `CommandSendFailed` を返した場合も、run task の結果を確認して真のエラーを優先表示する
- テスト失敗メッセージは日本語で、機密情報 (シグナリング URL・credential) を含めない

## 完了条件

- 接続失敗時に run task のエラー内容がテスト失敗メッセージに表示される
- タイムアウトと接続エラーが区別できる
- 既存テストの正常系の挙動が変わらない
- `cargo test --workspace` が成功する

## 変更対象

- `e2e-tests/src/test_connection.rs`
- `e2e-tests/src/lib.rs` (必要に応じて)
