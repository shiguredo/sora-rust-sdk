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
  - DataChannel 経由の `ping` (stats 付き) への応答: コールバック内で stats 付きの `pong` を生成し、`SoraEvent::SendDataChannelMessage` (ラベル `signaling`) を `event_tx` に送信する
  - DataChannel 経由の `req-stats` への応答: コールバック内で `stats` メッセージを生成し、`SoraEvent::SendDataChannelMessage` (ラベル `stats`) を `event_tx` に送信する
  - 非 stats 付きの `ping` (stats: false / None) への応答は同期送信のまま残し、新規 `SoraEvent` バリアント経由にはしない (既存の `ping_without_signaling_data_channel_returns_data_channel_missing` テストの `DataChannelMissing` エラー伝播を維持する)
- コールバックから応答チャネルへ直接送信できない場合は、送信要求を運ぶ新規 `SoraEvent` バリアントを追加し、run ループの match arm で送信する。追加するバリアントは以下の 2 つ
  - `SoraEvent::SendWebsocketMessage(String)`: WebSocket 経由でメッセージを送信する。`on_signaling_message` は発生させない
  - `SoraEvent::SendDataChannelMessage { label, message }`: 送信先ラベルを持つ DataChannel メッセージを送信する。`on_signaling_message` は発生させない
- pong や req-stats の結果を出力する際は、WebSocket 経路・DataChannel 経路とも、stats の有無にかかわらず `on_signaling_message` を発生させない仕様とする
  - WebSocket 経由で受信した `ping` / `req-stats` への応答: `send_pong` / `request_stats_pong` / `request_stats_response` が `SoraEvent::SendWebsocketMessage` を送信する (従来は `SoraEvent::SignalingMessage` を送信しており `on_signaling_message` が発生していたが、今回の変更で発生しなくなる)
  - DataChannel 経由で受信した `ping` (stats 付き) / `req-stats` への応答: コールバック内で `pong` または `stats` メッセージを生成し、`SoraEvent::SendDataChannelMessage` を送信する (従来 stats 付き `ping` への `pong` 応答では `on_signaling_message` が発生していたが、今回の変更で発生しなくなる。`req-stats` 応答は従来から発生させていない)
  - DataChannel 経由で受信した非 stats 付き `ping` への応答は同期送信のままとし、`on_signaling_message` を発生させない (従来は発生していたが、今回の変更で発生しなくなる)
  - `ping` 応答は `signaling` ラベル、`req-stats` 応答は `stats` ラベルに送信する (現在の挙動を維持する)
  - 送信経路は `use_data_channel_signaling` に依存させず、受信した経路と同じチャネル (WebSocket 経由で受信したら WebSocket、DataChannel 経由で受信したら DataChannel) へ常に送る
  - match arm での送信失敗は接続全体のエラーにせず、warning ログを出力して継続する (応答の送信失敗で接続を切らない。従来の `event_tx.send` の失敗を無視するのと同じ扱い)
- コールバック不発時の永久待ちは、`SoraConnectionHandle::get_stats` の待機にタイムアウトを設けることで防ぐ。`send_command` は get_stats 以外のコマンド (disconnect / send_rpc_request / send_message / selected_signaling_url / connected_signaling_url) でも共有している汎用ヘルパーであるため、`send_command` に `timeout: Option<Duration>` を追加してタイムアウトをサポートする。`None` の場合は従来通り `rx.await` で無限に待機し、`Some(duration)` の場合は `tokio::time::timeout` で包んで待機する。get_stats は `Some(Duration::from_secs(5))` を渡す
  - タイムアウト計測には run ループが `GetStats` コマンドを処理するまでの待ち時間も含まれる点に注意する (正常系ではコマンド処理は即座に完了するため問題にならないが、run ループが offer 処理等で占有されている場合に `Error::CommandTimeout` を返し得る)
  - タイムアウト値は既存のシグナリング待機 (SetRemoteDescription / createAnswer / SetLocalDescription、いずれも 5 秒) と同じ 5 秒に合わせる
  - タイムアウト時は新規エラーバリアントを返す。汎用ヘルパーの `send_command` が get_stats 専用エラーを返すのは不自然なため、既存の `SetRemoteDescriptionTimeout` 等のタイムアウト系バリアントと `CommandSendFailed` / `CommandResponseMissing` の流儀に倣い、失敗したコマンド名を持つ `Error::CommandTimeout { command }` を追加する。`Display` メッセージは既存のタイムアウト系バリアントと同じ日本語にする (production log は英語だが、`Error` の `Display` は既存バリアントが日本語のため)
  - タイムアウト時は英語の warning ログを出力する
  - タイムアウト後にコールバックが遅れて発火した場合は、oneshot の Receiver がドロップ済みのため `tx.send` は失敗し、無視される (パニックやリークは発生しない)
- DataChannel 経由の `ping` (stats 付き) / `req-stats` は、コールバック不発時に無応答とする (WebSocket 経由と同じ挙動)。エラーを `handle_data_channel_message` の外へ伝播させない
- 3 経路すべてがコールバック内で直接送信する方式に変わるため、現行の private な `async fn get_stats` (`src/connection.rs`) は呼び出し元を失う。未使用 private メソッドとして残すと dead_code 警告になるため、削除する
- タイムアウトの検証は、実 PeerConnection ではコールバック不発を再現できないため、`SoraConnectionHandle` と、`GetStats` コマンドを受信して応答を送らず oneshot の Sender をテスト側へ渡すサーバー (`spawn_message_server` と同じテスト内チャネル駆動のパターン) を立て、`send_command` (タイムアウトを短縮) 経由で単体テスト (`src/connection.rs` 内の `#[cfg(test)]` モジュール) を行う。タイムアウト後の挙動 (get_stats が `Error::CommandTimeout` を返すこと) と、タイムアウト後の遅延コールバック送信が無視されることを同テストで確認する。DataChannel 経路の「無応答」はコールバック不発を再現できないためテストではなく挙動の期待値として定義する

## 完了条件

- `get_stats` のコールバック待機中も run ループがブロックされず、disconnect を含む他のコマンドを処理できる
- コールバックが来ない場合でも `SoraConnectionHandle::get_stats` がタイムアウトで `Error::CommandTimeout` を返す
- タイムアウト後にコールバックが発火しても panic せず、無視される
- 正常系の統計取得挙動が変わらない
- `ping` / `req-stats` への応答 (pong / stats メッセージ) では、WebSocket 経路・DataChannel 経路とも `on_signaling_message` が発生しない
- `SoraConnectionHandle::get_stats` の doc コメントにタイムアウト時に `Error::CommandTimeout` を返す旨を追記する
- `cargo test --workspace` が成功する
- CHANGES.md の develop セクションに [FIX] を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs` (private な `SoraConnection::get_stats` の削除、`SoraConnectionCommand::GetStats` コマンド処理、`handle_data_channel_message`、`send_pong` / `request_stats_pong` / `request_stats_response`、`SoraConnectionHandle::get_stats` と `send_command` の `timeout: Option<Duration>` 追加、新規 `SoraEvent` バリアント (`SendWebsocketMessage` / `SendDataChannelMessage`) と run ループの match arm、テスト)
- `src/error.rs` (`Error::CommandTimeout { command }` を追加)
- `CHANGES.md`
