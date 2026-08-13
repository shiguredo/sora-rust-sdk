# get_stats で run ループがブロックされないようにする

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-get-stats-run-loop-blocking
- Polished: 2026-08-12

## 目的

`PeerConnection::get_stats` のコールバック待機中に `SoraConnection::run` のメインループがブロックされないようにする。あわせて、コールバックが発火しない異常系で待機が永久に続かないようにする。

## 現状

`SoraConnection::get_stats` (`src/connection.rs` の private な `async fn get_stats`) は `pc.get_stats` のコールバックを oneshot チャネルで待つ。この待機は以下の 3 経路から呼ばれ、いずれも run ループ内で `await` されるため、コールバックが発火するまでの間、run ループ全体 (WebSocket 読み取り・DataChannel イベント・タイマー・disconnect を含む全コマンドの処理) が停止する。

- `SoraConnectionCommand::GetStats` コマンド処理
- DataChannel 経由の `ping` (stats 付き) への応答 (`handle_datachannel_message` 内)
- DataChannel 経由の `req-stats` への応答 (`handle_datachannel_message` 内)

また、コールバック自体が呼ばれない場合 (libwebrtc 内部の挙動に依存する) は oneshot の Sender がドロップされず、`rx.await` が永久にブロックする。なお、null report 付きでコールバックが呼ばれた場合も Sender はドロップされ、`rx.await` は即復帰する。

一方、WebSocket 経由の `ping` (stats 付き) / `req-stats` 応答は `request_stats_pong` / `request_stats_response` (`src/connection.rs`) がコールバック内で直接 `event_tx` に送信するため待機を持たず、run ループをブロックしない。public API の `SoraConnectionHandle::get_stats` が通る `send_command` の `rx.await` もタイムアウトを持たないが、修正後は handle 側の待機にタイムアウトを設けるため解決する。

## 設計方針

- 3 経路を WebSocket 経由と同じ「コールバック内で直接送信」する方式に統一し、run ループがコールバックを待たないようにする
  - `SoraConnectionCommand::GetStats` コマンド処理: `pc.get_stats` のコールバック内で `stats_response_tx` (oneshot) に結果を直接送信する。コールバック内で `report.to_json()` を `JsonString` に変換して送信する
  - DataChannel 経由の `ping` (stats 付き) への応答: コールバック内で空の `pong` または stats 付きの `pong` を生成し、`event_tx` に送信する
  - DataChannel 経由の `req-stats` への応答: コールバック内で `stats` メッセージを生成し、`event_tx` に送信する
- run ループは get_stats のコールバックを await せず、`SoraEvent` 経由で届いた送信要求を既存の送信処理 (`send_message_for_signaling` / `send_data_channel_message`) で処理する。コールバックから DataChannel へ直接送信できないため、DataChannel への送信が必要な応答は run ループ側で行う
- コールバック不発時の永久待ちは、public API の `SoraConnectionHandle::get_stats` が通る `send_command` の `rx.await` にタイムアウトを設けることで防ぐ
  - タイムアウト値は既存のシグナリング待機 (SetRemoteDescription / createAnswer / SetLocalDescription、いずれも 5 秒) と同じ 5 秒に合わせる
  - タイムアウト時は新規エラーバリアントを返す。既存の `SetRemoteDescriptionTimeout` 等のタイムアウト系バリアントに倣い、`Error::GetStatsTimeout` を追加する
  - タイムアウト時は英語の warning ログを出力する
  - タイムアウト後にコールバックが遅れて発火した場合は、oneshot の Receiver がドロップ済みのため `tx.send` は失敗し、無視される (パニックやリークは発生しない)
- DataChannel 経由の `ping` (stats 付き) / `req-stats` は、コールバック不発時に無応答とする (WebSocket 経由と同じ挙動)。エラーを `handle_datachannel_message` の外へ伝播させない
- タイムアウトの検証は、実 PeerConnection ではコールバック不発を再現できないため、待機ロジックをタイムアウト値を引数で受け取るヘルパー関数に切り出し、oneshot チャネルを直接検証する単体テスト (`src/connection.rs` 内の `#[cfg(test)]` モジュール) で行う。タイムアウト後の挙動 (エラー応答の送信・無応答) と、タイムアウト後の遅延コールバック送信が無視されることを同テストで確認する

## 完了条件

- `get_stats` のコールバック待機中も run ループがブロックされず、disconnect を含む他のコマンドを処理できる
- コールバックが来ない場合でも `SoraConnectionHandle::get_stats` がタイムアウトで `Error::GetStatsTimeout` を返す
- タイムアウト後にコールバックが発火しても panic せず、無視される
- 正常系の統計取得挙動が変わらない
- `SoraConnectionHandle::get_stats` の doc コメントにタイムアウト時に `Error::GetStatsTimeout` を返す旨を追記する
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs` (private な `SoraConnection::get_stats`、`SoraConnectionCommand::GetStats` コマンド処理、`handle_datachannel_message`、`SoraConnectionHandle::get_stats` の doc コメント、テスト)
- `src/error.rs` (`Error::GetStatsTimeout` を追加)
- `CHANGES.md`
