# `ignore_disconnect_websocket=true` で WebSocket の close_notify なし切断時に run task が異常終了する

Created: 2026-05-06
Model: Opus 4.7

## 概要

`ignore_disconnect_websocket=true` を指定して接続したとき、`switched` 後に WebSocket が
切断されても DataChannel シグナリングは継続するべきだが、ピアが TLS の `close_notify` を
送らずに TCP を閉じた場合、`SoraConnection::run()` が `UnexpectedEof` を伝播させて即終了する。
結果、DataChannel 経由のコマンド (stats 取得・RPC・send_message など) も応答しなくなる。

## 該当箇所

- `src/connection.rs:859-861`

```rust
read = stream.read(&mut buf), if !websocket_closed => {
    let n = read?;        // close_notify なしの切断は UnexpectedEof でここで死ぬ
    if n == 0 {
        if switched_ignore_disconnect_websocket {
            // DataChannel シグナリング継続
        } else {
            break;
        }
    }
    ...
}
```

`stream.read()` が `Ok(0)` (close_notify あり) を返した場合は
`switched_ignore_disconnect_websocket` 分岐で握り潰されて DataChannel シグナリングが
継続するが、`Err(UnexpectedEof)` (close_notify なし) は `?` でそのまま `Error` に
変換されて `run()` が抜ける。

## 再現手順

1. `data_channel_signaling(true)` + `ignore_disconnect_websocket(true)` で Sora に接続する
2. `switched` 通知を受信し、全 DataChannel が open する
3. SDK が `WS_DISCONNECT_DELAY (10 秒)` 経過後に `ws.close(NORMAL, "switching to datachannel")` を発行
4. ピアが close handshake のあと `close_notify` を送らずに TCP を閉じる
5. `stream.read()` が `Err(UnexpectedEof)` を返す
6. `run()` がエラー終了し、command channel の receiver が drop され、
   以降の `get_stats()` などが `CommandResponseMissing { source: RecvError(()) }` で失敗する

## 再現時のエラーメッセージ

`e2e-tests/tests/ignore_disconnect_websocket.rs` の
`test_recvonly_ignore_disconnect_websocket_keeps_signaling` を実行すると以下で失敗する:

```
panicked at e2e-tests/tests/ignore_disconnect_websocket.rs:61:10:
WebSocket クローズ後の stats 取得に失敗しました (run task が異常終了している可能性):
  CommandResponseMissing { source: RecvError(()), command: "get_stats" }
```

`WebsocketClose` コールバックは発火しているため、close handshake 自体は成立しており、
直後の TCP 切断 (`UnexpectedEof`) のみで run task が死んでいることを確認している。

## 原因

`tokio::io::AsyncReadExt::read` は ピアが `close_notify` を送らずに TCP を閉じた場合に
`Err(UnexpectedEof)` を返す。一方、`close_notify` 付きで閉じた場合は `Ok(0)` を返す。
`ignore_disconnect_websocket=true` が成立した状況では「WebSocket レイヤの切断は無視して
DataChannel で通信を継続する」のが期待挙動であり、close_notify の有無で挙動が変わるのは
ピア (Sora や中間機器) の実装に依存する不安定要素になる。

## 優先度

高

`ignore_disconnect_websocket=true` の本来の意図 (WebSocket が切れても通信を継続する) が
ピアの close_notify 送信有無に依存して破綻するため。

## 修正方針

`stream.read()` の戻り値を分岐し、`UnexpectedEof` を `n = 0` と同等に扱って
既存の `switched_ignore_disconnect_websocket` 分岐へ合流させる。

```rust
read = stream.read(&mut buf), if !websocket_closed => {
    let n = match read {
        Ok(n) => n,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => 0,
        Err(e) => return Err(e.into()),
    };
    if n == 0 { /* 既存処理 */ }
    ...
}
```

## テスト

`e2e-tests/tests/ignore_disconnect_websocket.rs` を追加済み。
修正前は `CommandResponseMissing` で失敗し、修正後は green になることを確認する。
