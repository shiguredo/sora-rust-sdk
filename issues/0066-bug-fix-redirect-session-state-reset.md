# redirect 時に session 状態をリセットする

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-28
- Model: Opus 4.7
- Branch: feature/fix-redirect-session-state-reset
- Polished: 2026-07-27

## 目的

`SoraConnection::run` の redirect 処理で、`websocket_closed` / `redirect` / `timers` / `ws` / `stream` / `connected_signaling_url` はリセットされるが、`switched_received` / `switched_ignore_disconnect_websocket` / `use_datachannel_signaling` / `opened_datachannels` / `self.data_channels` / `self.data_channel_configs` / `self.pending_rpc_responses` などの接続固有状態が前セッションから持ち越される。redirect 分岐冒頭でこれらの session 状態をクリーンにリセットする。

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

以下の状態はクリアされない。なお `switched_received` / `switched_ignore_disconnect_websocket` / `use_datachannel_signaling` / `opened_datachannels` / `ws_disconnect_delay_start` は `run()` 内のローカル変数である。

- `switched_received` — 旧セッションで switched 済みのまま持ち越され、redirect 先が switched を返さないサーバーでも `switched_received && switched_ignore_disconnect_websocket` が真になり、意図しない WebSocket 切断が始まる
- `switched_ignore_disconnect_websocket`
- `use_datachannel_signaling` — 旧セッションで DataChannel シグナリングが有効化されていると、redirect 先の新 WebSocket 接続直後にシグナリングメッセージが DC 側に流れてしまう
- `opened_datachannels` (旧セッションで open した DC 名が残る)
- `self.data_channels` (旧 `ManagedDataChannel` エントリ)
- `self.data_channel_configs` — redirect 後・新 offer 前に `self.data_channel_configs.len()` が読まれ (l.1232)、意図しない DC シグナリング切替条件が発火しうる
- `self.pending_rpc_responses` (旧セッションの RPC id は絶対に返ってこない → 5 秒後 RpcTimeout)
- `self.simulcast_encodings` — 新 offer 受信時 (`l.1085-1087`) に上書きされ、これらの値を実際に読む `handle_offer()` (`l.1533`) は上書き後に呼ばれるため、旧値が誤動作を起こす経路は存在しない。ただし redirect〜新 offer 間の防御的リセットとして初期値に戻す。
- `self.offer_simulcast` — 同上
- `ws_disconnect_delay_start`
- `self.rpc_id_counter` — `pending_rpc_responses` がクリアされるため ID 衝突の実害はないが、防御的リセットの対象に含める

## 設計方針

1. redirect 処理の冒頭（`redirect_location.take()` 直後）で、`## 現状` に列挙した全フィールドを初期状態に戻す。対象にはローカル変数 (`switched_received` 等) と `self` のフィールド (`data_channels` 等) の両方が含まれるため、`self` のメソッドではなく redirect 分岐内のインラインブロックとして実装する。
2. リセット前に、`opened_datachannels` に登録がある各ラベルについて `handler.on_data_channel_close(label)` を呼んでユーザーに通知する。`self.data_channels` のクリアは、`handler.on_data_channel_close` の通知完了後に行う（close 通知をクリア前に完了させるため）。
3. 既存 PeerConnection は redirect 時に close して作り直すのが安全 (別 issue で検討)。本 issue では PeerConnection のリセットは範囲外。したがって `self.video_sender`（`pc.add_track()` 由来の `RtpSender`）はリセットしない（PeerConnection が同一のままなら旧値は引き続き有効であり、かつ `self.config.sender_video_track` は初回 offer 時に `.take()` で消費済みのため redirect 後は再設定不可）。
4. `self.simulcast_encodings` / `self.offer_simulcast` は新 offer 受信時 (`l.1085-1087`) に上書きされ、これらの値を実際に読む `handle_offer()` (`l.1533`) は上書き後に呼ばれるため、旧値が誤動作を起こす経路は存在しない。ただし redirect〜新 offer 間で外部から何らかの形で読まれる可能性を排除できないため、防御的リセットとして初期値に戻す。
5. `self.pending_rpc_responses` は各エントリの `oneshot::Sender` に `Err(Error::Redirected)` (新設) を send してから `clear()` する。`Error::CommandSendFailed` は「コマンド送信失敗」を意味し redirect とは意味が異なるため新設する。`response_tx` は `Option<oneshot::Sender>` であり、送信前に `.take()` で取り出すこと。`PendingRpcRequest` の `Drop` が `timeout_handle.abort()` のみを行う実装 (`l.577-581`) であり、エラー送信は `Drop` に任せず明示的に行う。
6. リセット処理のユニットテストは、`SoraConnection` の生成コストが高いため難しい。少なくとも意図をコメントで明示する。
7. `self.event_rx` には、redirect 前に送信された旧セッション由来のイベントが滞留している可能性がある。とくに `DataChannelMessage`（旧データで `handler.on_data_channel_message` が呼ばれる）や `DataChannelRegister`（クリア済みの `self.data_channels` に旧チャネルが再登録される）が問題になりうる。redirect 分岐内で `while self.event_rx.try_recv().is_ok() {}` によりドレインする。ただし PeerConnection が同一のままであるため、`Track` 系のイベントは依然として有効な可能性があり、一律ドレインの是非は注意を要する。

## 解決方法

`SoraConnection::run` の redirect 分岐冒頭で、以下の session 状態をすべて初期状態にリセットした。
- ローカル変数: `switched_received`, `switched_ignore_disconnect_websocket`, `use_datachannel_signaling`, `opened_datachannels`, `ws_disconnect_delay_start`
- `self` フィールド: `data_channels`, `data_channel_configs`, `pending_rpc_responses`, `simulcast_encodings`, `offer_simulcast`, `rpc_id_counter`
- リセット前に `opened_datachannels` の各ラベルについて `handler.on_data_channel_close` を呼びユーザーに通知する。
- `pending_rpc_responses` の各エントリには `Error::Redirected` (新設) を send してから clear する。
- `event_rx` を `try_recv()` でドレインして旧セッション由来の滞留イベントを破棄する。

## 完了条件

- redirect 分岐冒頭で session 状態がクリーンにリセットされる。
- redirect 時に旧 DataChannel の close 通知 (`handler.on_data_channel_close`) がユーザーに届く。
- 旧セッションの `pending_rpc_responses` が redirect 時に `Error::Redirected` でエラー通知されて clear される。
- リセット処理の意図がコードコメントで明示されている。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
