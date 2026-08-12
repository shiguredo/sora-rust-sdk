# get_stats のコールバック待機にタイムアウトを設ける

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-get-stats-timeout
- Polished: 2026-08-12

## 目的

`PeerConnection::get_stats` のコールバックが発火しない異常系で、`SoraConnection::run` のメインループが永久停止しないようにする。

## 現状

`SoraConnection::get_stats` (`src/connection.rs` の private な `async fn get_stats`) は `pc.get_stats` のコールバックを oneshot チャネルで待つが、`rx.await` にタイムアウトがない。コールバック自体が呼ばれない場合 (libwebrtc 内部の挙動に依存する) は oneshot の Sender がドロップされず、`rx.await` が永久にブロックする。このとき `run()` は WebSocket 読み取り・DataChannel イベント・タイマー・disconnect を含む全コマンドの処理を停止する。なお、null report 付きでコールバックが呼ばれた場合も Sender はドロップされ、`rx.await` は即復帰する。

この待機は以下の 3 経路から呼ばれる。

- `SoraConnectionCommand::GetStats` コマンド処理
- DataChannel 経由の `ping` (stats 付き) への応答
- DataChannel 経由の `req-stats` への応答

なお、WebSocket 経由の `ping` (stats 付き) / `req-stats` 応答は `request_stats_pong` / `request_stats_response` (`src/connection.rs`) がコールバック内で直接送信するため待機を持たず、対象外である。public API の `SoraConnectionHandle::get_stats` が通る `send_command` の `rx.await` もタイムアウトを持たないが、修正後は run ループがタイムアウト時にエラー応答を送信するため解決し、改修不要である。

## 設計方針

- 3 経路が共通で呼ぶ private な `SoraConnection::get_stats` の oneshot 待機にタイムアウトを設ける
- タイムアウト値は既存のシグナリング待機 (SetRemoteDescription / createAnswer / SetLocalDescription、いずれも 5 秒) と同じ 5 秒に合わせる
- タイムアウト時は新規エラーバリアントを返す。既存の `SetRemoteDescriptionTimeout` 等のタイムアウト系バリアントに倣い、`Error::GetStatsTimeout` を追加する
- タイムアウト時は英語の warning ログを出力する
- 経路ごとのタイムアウト時の挙動は次のとおりとし、エラーを `handle_datachannel_message` の外へ伝播させない
  - `SoraConnectionCommand::GetStats` コマンド処理: oneshot の送信元へエラーを返す
  - DataChannel 経由の `ping` (stats 付き) への応答: 空の `pong` にフォールバックする (現状の `.ok()` による挙動を維持する)
  - DataChannel 経由の `req-stats` への応答: 応答なしで継続する (現状の `if let Ok` による挙動を維持する)
- タイムアウトの検証は、実 PeerConnection ではコールバック不発を再現できないため、待機ロジックをタイムアウト値を引数で受け取るヘルパー関数に切り出し、oneshot チャネルを直接検証する単体テスト (`src/connection.rs` 内の `#[cfg(test)]` モジュール) で行う。タイムアウト後のメインループ継続は、経路ごとのフォールバック挙動 (空 pong・無応答・エラー応答の送信) が維持されることを同テストで確認する

## 完了条件

- コールバックが来ない場合でも `get_stats` がタイムアウトでエラーを返す
- タイムアウト後もメインループが継続し、他のコマンドが処理できる
- 正常系の統計取得挙動が変わらない
- `SoraConnectionHandle::get_stats` の doc コメントにタイムアウト時に `Error::GetStatsTimeout` を返す旨を追記する
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs` (private な `SoraConnection::get_stats` の待機、`SoraConnectionHandle::get_stats` の doc コメント、テスト)
- `src/error.rs` (`Error::GetStatsTimeout` を追加)
- `CHANGES.md`
