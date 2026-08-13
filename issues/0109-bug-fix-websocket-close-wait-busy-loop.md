# WebSocket クローズ完了待ちのビジーループを防ぐ

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-websocket-close-wait-busy-loop
- Polished: 2026-08-13

## 目的

DataChannel シグナリングへの切替後に WebSocket のクローズハンドシェイクを待つ間、100% CPU を消費するビジーループにならないようにする。

## 現状

`SoraConnection::run` のメインループ (`src/connection.rs`) では、`ws_disconnect_delay_start` が設定されている間、select! の遅延分岐が `tokio::time::sleep_until(start + WS_DISCONNECT_DELAY)` を待つ。

遅延経過後に `ws.close()` を送信しても `ws_disconnect_delay_start` はクリアされない。`start + WS_DISCONNECT_DELAY` が過去になった瞬間から遅延分岐が毎回即 Ready になり、他の分岐にイベントが無い間は select! がスピンし続ける。close ハンドシェイクの完了 (サーバーからの close 応答、または shiguredo_websocket の CloseTimeout 発火。デフォルト 5 秒) まで 100% CPU を消費する。

EOF・read/feed_recv_buf エラー吸収・redirect・`close_emitted` 経路では `ws_disconnect_delay_start = None` にリセットされるが、自発 `ws.close()` 送信後の経路だけリセットされない。遅延経過時に `ws.close()` を送信できない状態 (切替条件が崩れた、または state が `Connected` でない) だった場合も同様にリセットされない。特に、切替条件成立後に設定済みの DataChannel が閉じた場合は `is_data_channel_signaling_ready` が false になり close 送信ブロック全体がスキップされるため、EOF・サーバー close フレーム・ユーザー disconnect が来るまで無期限にスピンする。

## 設計方針

- 遅延分岐の判定を単体テスト可能な関数に切り出し、メインループとテストが同じ関数を使う
  - 関数は `ws.state()` が `Connected` でない (close 送信後) とき、または切替条件が成立しないときに `None` を返す (遅延分岐は pending になる)
  - それ以外は現在の `ws_disconnect_delay_start` を維持し、未設定なら `now` で設定する
- close ハンドシェイクの完了待機自体は、shiguredo_websocket の CloseTimeout タイマーと既存の close 検知 (timer_rx 経由の `Closed` 遷移 → `close_emitted`) が担っており、ビジーループの原因は遅延分岐が期限切れの `sleep_until` で毎回即 Ready になることだけである
- 切替条件崩壊時は close 送信ブロック自体がスキップされるため、遅延分岐判定の呼び出しは既存の切替判定 if ブロックの外側 (else 分岐等) で行う
- リセットタイミングを変えた場合、DataChannel シグナリング継続の判定 (EOF・flush error・Closed state) に影響がないことを確認する

## 完了条件

- 自発 `ws.close()` 送信後の close 完了待ちの間、select! がビジーループしない
- 切替条件が崩れた場合も、期限経過後にビジーループしない
- ビジーループの有無を検証するテストがある。設計方針の遅延分岐判定関数を対象に、`ws_disconnect_delay_start` が `Some(過去)` のままであっても、close 送信後 (state が `Connected` でない) と切替条件崩壊後は `None` を返すこと、待機中 (state が `Connected` で切替条件成立) は `None` を返さないことを検証する。これにより「リセット漏れ」と「再設定漏れ」がテストで検出できる。配置は `src/connection.rs` の `#[cfg(test)]` モジュール
- 既存の切替・クローズ検知の挙動が変わらない (e2e テスト `ignore_disconnect_websocket` / `server_close_message` で確認する)
- CHANGES.md の develop セクションに [FIX] を追記する
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs` (実装と、`#[cfg(test)]` モジュール内のビジーループ検証テスト)
- `CHANGES.md`
