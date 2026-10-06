//! SoraConnection のイベントハンドラトレイト。
use shiguredo_webrtc::{
    IceConnectionState, IceGatheringState, PeerConnectionState, RtpReceiver, RtpTransceiver,
    SignalingState,
};

use crate::types::{SignalingDirection, SignalingType};

/// [SoraConnection](crate::SoraConnection) のイベントを受け取るトレイト。
///
/// 16 個のコールバックメソッドを集約し、ユーザーが自身の struct に状態を持たせて
/// `&mut self` で共有できるようにする。
/// 全メソッドにデフォルトの空実装を提供しており、必要なメソッドのみ
/// オーバーライドすればよい。
///
/// 実装型は [Send] を満たす必要がある。
/// [Sync] は不要（各コールバックは単一タスクから直列に呼ばれるため）。
pub trait SoraConnectionEventHandler: Send {
    /// シグナリングメッセージの送受信を監視する。
    ///
    /// Sora サーバーとの間でやり取りされる JSON メッセージの内容を、
    /// デバッグやログ記録のために取得できる。
    /// 第一引数はシグナリング経路（[SignalingType::WebSocket] または [SignalingType::DataChannel]）、
    /// 第二引数はメッセージの方向（[SignalingDirection::Sent] または [SignalingDirection::Received]）、
    /// 第三引数は JSON 文字列。
    fn on_signaling_message(
        &mut self,
        _signaling_type: SignalingType,
        _direction: SignalingDirection,
        _text: &str,
    ) {
    }

    /// シグナリング通知メッセージを受信したときに呼ばれる。
    ///
    /// 引数には Sora サーバーから送られてきた JSON 文字列が渡される。
    /// チャネル参加者の接続・切断・メタデータ変更などのイベント情報を受け取れる。
    fn on_notify(&mut self, _text: &str) {}

    /// プッシュ通知メッセージを受信したときに呼ばれる。
    ///
    /// 引数には Sora サーバーから送られてきた JSON 文字列が渡される。
    /// プッシュ API やシグナリング通知メタデータ拡張から送信された通知を受け取れる。
    fn on_push(&mut self, _text: &str) {}

    /// リモート参加者から映像または音声トラックを受信したときに呼ばれる。
    ///
    /// 引数には受信した [RtpTransceiver] が渡される。
    /// このトランシーバーからトラックや RTP 統計情報を取得できる。
    fn on_track(&mut self, _transceiver: RtpTransceiver) {}

    /// リモート参加者の映像または音声トラックが削除されたときに呼ばれる。
    ///
    /// 引数には削除されたトラックに対応する [RtpReceiver] が渡される。
    fn on_remove_track(&mut self, _receiver: RtpReceiver) {}

    /// ネゴシエーションの状態 ([SignalingState]) が変化したときに呼ばれる。
    ///
    /// W3C の `signalingstatechange` イベントに対応する。
    /// offer/answer の交換に伴う状態遷移を、Sora サーバーとの接続状態とは独立に取得できる。
    fn on_signaling_state_change(&mut self, _state: SignalingState) {}

    /// PeerConnection の接続状態 ([PeerConnectionState]) が変化したときに呼ばれる。
    ///
    /// W3C の `connectionstatechange` イベントに対応する。
    /// ICE と DTLS を合わせた接続状態で、`Failed` はメディア経路の復旧が見込めないこと、
    /// `Disconnected` は一時的な切断で復旧し得ることを表す。
    fn on_connection_state_change(&mut self, _state: PeerConnectionState) {}

    /// ICE の接続状態 ([IceConnectionState]) が変化したときに呼ばれる。
    ///
    /// W3C の `iceconnectionstatechange` イベントに対応する。
    /// ICE 単体の状態で、接続の失敗と切断を
    /// [on_connection_state_change](Self::on_connection_state_change) より細かく判別できる。
    fn on_ice_connection_state_change(&mut self, _state: IceConnectionState) {}

    /// ICE の候補収集の状態 ([IceGatheringState]) が変化したときに呼ばれる。
    ///
    /// W3C の `icegatheringstatechange` イベントに対応する。
    /// `Gathering` は候補の収集中、`Complete` は収集の完了を表す。
    fn on_ice_gathering_state_change(&mut self, _state: IceGatheringState) {}

    /// WebSocket シグナリングから DataChannel シグナリングへの切替が
    /// 完了したときに呼ばれる。
    fn on_switched(&mut self) {}

    /// WebSocket 接続がクローズされたときに呼ばれる。
    ///
    /// 第一引数はクローズコード（`Some(u16)`）または `None`（正常クローズ以外）、
    /// 第二引数はクローズ理由の文字列。
    fn on_websocket_close(&mut self, _code: Option<u16>, _reason: &str) {}

    /// `#` プレフィックス付きのユーザー定義 DataChannel ラベル経由で
    /// メッセージを受信したときに呼ばれる。
    ///
    /// 第一引数は DataChannel のラベル名（`#` プレフィックスを含む）、
    /// 第二引数は受信したバイナリデータ。
    /// 任意のアプリケーションデータを DataChannel 経由で送受信するために使う。
    fn on_message(&mut self, _label: &str, _data: &[u8]) {}

    /// Sora サーバーから DataChannel が作成されたときに呼ばれる。
    ///
    /// 引数には作成された DataChannel のラベル名が渡される。
    fn on_data_channel(&mut self, _label: &str) {}

    /// DataChannel が開かれたときに呼ばれる。
    ///
    /// 引数には開かれた DataChannel のラベル名が渡される。
    fn on_data_channel_open(&mut self, _label: &str) {}

    /// DataChannel 経由でメッセージを受信したときに呼ばれる。
    ///
    /// 第一引数は DataChannel のラベル名、第二引数は受信したバイナリデータ。
    fn on_data_channel_message(&mut self, _label: &str, _data: &[u8]) {}

    /// DataChannel が閉じられたときに呼ばれる。
    ///
    /// 引数には閉じられた DataChannel のラベル名が渡される。
    fn on_data_channel_close(&mut self, _label: &str) {}
}
