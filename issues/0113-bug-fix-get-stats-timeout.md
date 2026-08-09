# get_stats のコールバック待機にタイムアウトを設ける

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-get-stats-timeout
- Polished: {YYYY-MM-DD}

## 目的

`PeerConnection::get_stats` のコールバックが発火しない異常系で、`SoraConnection::run` のメインループが永久停止しないようにする。

## 現状

`SoraConnection::get_stats` (`src/connection.rs`) は `pc.get_stats` のコールバックを oneshot チャネルで待つが、`rx.await` にタイムアウトがない。libwebrtc は通常必ずコールバックするが、異常系でコールバックが来ないと `run()` が永久にブロックし、disconnect を含む全コマンドが処理できなくなる。

この待機は以下の 3 経路から呼ばれる。

- `SoraConnectionCommand::GetStats` コマンド処理
- DataChannel 経由の `ping` (stats 付き) への応答
- DataChannel 経由の `req-stats` への応答

## 設計方針

- `get_stats` の oneshot 待機にタイムアウトを設け、タイムアウト時はエラーを返す
- タイムアウト値は既存のシグナリング待機 (SetRemoteDescription 等の 5 秒) に合わせるか、別の妥当な値で固定する
- タイムアウト後の挙動 (メインループが継続すること) を確認する

## 完了条件

- コールバックが来ない場合でも `get_stats` がタイムアウトでエラーを返す
- タイムアウト後もメインループが継続し、他のコマンドが処理できる
- 正常系の統計取得挙動が変わらない
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `src/error.rs` (必要に応じて)
- `CHANGES.md`
