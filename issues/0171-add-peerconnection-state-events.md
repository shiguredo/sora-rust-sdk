# PeerConnection / ICE / ICE gathering / SignalingState の状態変化をイベントで通知する

- Created: 2026-10-05
- Completed: 2026-10-06
- Branch: feature/add-peerconnection-state-events
- Polished: {YYYY-MM-DD}

## 目的

PeerConnection / ICE / ICE gathering / SignalingState の状態変化を SDK の利用者が取得できるようにする。接続状態の表示、ICE 失敗の判別、再接続の判断、ネゴシエーション状態の把握に使う。

## 現状

`SoraConnectionEventHandler` に WebRTC の接続状態を通知するメソッドが無い。

webrtc-rs の `PeerConnectionObserverHandler` には `on_connection_change` (`PeerConnectionState`) と `on_standardized_ice_connection_change` (`IceConnectionState`) と `on_ice_gathering_change` (`IceGatheringState`) がある。

sora-rust-sdk は `on_connection_change` を実装しているが `rtc_log_info!` でログ出力するだけで、イベントハンドラにもハンドルにも渡していない。ICE 系の 2 つは実装していない。

SignalingState は webrtc-rs が公開していない。C ラッパーの `peer_connection_interface.cc` の `OnSignalingChange` が空実装で、`signaling_state` の getter も無い。このため sora-rust-sdk だけでは対応できない。webrtc-rs 側で `SignalingState` と `on_signaling_change` を公開する必要があり、その対応は webrtc-rs の issue として起票済み。

## 設計方針

`SoraConnectionEventHandler` に WebRTC の状態変化のコールバックを追加する。

- `on_connection_change` / `on_standardized_ice_connection_change` / `on_ice_gathering_change` と、SignalingState のコールバックを追加する
- 状態の型は `shiguredo_webrtc` の `PeerConnectionState` / `IceConnectionState` / `IceGatheringState` / `SignalingState` をそのまま使う (既存の `on_track` と同様に利用者は `shiguredo_webrtc` に依存する。sora-rust-sdk は re-export しない)
- SignalingState の公開は webrtc-rs 側の対応が先になる。webrtc-rs の対応が入り次第、この issue で SignalingState の通知まで対応して完了とする
- 既存の `on_connection_change` のログ出力は残す
- 状態に応じた接続の終了 (failed での切断など) はこの issue では行わない。通知だけを行う

## 完了条件

- PeerConnection / ICE / ICE gathering / SignalingState の状態変化が型付きで通知される (SignalingState は webrtc-rs 側の公開後)
- `run` の終了条件は変わらない

## 変更対象

- `src/connection_event_handler.rs` の `SoraConnectionEventHandler`
- `src/connection.rs` の `PcObserverHandler` と `run`
- `skills/sora-rust-sdk/SKILL.md` (イベント一覧の更新)
- `SignalingState` と `on_signaling_change` の公開は webrtc-rs 側で対応する

## 解決方法

- `src/connection_event_handler.rs`
  - `SoraConnectionEventHandler` に `on_signaling_state_change` / `on_connection_state_change` / `on_ice_connection_state_change` / `on_ice_gathering_state_change` を追加した
  - メソッド名は W3C の WebRTC のイベントハンドラ名 (`signalingstatechange` / `connectionstatechange` / `iceconnectionstatechange` / `icegatheringstatechange`) に揃えた
  - 状態の型は `shiguredo_webrtc` の `SignalingState` / `PeerConnectionState` / `IceConnectionState` / `IceGatheringState` をそのまま使う (re-export しない)
- `src/connection.rs`
  - `PcObserverHandler` に `on_signaling_change` / `on_standardized_ice_connection_change` / `on_ice_gathering_change` を実装し、`SoraEvent` の `SignalingChange` / `IceConnectionChange` / `IceGatheringChange` として送るようにした。既存の `on_connection_change` も `ConnectionChange` として送るようにした
  - `run` のイベント処理から上記 4 つのコールバックを呼ぶようにした。`run` の終了条件は変えていない
  - 既存の `on_connection_change` のログ出力は残した
  - 状態変化の通知に使うチャネルは `PcObserverHandler` と `DcObsHandler` で用途ごとに分けず、それぞれ `event_tx` 1 つにまとめた
- `src/connection.rs` の `#[cfg(test)]`
  - 実 libwebrtc の PeerConnection で offer を適用し、`SignalingState` の `HaveLocalOffer` と `Closed`、`IceGatheringState` の `Gathering` と `Complete` が通知されることを確認する
  - PeerConnection を 2 つ用意して loopback で接続し、candidate メッセージとして通知された ICE 候補を相互に渡して `IceConnectionState` の `Checking` と `Connected`、`PeerConnectionState` の `Connecting` と `Connected` が通知されることを確認する。テスト用の接続は m 行を持たないため、recvonly の音声 transceiver を追加してから offer を生成する
- `e2e-tests/src/test_connection.rs` / `e2e-tests/tests/connection_state.rs`
  - 実 Sora へ接続し、4 つのコールバックが通知されることを確認する
- `skills/sora-rust-sdk/SKILL.md` のイベント一覧と `CHANGES.md` の `## develop` を更新した
- `shiguredo_webrtc` を 0.154.1-canary.4 に更新した (`SignalingState` と `on_signaling_change` の追加に追随)
