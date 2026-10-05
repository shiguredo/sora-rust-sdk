# PeerConnection / ICE / ICE gathering / SignalingState の状態変化をイベントで通知する

- Created: 2026-10-05
- Completed: {YYYY-MM-DD}
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
