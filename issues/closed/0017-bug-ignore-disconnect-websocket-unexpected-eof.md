# `ignore_disconnect_websocket=true` でも WebSocket クローズ時に run task が終了し DataChannel シグナリングが継続しない

Created: 2026-05-06
Completed: 2026-05-06
Priority: High
Model: Opus 4.7

## 概要

`ignore_disconnect_websocket=true` を指定して接続したとき、`switched` 後も WebSocket が
切れても DataChannel シグナリングを継続するべきだが、実際には WebSocket close 完了直後に
`run()` ループが break して終了してしまう。

結果、command channel の receiver が drop され、以降の DataChannel 経由のコマンド
(stats 取得・RPC・send_message など) が `CommandResponseMissing { source: RecvError(()) }`
で失敗する。

## 該当箇所

- `src/connection.rs:1212` (旧): `flush_ws_output` が `Ok(true)` を返したら無条件 `break`
- `src/connection.rs:1216` (旧): `ws.state() == ConnectionState::Closed` で無条件 `break`
- `src/connection.rs:859-861` (旧): `stream.read()` のエラーを `?` でそのまま伝播

## 再現手順

1. `data_channel_signaling(true)` + `ignore_disconnect_websocket(true)` で Sora に接続する
2. `switched` 通知を受信し、全 DataChannel が open する
3. SDK が `WS_DISCONNECT_DELAY (10 秒)` 経過後に `ws.close(NORMAL, "switching to datachannel")` を発行する
4. Sora が close ack を返し、`ws` の状態が `ConnectionState::Closed` になる
5. `flush_ws_output` が `ConnectionOutput::CloseConnection` を吐き出して `Ok(true)` を返す
6. `connection.rs:1212` の `if let Ok(true) = flush_ws_output(...) { break; }` で外側ループを抜ける
7. `run()` が `Ok(())` で正常終了し、command channel が drop される
8. テスト側で `get_stats()` を呼ぶと `CommandResponseMissing { source: RecvError(()) }` で失敗する

## 再現時のエラーメッセージ

`e2e-tests/tests/ignore_disconnect_websocket.rs` の
`test_recvonly_ignore_disconnect_websocket_keeps_signaling` を実行すると以下で失敗する:

```
panicked at e2e-tests/tests/ignore_disconnect_websocket.rs:NN:
WebSocket クローズ後の stats 取得に失敗しました (run task が異常終了している可能性):
  CommandResponseMissing { source: RecvError(()), command: "get_stats" }
```

なお、run task の終了結果自体は `Ok(())` であり、エラーで死んでいるわけではない
(WebSocket close 完了で「正常終了」と扱われているのが問題)。

## 原因

`switched_received && switched_ignore_disconnect_websocket` のとき、SDK は意図的に
WebSocket を閉じる (`connection.rs:1208`) が、その後の close 完了処理 (1212 / 1216) は
`switched_ignore_disconnect_websocket` フラグを見ずに無条件で外側ループを break する。

結果、DataChannel シグナリングへの切替が行われず `run()` が即終了する。

## 優先度

高

`ignore_disconnect_websocket=true` の本来の意図 (WebSocket が切れても DataChannel で
通信を継続する) が成立しない。

## 解決方法

`src/connection.rs:1212` および `1216` の break を、
`switched_ignore_disconnect_websocket` フラグで分岐させた:

- フラグが立っているときは break せず、`websocket_closed = true` にして
  DataChannel シグナリング側で `run()` ループを継続する
- フラグが立っていないときは従来どおり break して `run()` を終了する

加えて、防御的な追加として `connection.rs:859-861` の `stream.read()` エラー処理を変更し、
`std::io::ErrorKind::UnexpectedEof` (ピアが close_notify を送らずに TCP を閉じたケース) を
`n == 0` と同等扱いに合流させた。これにより、close_notify を送らないピアに対しても
既存の `n == 0` ハンドリング (フラグが立っていれば DataChannel 継続、立っていなければ
正常終了) が適用される。

`e2e-tests/tests/ignore_disconnect_websocket.rs::test_recvonly_ignore_disconnect_websocket_keeps_signaling` で
修正前は `CommandResponseMissing` で失敗、修正後は green になることを確認した。
