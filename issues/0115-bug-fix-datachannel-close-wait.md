# 切断時の DataChannel クローズ待機処理を正す

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-datachannel-close-wait
- Polished: 2026-08-13

## 目的

`SoraConnection::run` 終了時の DataChannel クローズ待機処理の欠陥を修正し、ユーザーへの close 通知と切断操作の整合性を保つ。

## 現状

`SoraConnection::run` のシャットダウン待機ループ (`src/connection.rs`) に以下の欠陥がある。

- 待機ループは `DataChannelStateChange` イベントを受信しただけで `opened_data_channels` から remove する。通常の状態遷移ハンドラ (`handle_data_channel_state`) は `is_data_channel_closed` を確認してから remove するため、Closing 遷移の時点でまだ閉じていないチャネルが「閉じた」と通知される
- 待機ループは `event_rx` のみ監視し `command_rx` を処理しないため、待機中に `disconnect()` を呼ぶと ack が返らず `Error::CommandResponseMissing` になる
- 待機ループの `event_rx.recv()` が `None` を返した場合もタイムアウトと同一分岐で扱われ、「切断待機がタイムアウトしました」と誤った警告ログを出す。この経路は待機ループ中も `event_tx` の送信側が生存するため実質到達不能だが、event_rx クローズとタイムアウトを区別しないコード構造になっている

なお、待機ループのタイムアウト時に残りチャネルの close 通知を行わない欠陥は、既に修正済みである (タイムアウト時に残りチャネルへ close コールバックを通知する実装が存在する)。この挙動は closed issue 0090 の設計方針 (timeout した label には Closed 未観測のまま合成 close コールバックを呼ばない) とは異なるが、0107 の実装で現在の挙動に変更された。本 issue はこの現状を維持する (close 通知が漏れないことを優先する判断)。

## 設計方針

- 待機ループの remove 判定を `handle_data_channel_state` と同じくチャネル状態の確認付きにする (`is_data_channel_closed` を確認してから remove する)。判定は純粋関数 (チャネルの `is_closed` 判定を引数で受け取る関数) に切り出し、待機ループと `handle_data_channel_state` の両方から使って 1 箇所に集約する
- 待機中も `command_rx` を処理する。待機中に受信した `Disconnect` コマンドには ack を返し、待機を継続する (ack 時に close 通知や待機終了を行わない。close 通知は従来どおりチャネルの Closed 遷移とタイムアウト時に行う)
  - 待機中の `Disconnect` ではサーバーへの disconnect メッセージは再送しない (初回の切断要求で送信を試行済み、またはサーバー主導の切断のため不要。送信失敗時も再送しない。disconnect メッセージの送信は best-effort 扱い)
  - `Disconnect` 以外のコマンド (`GetStats` / `GetSelectedSignalingUrl` / `GetConnectedSignalingUrl` / `SendRpcRequest` / `SendMessage`) は受信しても破棄する (従来どおり呼び出し側は `Error::CommandResponseMissing` になる)。待機ループの目的はクローズ待機であり、これらの応答処理は不要のため処理しない
  - `command_rx` の分岐は `Some(command)` の arm パターンで受ける。クローズ済みチャネルに対する `recv()` は毎回即 `None` を返すため、`None` を arm 内で受け続ける実装にするとイベントの無い間に select! がビジーループする。`Some(command)` の arm パターンでは、各 select! 呼び出しで `None` 受信時に分岐が無効化され、event_rx の到着を待つためビジーループしない。`command_rx` が `None` (ハンドル全ドロップ) を返した後も待機は続き、残りチャネルの close 通知は漏らさない
- `event_rx` クローズ時とタイムアウト時でログを区別する (実質到達不能な経路だが、コード構造上は区別できる形にする)。`event_rx` は `event = self.event_rx.recv()` の形で受け、`None` を arm 内で処理する (`Some(event)` の arm パターンでは `None` 受信時に分岐が無効化されて区別できない)。`event_rx` クローズ時は、タイムアウト時と同じく残りチャネルへの close 通知を行ってから待機を終了する (現状の挙動を維持する。通知後にループを継続すると `None` が毎回即 Ready になりビジーループする)
- ログ区別の修正で待機ループの警告ログを変更する際は、AGENTS.md の規約に従い英語で記述する (対象は「切断待機がタイムアウトしました」の書き直しと、event_rx クローズ用に追加する新規ログのみ)。このうち「切断待機がタイムアウトしました」は open issue 0122 の日本語ログ英語化の対象にも含まれるが、本 issue で書き直すことにより 0122 の対象から外れる (0122 は残りの日本語ログと expect メッセージを扱う)
- リモート側のクローズでは libwebrtc が Closed 状態への遷移イベントを発することを前提とする。仮に Closed に遷移しないチャネルがあっても、`disconnect_wait_timeout` 満了時の分岐で close 通知は漏れない
- 本 issue の対象は DataChannel シグナリング時のクローズ待機ループのみとする。待機ループ終了後の WebSocket close handshake 中の `disconnect()` や、WebSocket シグナリング時のサーバー主導 Close 経路での close 通知は対象外とする

## 完了条件

- Closing 遷移で誤って「閉じた」と通知されない
- 待機中に `disconnect()` が呼ばれても ack が返り、`Error::CommandResponseMissing` にならない
- `event_rx` クローズ時とタイムアウト時でログが区別される
- 待機ループの修正を検証するテストがある
  - remove 判定ロジックを純粋関数 (チャネルの `is_closed` 判定を引数で受け取る関数) に切り出し、`src/connection.rs` 内の `#[cfg(test)]` モジュールで「Closing 状態では remove されない」「Closed 状態では remove される」ことを検証する。「Closing / Closed 状態」は注入する `is_closed` 述語の真偽で表現する (実 DataChannel の状態判定そのものの検証は実装と e2e に委ねる)
  - 待機ループのコマンド・イベント処理を実際の mpsc チャネルを引数で受け取る単体テスト可能な形に切り出し、`src/connection.rs` 内の `#[cfg(test)]` モジュールで次を検証する (実サーバー e2e では待機窓が短く決定的に再現できないため)
    - 待機ループが remove 判定を純粋関数経由で行うこと (Closing 状態の `DataChannelStateChange` イベントを受信しても close 通知・remove が発生しない)
    - 全チャネルの Closed イベントを受信すると close 通知が行われ、待機が正常終了すること
    - 待機中に `disconnect()` が呼ばれた場合に ack が返り、待機が継続すること (ack 時に close 通知・待機終了が発生せず、その後の `DataChannelStateChange` イベントが通常どおり処理される)
    - 待機中に `Disconnect` 以外のコマンド (`GetStats` 等) を受信しても応答されず、呼び出し側が `Error::CommandResponseMissing` になること
    - `event_rx` クローズ時とタイムアウト時の待機終了要因の区別 (ログ出力そのものではなく、待機終了要因を表す戻り値で検証する)
    - タイムアウト時に残りチャネルへ close 通知される現状維持の挙動
  - 実サーバーを使う e2e で、DataChannel が閉じられたときに close 通知が「重複・欠落なく 1 回」呼ばれることを検証する。既存の `e2e-tests/tests/server_close_message.rs` の `assert_data_channel_close_not_duplicated` は `close_count <= 1` (重複なし) しか検証しないため、「欠落なし」 (`== 1`) を含む検証に拡張する (関数名も内容に合わせて変更してよい)。この e2e は close 通知契約の回帰確認であり、Closing 遷移での誤通知 (通知タイミング) の検出は上記の単体テストが担う
  - モックやスタブは使わない
- `cargo test --workspace` が成功する
- CHANGES.md の develop セクションに [FIX] を追記する
- 本 issue で変更・追加する production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs` (待機ループの修正、remove 判定の純粋関数化)
- `src/connection.rs` 内の `#[cfg(test)]` モジュール (待機ループの単体テスト)、`e2e-tests/tests/server_close_message.rs` (close 通知契約の検証拡張)
- `CHANGES.md`
