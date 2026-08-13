# get_stats で run ループがブロックされないようにする

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-get-stats-run-loop-blocking
- Polished: 2026-08-13

## 目的

`PeerConnection::get_stats` のコールバック待機中に `SoraConnection::run` のメインループがブロックされないようにする。あわせて、コールバックが発火しない異常系で待機が永久に続かないようにする。

## 現状

`SoraConnection::get_stats` (`src/connection.rs` の private な `async fn get_stats`) は `pc.get_stats` のコールバックを oneshot チャネルで待つ。この待機は以下の 3 経路から呼ばれ、いずれも run ループ内で `await` されるため、コールバックが発火するまでの間、run ループ全体 (WebSocket 読み取り・DataChannel イベント・タイマー・disconnect を含む全コマンドの処理) が停止する。

- `SoraConnectionCommand::GetStats` コマンド処理
- DataChannel 経由の `ping` (stats 付き) への応答 (`handle_data_channel_message` 内)
- DataChannel 経由の `req-stats` への応答 (`handle_data_channel_message` 内)

また、コールバック自体が呼ばれない場合 (libwebrtc 内部の挙動に依存する) は oneshot の Sender がドロップされず、`rx.await` が永久にブロックする。

一方、WebSocket 経由の `ping` (stats 付き) / `req-stats` 応答は `request_stats_pong` / `request_stats_response` (`src/connection.rs`) がコールバック内で直接 `event_tx` に送信するため待機を持たず、run ループをブロックしない。public API の `SoraConnectionHandle::get_stats` が通る `send_command` の `rx.await` もタイムアウトを持たない。

## 設計方針

- 3 経路とも、run ループが get_stats のコールバックを await しない方式に統一する
  - `SoraConnectionCommand::GetStats` コマンド処理: `pc.get_stats` のコールバック内で `stats_response_tx` (oneshot) に結果を直接送信する。コールバック内で `report.to_json()` の結果を `JsonString` に変換 (`.parse()` 相当の再検証を含む) して送信する
  - DataChannel 経由の `ping` (stats 付き) への応答: コールバック内で空の `pong` または stats 付きの `pong` を生成し、`event_tx` に送信する
  - DataChannel 経由の `req-stats` への応答: コールバック内で `stats` メッセージを生成し、`event_tx` に送信する
  - 非 stats 付きの `ping` (stats: false / None) は同期送信のまま残し、新規 `SoraEvent` バリアント経由にはしない (既存の `ping_without_signaling_data_channel_returns_data_channel_missing` テストの挙動を維持する)
- DataChannel 経由の応答は、コールバックから DataChannel へ直接送信できないため、送信要求 (送信先ラベル + メッセージ) を運ぶ新規 `SoraEvent` バリアントを追加し、run ループの match arm で `send_data_channel_message` により送信する
  - `ping` 応答は `signaling` ラベル、`req-stats` 応答は `stats` ラベルに送信する (現在の挙動を維持する)
  - 送信経路は `use_data_channel_signaling` に依存させず、DataChannel 経路の応答は常に DataChannel へ送る (切替前の状態でも WebSocket へ送らない)
  - `handler.on_signaling_message(SignalingType::DataChannel, SignalingDirection::Sent, ...)` 通知は `ping` 応答 (送信先ラベルが `signaling`) の場合のみ呼び、`req-stats` 応答 (`stats` ラベル) では呼ばない (現在の挙動を維持する)。判定は新規バリアントが運ぶ送信先ラベルで行う
  - match arm での `send_data_channel_message` の失敗は接続全体のエラーにせず、warning ログを出力して継続する (応答の送信失敗で接続を切らない。WebSocket 経路の `request_stats_pong` / `request_stats_response` が `event_tx.send` の失敗を無視するのと同じ扱い)
- コールバック不発時の永久待ちは、`SoraConnectionHandle::get_stats` の待機にタイムアウトを設けることで防ぐ。`send_command` は get_stats 以外のコマンド (disconnect / send_rpc_request / send_message / selected_signaling_url / connected_signaling_url) でも共有している汎用ヘルパーであり、`send_rpc_request` はユーザー指定の待機タイムアウト (`RpcRequestOptions.timeout`) を持つため、**`send_command` 自体は変更せず**、get_stats の待機だけをタイムアウト付きでラップする
  - タイムアウト計測には run ループが `GetStats` コマンドを処理するまでの待ち時間も含まれる点に注意する (正常系ではコマンド処理は即座に完了するため問題にならないが、run ループが offer 処理等で占有されている場合に `Error::GetStatsTimeout` を返し得る)
  - タイムアウト値は既存のシグナリング待機 (SetRemoteDescription / createAnswer / SetLocalDescription、いずれも 5 秒) と同じ 5 秒に合わせる
  - タイムアウト時は新規エラーバリアントを返す。既存の `SetRemoteDescriptionTimeout` 等のタイムアウト系バリアントに倣い、`Error::GetStatsTimeout` を追加する。`Display` メッセージは既存のタイムアウト系バリアントと同じ日本語にする (production log は英語だが、`Error` の `Display` は既存バリアントが日本語のため)
  - タイムアウト時は英語の warning ログを出力する
  - タイムアウト後にコールバックが遅れて発火した場合は、oneshot の Receiver がドロップ済みのため `tx.send` は失敗し、無視される (パニックやリークは発生しない)
- DataChannel 経由の `ping` (stats 付き) / `req-stats` は、コールバック不発時に無応答とする (WebSocket 経由と同じ挙動)。エラーを `handle_data_channel_message` の外へ伝播させない
- 3 経路すべてがコールバック内で直接送信する方式に変わるため、現行の private な `async fn get_stats` (`src/connection.rs`) は呼び出し元を失う。未使用 private メソッドとして残すと dead_code 警告になるため、削除する
- タイムアウトの検証は、実 PeerConnection ではコールバック不発を再現できないため、待機ロジックをタイムアウト値を引数で受け取るヘルパー関数に切り出し、oneshot チャネルを直接検証する単体テスト (`src/connection.rs` 内の `#[cfg(test)]` モジュール) で行う。タイムアウト後の挙動 (get_stats が `Error::GetStatsTimeout` を返すこと) と、タイムアウト後の遅延コールバック送信が無視されることを同テストで確認する。DataChannel 経路の「無応答」はコールバック不発を再現できないためテストではなく挙動の期待値として定義する

## 完了条件

- `get_stats` のコールバック待機中も run ループがブロックされず、disconnect を含む他のコマンドを処理できる
- コールバックが来ない場合でも `SoraConnectionHandle::get_stats` がタイムアウトで `Error::GetStatsTimeout` を返す
- タイムアウト後にコールバックが発火しても panic せず、無視される
- 正常系の統計取得挙動が変わらない
- `SoraConnectionHandle::get_stats` の doc コメントにタイムアウト時に `Error::GetStatsTimeout` を返す旨を追記する
- `cargo test --workspace` が成功する
- CHANGES.md の develop セクションに [FIX] を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs` (private な `SoraConnection::get_stats` の削除、`SoraConnectionCommand::GetStats` コマンド処理、`handle_data_channel_message`、`SoraConnectionHandle::get_stats` の doc コメント、新規 `SoraEvent` バリアントと run ループの match arm、テスト)
- `src/error.rs` (`Error::GetStatsTimeout` を追加)
- `CHANGES.md`
