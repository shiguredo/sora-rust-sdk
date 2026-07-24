# redirect 時に session 状態をリセットする経路を追加する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-redirect-session-state-reset
- Polished: {YYYY-MM-DD}

## 目的

`SoraConnection::run` の redirect 処理で、`websocket_closed` / `redirect` / `timers` / `ws` / `stream` / `connected_signaling_url` はリセットされるが、`switched_received` / `switched_ignore_disconnect_websocket` / `use_datachannel_signaling` / `opened_datachannels` / `self.data_channels` / `self.data_channel_configs` / `self.pending_rpc_responses` などの接続固有状態が前セッションから持ち越される。redirect 分岐冒頭で session 状態をクリーンにリセットする関数を追加する。

## 優先度根拠

High。Sora 側の実装や運用で redirect が offer 後や switched 後に来た場合、旧接続の状態を持ち越して新セッションに入るため、`DataChannelSendFailed` / `RpcTimeout` / DataChannel の close 通知漏れなど、複数の連鎖障害が起きる。Sora 側の正常運用では redirect は接続前段で来るが、SDK 側は防御的にリセットすべき。

## 現状

`src/connection.rs:1184-1228` の redirect 処理では以下しかリセットしていない:

```rust
stream = connect_websocket(&new_target, proxy.as_ref(), &tls_config, ...).await?;
self.connected_signaling_url = Some(location);
...
ws = WebSocketClientConnection::new(options, secure_random.clone());
ws.connect()?;
websocket_closed = false;
redirect = true;
```

以下の状態はクリアされない:

- `switched_received`
- `switched_ignore_disconnect_websocket`
- `use_datachannel_signaling`
- `opened_datachannels` (旧セッションで open した DC 名が残る)
- `self.data_channels` (旧 `ManagedDataChannel` エントリ)
- `self.data_channel_configs`
- `self.pending_rpc_responses` (旧セッションの RPC id は絶対に返ってこない → 5 秒後 RpcTimeout)
- `self.simulcast_encodings`
- `self.offer_simulcast`
- `self.video_sender`
- `ws_disconnect_delay_start`

## 設計方針

1. redirect 処理の冒頭で `SoraConnection::reset_session_state()` (仮) を呼び、上記フィールドをすべて初期状態に戻す。
2. `opened_datachannels` に登録がある場合は、リセット前に `handler.on_data_channel_close(label)` を呼んで通知する (ユーザー可観測性)。
3. 既存 PeerConnection は redirect 時に close して作り直すのが安全 (別 issue で検討)。本 issue では PeerConnection のリセット可否は範囲外。
4. `pending_rpc_responses` は各 `oneshot::Sender` に `Err(Error::CommandSendFailed)` あるいは新設のエラーを返してから clear する (呼び出し側が無限に待たない)。
5. リセット関数のユニットテストは、`SoraConnection` の生成コストが高いため難しい。少なくとも意図をコメントで明示する。

## 完了条件

- redirect 分岐冒頭でセッション状態がクリーンにリセットされる。
- redirect 直後に旧 DataChannel の close 通知がユーザーに届く。
- 旧セッションの `pending_rpc_responses` が redirect 時にエラー通知されて clear される。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
