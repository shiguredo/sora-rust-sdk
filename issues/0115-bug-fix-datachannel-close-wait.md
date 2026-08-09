# 切断時の DataChannel クローズ待機処理を正す

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-datachannel-close-wait
- Polished: {YYYY-MM-DD}

## 目的

`SoraConnection::run` 終了時の DataChannel クローズ待機処理の欠陥を修正し、ユーザーへの close 通知と切断操作の整合性を保つ。

## 現状

`SoraConnection::run` のシャットダウン待機ループ (`src/connection.rs`) に以下の欠陥がある。

- 待機ループが `disconnect_wait_timeout` を超えた場合、閉じなかった残りチャネルの `on_data_channel_close` が呼ばれずに run() が終了する。`Disconnect` コマンド経路は break 前に全チャネルへ close 通知するのに非対称
- 待機ループは `DataChannelStateChange` イベントを受信しただけで `opened_datachannels` から remove する。通常の状態遷移ハンドラ (`handle_datachannel_state`) は `is_datachannel_closed` を確認してから remove するため、Closing 遷移の時点でまだ閉じていないチャネルが「閉じた」と通知される
- 待機ループは `event_rx` のみ監視し `command_rx` を処理しないため、WebSocket 切断後の待機中に `disconnect()` を呼ぶと ack が返らず `Error::CommandResponseMissing` になる
- `event_rx.recv()` が `None` (送信側全滅) を返した場合も「切断待機がタイムアウトしました」と誤った警告ログを出す

## 設計方針

- タイムアウト時に残りチャネルへの `on_data_channel_close` 通知を行う
- 待機ループの remove 判定を `handle_datachannel_state` と同じくチャネル状態の確認付きにする
- 待機中も `command_rx` を処理する (または `Disconnect` コマンドが既に処理済みであることを確認する)
- `event_rx` クローズ時とタイムアウト時でログを区別する

## 完了条件

- タイムアウト時に残りチャネルの close 通知が行われる
- Closing 遷移で誤って「閉じた」と通知されない
- 待機中に `disconnect()` が呼ばれても適切に処理される
- タイムアウトとチャネルクローズのログが区別される
- モックやスタブを使わないテストがある
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `CHANGES.md`
