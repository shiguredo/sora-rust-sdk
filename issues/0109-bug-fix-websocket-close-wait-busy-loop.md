# WebSocket クローズ完了待ちのビジーループを防ぐ

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-websocket-close-wait-busy-loop
- Polished: {YYYY-MM-DD}

## 目的

DataChannel シグナリングへの切替後に WebSocket のクローズハンドシェイクを待つ間、100% CPU を消費するビジーループにならないようにする。

## 現状

`SoraConnection::run` のメインループ (`src/connection.rs`) では、`ws_disconnect_delay_start` が設定されている間、select! の遅延分岐が `tokio::time::sleep_until(start + WS_DISCONNECT_DELAY)` を待つ。

遅延経過後に `ws.close()` を送信しても `ws_disconnect_delay_start` はクリアされない。`start + WS_DISCONNECT_DELAY` が過去になった瞬間から遅延分岐が毎回即 Ready になり、他の分岐にイベントが無い間は select! がスピンし続ける。close ハンドシェイクの完了 (サーバーからの close 応答待ち、最大 shiguredo_websocket の CloseTimeout) まで 100% CPU を消費する。

EOF 経路と `close_emitted` 経路では `ws_disconnect_delay_start = None` にリセットされるが、自発 `ws.close()` 送信後の経路だけリセットされない。

## 設計方針

- `ws.close()` 送信後に `ws_disconnect_delay_start` を `None` にリセットする
- または、close 送信後の待機を別の手段 (CloseTimeout タイマー待ち等) に置き換え、遅延分岐を即 Ready にしない
- リセットタイミングを変えた場合、DataChannel シグナリング継続の判定 (EOF・flush error・Closed state) に影響がないことを確認する

## 完了条件

- 自発 `ws.close()` 送信後の close 完了待ちの間、select! がビジーループしない
- ビジーループの有無を検証するテスト (クローズ待機中に他のイベントが処理されること) がある
- 既存の切替・クローズ検知の挙動が変わらない
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `CHANGES.md`
