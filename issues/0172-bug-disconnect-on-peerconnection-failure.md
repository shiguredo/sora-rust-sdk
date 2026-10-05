# 接続確立後に PeerConnection が失敗したら接続を終了する

- Created: 2026-10-05
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-disconnect-on-peerconnection-failure
- Polished: {YYYY-MM-DD}

## 目的

接続確立後にメディア経路 (ICE / DTLS) が死んだ場合に接続を終了する。WebSocket シグナリングではシグナリング経路が生き残るため、現状は `run` が終了せず利用者が異常を検知できない。

## 現状

接続確立前に WebRTC が成立しない場合は、Sora の `connection_created_wait_timeout` (デフォルト 30 秒) が WebSocket の 4490 クローズとして通知されるため `run` は終了する。

接続確立後はこれに相当する仕組みが無い。`PcObserverHandler::on_connection_change` はログを出すだけで、状態を `run` のループに渡していない。そのため PeerConnection が `failed` になっても `run` は待ち続ける。

Wi-Fi からモバイルへの切り替え、NAT リバインディング、VPN の切り替え、TURN 障害などでメディア経路だけが死ぬと、次の状態になる。

- 音声と映像の送受信が止まる
- WebSocket シグナリングは生きているため `on_websocket_close` も呼ばれない
- `run` は終了せず、利用者は接続中のまま異常を検知できない

参考実装の挙動は次のとおり。

- sora-js-sdk は `connectionState` / `iceConnectionState` が `failed` なら即座に切断し、`disconnected` のまま 10 秒経過したら切断する
- sora-ios-sdk は `RTCPeerConnectionState` が `failed` なら即座に切断し、`disconnected` のまま 5 秒経過したら切断する。`connected` / `connecting` に戻ったら猶予タイマーをキャンセルする

## 設計方針

`PcObserverHandler::on_connection_change` の状態を `SoraEvent` として `run` のループに渡し、接続を終了する条件を判定する。

- `failed` は終端状態なので即座に接続を終了する
- `disconnected` は回復し得るため、猶予時間の間 `disconnected` のままの場合に接続を終了する。`connected` または `connecting` に戻ったら猶予をキャンセルする
- 猶予時間は `SoraConnectionBuilder` で設定できるようにする。デフォルト値は実装時に決める (sora-js-sdk は 10 秒、sora-ios-sdk は 5 秒)
- 接続確立前の `failed` は Sora 側のタイムアウトと競合し得るため、既存の終了処理と二重に走らないようにする
- 終了時の切断理由は、切断理由を型付きで返す別 issue で扱う
- 状態変化の通知 (別 issue) とは独立して動作させる

## 完了条件

- 接続確立後に PeerConnection が `failed` になったら `run` が終了する
- 接続確立後に PeerConnection が `disconnected` のまま猶予時間を超えたら `run` が終了する
- `disconnected` から `connected` に復帰した場合は接続を維持する
- 猶予時間を設定できる
- 接続確立前の失敗で終了処理が二重に走らない

## 変更対象

- `src/connection.rs` の `PcObserverHandler`、`SoraEvent`、`run`、`SoraConnectionBuilder`
- `skills/sora-rust-sdk/SKILL.md` (設定と終了条件の記述)
