# send_message のテストダブルを排除して実ロジックを検証する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-send-message-test-double
- Polished: {YYYY-MM-DD}

## 目的

`SoraConnectionHandle::send_message` のテストがフェイク実装でプロダクションの検証ロジックを複製しており、実コードを検証できていない問題を解消する。

## 現状

`src/connection.rs` のテストモジュールの `spawn_message_server` は、プロダクションの `SoraConnectionCommand::SendMessage` ハンドリングのラベル検証ロジックをテスト側で再実装している。テストは「フェイクが返す結果」を検証するだけで、実コードが壊れても全テストが通り続ける。

さらに、`send_message_accepts_registered_label` が検証する `Ok(())` の成功経路は、プロダクション実装では `data_channel_configs` にチャネルが未登録なら `Error::DataChannelMissing` になるため、プロダクションに存在しない動作を検証している。

これは AGENTS.md の「モックやスタブは絶対に利用しない」規約の趣旨 (実装をすり替えて検証をごまかさない) にも反する。

## 設計方針

- フェイクサーバーを排除し、プロダクションの検証ロジック (ラベル検証・`data_channel_configs` 参照・`DataChannelMissing` エラーパス) を直接テストする
- 実ロジックをテスト可能な形で切り出してテストするか、実 `SoraConnection` のメインループを使うテストに置き換える
- `DataChannelMissing` / `DataChannelSendFailed` のエラーパスを検証する

## 完了条件

- `spawn_message_server` によるフェイクがテストに残っていない
- 実ロジックのラベル検証がテストされる (未登録ラベル・`#` プレフィックス・予約ラベル)
- `DataChannelMissing` エラーパスがテストされる
- 実コードの挙動を変えずにテストが通る
- `cargo test --workspace` が成功する
- テストの assertion message は日本語にする

## 変更対象

- `src/connection.rs` (テストモジュール)
