# e2e テストの run task エラーを失敗時に表示する

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-e2e-run-task-error-visibility
- Polished: 2026-08-15

## 目的

e2e テストで接続が早期に失敗した場合、run task が保持する真のエラーをテスト失敗メッセージに表示し、原因の切り分けを可能にする。

## 現状

`e2e-tests/src/test_connection.rs` の `SoraTestConnectionBuilder::connect` は `tokio::spawn(async move { connection.run().await })` で run task を起動するだけである。接続が早期に失敗した場合:

- `wait_for_event` は「タイムアウトしました」としか報告せず、run task の `Result` を参照しない
- `disconnect_and_wait` は先に `disconnect().await?` を実行するため、run task が先に死んでいると command 送信が `CommandSendFailed` で先に失敗し、真のエラーに到達できない

この結果、シグナリング URL への接続失敗 (接続拒否・TLS 失敗・接続タイムアウト) などの真因が隠れ、SDK バグの切り分けが困難になる。

なお、run task が `Err` を返すのは主に接続確立前の失敗であり、認証失敗やサーバー側の WebSocket close では `run()` が `Ok(())` を返すため、本対応の対象外である。

## 設計方針

- 保持済みの run task の `JoinHandle<Result<()>>` の結果を、タイムアウト時・失敗時に参照して真のエラーを表示する
  - run task が終了済みで `Err` を保持している場合のみエラーを表示し、終了していない場合や `Ok(())` の場合は従来どおりタイムアウトとして扱う
  - run task の結果は読み取り 1 回きり (2 回目の読み出しで panic する) であるため、初回読み出し時は run task の終了を待機してから結果を保持し、2 回目以降は保持済みの結果を返す。`wait_for_run_finished` もこの保持済みの結果を返すようにする
- `disconnect_and_wait` が `CommandSendFailed` を返した場合も、run task の結果を確認して真のエラーを優先表示する
- テスト失敗メッセージは日本語で、機密情報 (シグナリング URL・接続先 host・credential) を含めない。run task のエラーをそのまま表示すると `Error::AllSignalingUrlsFailed` の `Display` が URL 原値を含むため、どの待機 API から表示する場合もシグナリング URL などの機密情報を除去してからメッセージに含める
- 対象は `&mut self` を取る待機 API (`wait_for_event` 系・`disconnect_and_wait`・`wait_for_run_finished`) とし、`&self` を取る `wait_stats` 系は対象外とする

## 完了条件

- 接続失敗時に run task のエラー内容がテスト失敗メッセージに表示される
- タイムアウトと接続エラーが区別できる
- 既存テストの正常系の挙動が変わらない
- 接続失敗後に `wait_for_connect` → `disconnect_and_wait` を続けて呼ぶ既存テストの失敗パスで panic しない
- `cargo test --workspace` が成功する

## 変更対象

- `e2e-tests/src/test_connection.rs`
