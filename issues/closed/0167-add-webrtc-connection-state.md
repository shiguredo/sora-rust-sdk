# WebRTC の接続状態 (PeerConnection / ICE / SignalingState) を通知できるようにする

- Created: 2026-10-04
- Completed: 2026-10-05
- Branch: feature/add-webrtc-connection-state
- Polished: {YYYY-MM-DD}

## 目的

PeerConnection / ICE / SignalingState の接続状態の変化を SDK の利用者が取得できるようにする。接続状態の表示、ICE 失敗と切断の判別、再接続の判断に使う。

## 現状

`SoraConnectionEventHandler` には WebRTC の接続状態を通知するメソッドが無く、`SoraConnectionHandle` にも状態アクセサが無い。

webrtc-rs の `PeerConnectionObserverHandler` には次のコールバックがある。

- `on_connection_change` (`PeerConnectionState`)
- `on_standardized_ice_connection_change` (`IceConnectionState`)
- `on_ice_gathering_change` (`IceGatheringState`)

sora_sdk は `on_connection_change` を実装しているが `rtc_log_info!` でログ出力するだけで、イベントハンドラにもハンドルにも渡していない。ICE 系のコールバックは実装していない。

SignalingState は webrtc-rs が公開していない。C ラッパーの `peer_connection_interface.cc` の `OnSignalingChange` が空実装で、`signaling_state` の getter も無い。このため sora-rust-sdk だけでは対応できない。

利用者は notify (`connection.created`) から Sora 接続の確立しか分からず、ICE の失敗と切断を区別できない。ICE の失敗は接続タイムアウトとしてしか観測できない。

observer はコア内部にあるため、ブリッジ側で回避する方法は無い。`get_stats` の transport stats から間接的に読める可能性はあるが、状態変化の通知はできない。

## 設計方針

`SoraConnectionEventHandler` に WebRTC の接続状態変化のコールバックを追加する。

- PeerConnection / ICE / ICE gathering の状態変化を型付きで通知する (webrtc-rs の既存コールバックから)
- 状態の型は shiguredo_webrtc の `PeerConnectionState` / `IceConnectionState` / `IceGatheringState` をそのまま使う (既存の `on_track` と同様に、利用者は shiguredo_webrtc に依存する。sora-rust-sdk は shiguredo_webrtc を re-export しない)
- SignalingState は webrtc-rs への公開 (コールバックまたは getter の追加) が先に必要。webrtc-rs 対応後に同じくイベントとして通知する
- 既存の `on_connection_change` のログ出力は残す
- 状態アクセサが必要な場合は `SoraConnectionHandle` に追加する

Sora 接続の確立・切断のイベント (別 issue) とは別に、WebRTC レベルの状態を通知する。

## 完了条件

- PeerConnection / ICE / ICE gathering の状態変化が型付きイベントで通知される
- ICE の失敗と切断を利用者が判別できる
- SignalingState は webrtc-rs への公開後に対応する (webrtc-rs 対応が完了するまでは対象外)

## 変更対象

- `src/connection_event_handler.rs` の `SoraConnectionEventHandler`
- `src/connection.rs` の `PcObserverHandler` (コールバックの実装) と `SoraConnection`
- shiguredo_webrtc の `PeerConnectionObserverHandler` / C ラッパー (SignalingState の公開。webrtc-rs 側)
- `skills/sora-rust-sdk/SKILL.md` (イベント一覧の更新)

## 解決方法

この issue は扱わないことにした。WebRTC の状態変化の通知と、その状態に応じた接続の終了を 1 つの issue に混ぜていたため、目的ごとに分割して別 issue に移した。

- WebRTC の状態変化の通知 (PeerConnection / ICE / ICE gathering / SignalingState) は別 issue で扱う
- 接続確立後に PeerConnection が失敗した場合の接続の終了は別 issue で扱う

SignalingState は webrtc-rs が公開していないため、webrtc-rs の issue を起票して対応する。sora-rust-sdk 側の通知はその対応後になる。
