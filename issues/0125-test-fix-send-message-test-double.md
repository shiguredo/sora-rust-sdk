# send_message のテストダブルを排除して実ロジックを検証する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-send-message-test-double
- Polished: 2026-08-15

## 目的

`SoraConnectionHandle::send_message` のテストがフェイク実装でプロダクションの検証ロジックを複製しており、実コードを検証できていない問題を解消する。

## 現状

`src/connection.rs` のテストモジュールの `spawn_message_server` は、プロダクションの `SoraConnectionCommand::SendMessage` ハンドリングのラベル検証ロジックをテスト側で再実装している。テストは「フェイクが返す結果」を検証するだけで、実コードが壊れても全テストが通り続ける。

さらに、`send_message_accepts_registered_label` が検証する `Ok(())` の成功経路は、プロダクション実装では実際に開設済みの DataChannel (`data_channels`) が無いと `Error::DataChannelMissing` になる (ラベルが `data_channel_configs` に未登録なら `Error::InvalidDataChannelLabel` になる)。フェイクは実チャネルの有無を無視して `Ok(())` を返すため、プロダクションには存在しない経路を検証している。

これは AGENTS.md の「モックやスタブは絶対に利用しない」規約の趣旨 (実装をすり替えて検証をごまかさない) にも反する。

なお、テストダブル方式は closed の 0065 (send_message ラベル検証の導入) で採用され、0113 で踏襲された。本 issue はその方式を是正する。

## 設計方針

`SendMessage` のラベル検証と送信処理は `SoraConnection::run()` のコマンドループ内にインラインで書かれており、単体テストから直接呼び出せない。そこで:

- `SendMessage` コマンドハンドラの処理 (ラベル検証と `send_data_channel_message` 呼び出し) を、挙動を変えずに `SoraConnection` のテスト可能なメソッドへ切り出し、`run()` のコマンドループから呼び出す
- テストは既存の `build_test_connection` と `register_compressed_data_channel` ヘルパーで実 `SoraConnection` と実 DataChannel を構築し、切り出したメソッドを直接呼んでラベル検証と `DataChannelMissing` エラーパスを検証する。`DataChannelMissing` は `data_channel_configs` にのみラベルを登録し実チャネルを登録しない状態で発生させる。`spawn_message_server` は削除する
- `Ok(())` の成功経路は Open 状態の実チャネルが必要なため単体テストでは用意せず、e2e テスト (`e2e-tests/tests/messaging.rs` の `test_messaging_sendrecv` が `send_message` を呼ぶ) が担保する
- `DataChannelSendFailed` のエラーパスは本対応の対象外とする
- `spawn_get_stats_server` はプロダクションロジックを再実装しておらず対象外

## 完了条件

- `spawn_message_server` によるフェイクがテストに残っていない
- 実ロジックのラベル検証がテストされる (未登録ラベル・`#` プレフィックス・SDK 内部用ラベル)
- `DataChannelMissing` エラーパスがテストされる
- 実コードの挙動を変えずにテストが通る
- `cargo test --workspace` が成功する
- `cargo fmt --check --all` と `cargo clippy --workspace -- -D warnings` が通る
- テストの assertion message は日本語にする

## 変更対象

- `src/connection.rs` (テストモジュールと、SendMessage コマンドハンドラのテスト可能なメソッドへの切り出し)
