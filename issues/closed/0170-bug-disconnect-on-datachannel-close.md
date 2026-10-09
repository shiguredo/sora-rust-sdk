# DataChannel が閉じられたら接続を終了する

- Created: 2026-10-05
- Completed: 2026-10-05
- Branch: feature/fix-disconnect-on-datachannel-close
- Polished: {YYYY-MM-DD}

## 目的

接続中に DataChannel が閉じられた場合に接続を終了し、利用者が異常を検知できるようにする。DataChannel シグナリングで `signaling` ラベルが閉じるとシグナリング手段が無くなるため、接続を維持しても復帰できない。

## 現状

`SoraConnection::run` は `SoraEvent::DataChannelStateChange` を受けると `SoraConnection::handle_data_channel_state` を呼ぶ。これは `on_data_channel_open` と `on_data_channel_close` を通知するだけで、接続を終了しない。

`SoraEvent::DataChannelStateChange` を待って close 通知を出す `wait_data_channels_close` は `run` のループを抜けた後の処理であり、接続中に DataChannel が閉じられたことを切断として扱う経路が無い。

DataChannel シグナリングと `ignore_disconnect_websocket=true` を組み合わせた構成で `signaling` ラベルが閉じると、次の状態になる。

- `use_data_channel_signaling` は true のまま (false に戻すのは redirect 時だけ) なので、シグナリングの送信は閉じた DataChannel に対して行われる
- 送信の扱いは経路によって異なる。`SoraEvent::SignalingMessage` の送信は `?` で `run` を `Err` 終了させるが、`SoraEvent::SendDataChannelMessage` と `SoraEvent::SendWebSocketMessage` の失敗は警告ログのみになる
- WebSocket は DataChannel シグナリングへの切り替え後に SDK 自身が閉じるため、サーバーからの re-offer に re-answer を返す手段が無い
- サーバーの `close` メッセージも届かないため `run` は終了せず、利用者は異常を検知できない

参考実装の挙動は次のとおり。

- sora-cpp-sdk は offer の `data_channels` に含まれる DataChannel が接続中に閉じられたら `DATACHANNEL_CLOSED` で通知する
- sora-ios-sdk は `readyState == .closed` になったら `DisconnectReason.dataChannelClosed` として切断する
- sora-js-sdk はすべての DataChannel の close で `disconnect()` を呼ぶ

## 設計方針

接続中に閉じた DataChannel を検知したら接続を終了する。

- 対象は offer の `data_channels` に含まれるすべてのラベルとする (`signaling` / `stats` などの SDK 内部ラベルと `#` プレフィックスのユーザー定義ラベル)。一部だけが閉じた接続は健全ではないため、接続全体を終了する
- 検知は `SoraConnection::handle_data_channel_state` の close 判定に合わせる
- 意図的な終了と競合させない
  - クライアント起点の切断 (`SoraConnectionCommand::Disconnect`) の進行中
  - サーバーの `close` メッセージの受信後 (すべての DataChannel が閉じるのを待つ既存処理を優先する)
  - redirect の進行中 (旧セッションの DataChannel close を新しい接続の終了として扱わない)
- `on_data_channel_close` はこれまでどおり通知し、二重通知を起こさない
- 終了時の切断理由は、切断理由を型付きで返す別 issue で `DataChannelClosed` として扱う。この issue では `run` が終了するところまでを扱う

## 完了条件

- 接続中に offer の `data_channels` に含まれる任意の DataChannel が閉じられたら `run` が終了する
- `signaling` ラベル以外 (`stats` や `#` プレフィックスのラベル) が閉じられた場合も `run` が終了する
- クライアント起点の切断とサーバーの `close` メッセージによる終了で、二重通知や close code / reason の喪失が起きない
- DataChannel シグナリングと `ignore_disconnect_websocket=true` を組み合わせた構成で `signaling` が閉じられた場合に `run` が終了する

## 変更対象

- `src/connection.rs` の `SoraConnection::handle_data_channel_state` と `run`
- `skills/sora-rust-sdk/SKILL.md` (終了条件の記述)

## 解決方法

`SoraConnection::handle_data_channel_state` が `DataChannelStateResult` を返すようにした。
Offer の `data_channels` に含まれ Open を観測した DataChannel が Closed へ遷移したら `on_data_channel_close` を通知して `Terminate` を返し、`SoraEvent::DataChannelStateChange` と `SoraEvent::DataChannelRegister` の処理で `run` のループを抜けて接続を終了する。
一部の DataChannel だけが閉じた場合も、そのラベルを使うシグナリング・統計・RPC・利用者メッセージは復帰できないため接続全体を終了する。

クライアント起点の切断・サーバーの `close` メッセージ・redirect はそれぞれの経路が `run` のループを抜けるため、終了処理は二重に走らない。
`opened_data_channels` からラベルを除去してから通知するため、`on_data_channel_close` の二重通知も起きない。
終了時の切断理由を型付きで返す対応は別 issue で扱う。

`signaling` が閉じた場合、`#` プレフィックスのユーザー定義ラベルが閉じた場合、`Closed` 以外の状態、Open を観測していないラベルの閉鎖の 4 通りを単体テストで検証した。
`skills/sora-rust-sdk/SKILL.md` に終了条件を、`CHANGES.md` に変更履歴を追記した。
