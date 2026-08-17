//! SoraConnection 本体と接続制御の実装。
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_lc_rs::rand::{SecureRandom as AwsSecureRandom, SystemRandom};
use nojson::Json;
use rustls::ClientConfig;
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, ServerName, TrustAnchor, UnixTime, pem::PemObject,
};
use rustls_platform_verifier::ConfigVerifierExt;
use shiguredo_http11::{Request, ResponseDecoder, auth::BasicAuth, uri::Uri};
use shiguredo_webrtc::{
    AudioTrack, CreateSessionDescriptionObserver, CreateSessionDescriptionObserverHandler,
    CxxString, DataChannel, DataChannelObserver, DataChannelObserverHandler, DataChannelState,
    IceCandidateRef, IceServer, MediaStreamTrack, PeerConnection, PeerConnectionDependencies,
    PeerConnectionObserver, PeerConnectionObserverHandler, PeerConnectionOfferAnswerOptions,
    PeerConnectionRtcConfiguration, PeerConnectionState, RTCStatsReport, Resolution, RtcError,
    RtpEncodingParameters, RtpEncodingParametersVector, RtpReceiver, RtpSender, RtpTransceiver,
    SSLCertChainRef, SSLCertificateVerifier, SSLCertificateVerifierHandler, SdpType,
    SessionDescription, SetLocalDescriptionObserver, SetLocalDescriptionObserverHandler,
    SetRemoteDescriptionObserver, SetRemoteDescriptionObserverHandler, StringVector, TlsCertPolicy,
    VideoTrack,
};
use shiguredo_websocket::{
    ClientConnectionOptions, CloseCode, ConnectionEvent, ConnectionOutput, ConnectionState,
    RandomSource, TimerId, Timestamp, WebSocketClientConnection,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_rustls::{TlsConnector, client::TlsStream};

use crate::connection_context::SoraConnectionContext;
use crate::connection_event_handler::SoraConnectionEventHandler;
use crate::error::{Error, Result};
use crate::rpc::{self, RpcRequestOptions, RpcResponse};
use crate::signaling_types::{
    DataChannelConfig, IceServerConfig, IncomingMessage, IncomingMessageData, OutgoingMessage,
    SimulcastEncodingConfig,
};
use crate::types::{
    Audio, ConnectDataChannel, ForwardingFilter, JsonString, ProxyInfo, Role, SignalingDirection,
    SignalingType, Video,
};
use crate::zlib::{compress_zlib, decompress_zlib};
use shiguredo_webrtc::{rtc_log_error, rtc_log_info, rtc_log_verbose, rtc_log_warning};

/// WebSocket (シグナリング接続) の TLS 設定。
///
/// TURN-TLS の TLS 設定は `SoraConnectionBuilder::turn_tls_insecure()` / `turn_tls_ca_cert()` で行う。
#[derive(Clone, Default)]
pub(crate) struct TlsConfig {
    /// サーバー証明書の検証をスキップする。
    pub(crate) insecure: bool,
    /// クライアント証明書 (PEM 形式)。
    pub(crate) client_cert: Option<String>,
    /// クライアント秘密鍵 (PEM 形式)。
    pub(crate) client_key: Option<String>,
    /// CA 証明書 (PEM 形式)。
    pub(crate) ca_cert: Option<String>,
}

type IceServerUrlConfigurer = dyn Fn(&mut IceServer, &[String]) + Send;

/// [SoraConnection] を構築するためのビルダー。
///
/// [SoraConnection::builder()] で生成し、各種設定メソッドでパラメータを指定した後、
/// [build()](Self::build) で [SoraConnection] と [SoraConnectionHandle] を生成する。
/// イベントハンドラは [SoraConnection::builder()] の第 5 引数として
/// [SoraConnectionEventHandler] 実装を渡す。
pub struct SoraConnectionBuilder {
    signaling_urls: Vec<String>,
    channel_id: String,
    role: Role,

    event_handler: Option<Box<dyn SoraConnectionEventHandler + Send>>,
    sender_video_track: Option<VideoTrack>,
    sender_audio_track: Option<AudioTrack>,

    // connect 時の設定
    client_id: Option<String>,
    bundle_id: Option<String>,
    metadata: Option<JsonString>,
    audio: Option<Audio>,
    video: Option<Video>,
    data_channel_signaling: Option<bool>,
    ignore_disconnect_websocket: Option<bool>,
    simulcast: Option<bool>,
    simulcast_request_rid: Option<String>,
    spotlight: Option<bool>,
    spotlight_focus_rid: Option<String>,
    spotlight_unfocus_rid: Option<String>,
    signaling_notify_metadata: Option<JsonString>,
    data_channels: Option<Vec<ConnectDataChannel>>,
    forwarding_filters: Option<Vec<ForwardingFilter>>,
    turn_tls_insecure: bool,
    turn_tls_ca_cert: Option<Vec<u8>>,
    ice_server_url_configurer: Option<Box<IceServerUrlConfigurer>>,
    proxy: Option<ProxyInfo>,
    websocket_connection_timeout: Duration,
    websocket_close_timeout: Duration,
    disconnect_wait_timeout: Duration,
    tls_config: TlsConfig,
    user_agent: Option<String>,
    // 他の保持オブジェクトより最後に破棄する必要がある。
    context: Arc<SoraConnectionContext>,
}

impl SoraConnectionBuilder {
    fn new(
        context: Arc<SoraConnectionContext>,
        signaling_urls: Vec<String>,
        channel_id: String,
        role: Role,
        event_handler: Box<dyn SoraConnectionEventHandler + Send>,
    ) -> Self {
        Self {
            signaling_urls,
            channel_id,
            role,
            event_handler: Some(event_handler),
            sender_video_track: None,
            sender_audio_track: None,
            client_id: None,
            bundle_id: None,
            metadata: None,
            audio: None,
            video: None,
            data_channel_signaling: None,
            ignore_disconnect_websocket: None,
            simulcast: None,
            simulcast_request_rid: None,
            spotlight: None,
            spotlight_focus_rid: None,
            spotlight_unfocus_rid: None,
            signaling_notify_metadata: None,
            data_channels: None,
            forwarding_filters: None,
            turn_tls_insecure: false,
            turn_tls_ca_cert: None,
            ice_server_url_configurer: None,
            proxy: None,
            websocket_connection_timeout: Duration::from_secs(30),
            websocket_close_timeout: Duration::from_secs(3),
            disconnect_wait_timeout: Duration::from_secs(5),
            tls_config: TlsConfig::default(),
            user_agent: None,
            context,
        }
    }

    /// Sora サーバーに送信する映像トラックを設定する。
    ///
    /// [SoraConnectionContext::create_video_track] などで生成した
    /// [VideoTrack] を渡す。
    pub fn sender_video_track(mut self, track: VideoTrack) -> Self {
        self.sender_video_track = Some(track);
        self
    }

    /// Sora サーバーに送信する音声トラックを設定する。
    ///
    /// [SoraConnectionContext::create_audio_track] などで生成した
    /// [AudioTrack] を渡す。
    pub fn sender_audio_track(mut self, track: AudioTrack) -> Self {
        self.sender_audio_track = Some(track);
        self
    }

    /// Sora サーバーに送信するクライアント ID を 1〜255 バイトの任意の文字列で指定する。
    ///
    /// 指定しない場合は Sora サーバーによって接続 ID が自動的に割り当てられる。
    /// 同一セッション内で同じクライアント ID を持つ接続が存在する場合、
    /// 設定によっては既存の接続が追い出されることがある。
    pub fn client_id(mut self, client_id: String) -> Self {
        self.client_id = Some(client_id);
        self
    }

    /// Sora サーバーに送信するバンドル ID を 1〜255 バイトの任意の文字列で指定する。
    ///
    /// マルチストリーム利用時に、同じバンドル ID を持つ接続からの
    /// 音声・映像・メッセージングを受信しなくなる。
    pub fn bundle_id(mut self, bundle_id: String) -> Self {
        self.bundle_id = Some(bundle_id);
        self
    }

    /// Sora サーバーに送信する認証メタデータを [JsonString] で指定する。
    ///
    /// この値は Sora サーバーの認証ウェブフックに渡される。
    /// 接続の認証やセッション管理に利用できる任意の JSON データを設定する。
    pub fn metadata(mut self, metadata: JsonString) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// 配信または受信する音声メディアの設定を指定する。
    ///
    /// [Audio::new_bool] で単純な有効/無効を、
    /// [Audio::new_opus] で Opus のビットレートやパラメータを指定できる。
    /// role が [Role::SendRecv] または [Role::SendOnly] の場合は配信設定として、
    /// [Role::RecvOnly] の場合は受信設定として扱われる。
    pub fn audio(mut self, audio: Audio) -> Self {
        self.audio = Some(audio);
        self
    }

    /// 配信または受信する映像メディアの設定を指定する。
    ///
    /// [Video::new_bool] で単純な有効/無効を、
    /// [Video::new_vp8] / [Video::new_vp9] / [Video::new_av1] /
    /// [Video::new_h264] / [Video::new_h265] でコーデックやビットレートを指定できる。
    /// role が [Role::SendRecv] または [Role::SendOnly] の場合は配信設定として、
    /// [Role::RecvOnly] の場合は受信設定として扱われる。
    pub fn video(mut self, video: Video) -> Self {
        self.video = Some(video);
        self
    }

    /// WebSocket でのシグナリング確立後に、
    /// 後続のシグナリングを DataChannel に切り替えるかどうかを指定する。
    ///
    /// true にすると、offer/answer 交換以降のシグナリングメッセージが
    /// WebSocket ではなく DataChannel 経由でやり取りされる。
    pub fn data_channel_signaling(mut self, value: bool) -> Self {
        self.data_channel_signaling = Some(value);
        self
    }

    /// DataChannel シグナリングへの切替後に WebSocket が切断された場合でも、
    /// DataChannel での通信を継続するかどうかを指定する。
    ///
    /// true にすると、WebSocket 接続が切れても
    /// DataChannel 経由のシグナリングが継続される。
    pub fn ignore_disconnect_websocket(mut self, value: bool) -> Self {
        self.ignore_disconnect_websocket = Some(value);
        self
    }

    /// サイマルキャストを有効にするかどうかを指定する。
    ///
    /// true にすると、配信映像が複数の品質（解像度・ビットレート）の
    /// ストリームに分割され、受信側が自分の帯域に合わせて
    /// 最適な品質のストリームを選択できるようになる。
    pub fn simulcast(mut self, value: bool) -> Self {
        self.simulcast = Some(value);
        self
    }

    /// サイマルキャスト利用時に受信側として要求する rid を指定する。
    ///
    /// rid は受信したい品質レイヤーを識別する文字列で、
    /// "r0"（低品質）、"r1"（中品質）、"r2"（高品質）などが使われる。
    pub fn simulcast_request_rid(mut self, value: String) -> Self {
        self.simulcast_request_rid = Some(value);
        self
    }

    /// スポットライト機能を有効にするかどうかを指定する。
    ///
    /// true にすると、現在話している参加者にフォーカスが当たり、
    /// フォーカスされた参加者の映像は高画質・音声は高音質で配信され、
    /// それ以外の参加者の映像は低画質に制限される。
    /// これにより視聴側の負荷が軽減される。
    /// [simulcast](Self::simulcast) が有効であることが前提。
    pub fn spotlight(mut self, value: bool) -> Self {
        self.spotlight = Some(value);
        self
    }

    /// スポットライトでフォーカスされた参加者の映像を受信する際の rid を指定する。
    ///
    /// この値で指定された品質レイヤーで、フォーカス参加者の映像を受信する。
    pub fn spotlight_focus_rid(mut self, value: String) -> Self {
        self.spotlight_focus_rid = Some(value);
        self
    }

    /// スポットライトでフォーカスされていない参加者の映像を受信する際の rid を指定する。
    ///
    /// この値で指定された品質レイヤーで、非フォーカス参加者の映像を受信する。
    pub fn spotlight_unfocus_rid(mut self, value: String) -> Self {
        self.spotlight_unfocus_rid = Some(value);
        self
    }

    /// シグナリング通知メタデータを [JsonString] で指定する。
    ///
    /// この値はシグナリング通知に含めてチャネル内の全参加者にブロードキャストされる。
    pub fn signaling_notify_metadata(mut self, value: JsonString) -> Self {
        self.signaling_notify_metadata = Some(value);
        self
    }

    /// 接続時に作成する追加の DataChannel の設定を指定する。
    ///
    /// 各 [ConnectDataChannel] でラベル、方向、圧縮の有無などを指定できる。
    /// ここで指定した DataChannel は WebRTC 接続確立時に自動的に作成される。
    pub fn data_channels(mut self, value: Vec<ConnectDataChannel>) -> Self {
        self.data_channels = Some(value);
        self
    }

    /// 接続時に適用する転送フィルターのリストを指定する。
    ///
    /// 転送フィルターを使うと、条件に合致する他参加者の音声や映像の受信を
    /// ブロックできる。各 [ForwardingFilter] は [ForwardingFilterRule](crate::types::ForwardingFilterRule) の
    /// リストで構成され、ルールには対象フィールド（kind 等）・
    /// 演算子（is_in 等）・値を指定する。
    /// 接続後は [SoraConnectionHandle::send_rpc_request] で
    /// DataChannel 経由にフィルターを変更することも可能。
    pub fn forwarding_filters(mut self, value: Vec<ForwardingFilter>) -> Self {
        self.forwarding_filters = Some(value);
        self
    }

    /// Sora サーバーから通知された ICE サーバー設定をカスタマイズする
    /// コールバックを設定する。
    ///
    /// コールバックの第一引数は設定対象の [IceServer]、
    /// 第二引数は Sora から通知された URL のリスト。
    /// 指定しない場合は通知された全 URL がそのまま ICE サーバーに追加される。
    /// 特定のプロトコル（TCP/UDP）のみを使うなどのフィルタリングに利用できる。
    pub fn ice_server_url_configurer<F>(mut self, configurer: F) -> Self
    where
        F: Fn(&mut IceServer, &[String]) + Send + 'static,
    {
        self.ice_server_url_configurer = Some(Box::new(configurer));
        self
    }

    /// シグナリング接続時に経由する HTTP プロキシを設定する。
    ///
    /// HTTP CONNECT メソッドを使ってプロキシサーバー経由で
    /// Sora サーバーに接続する。プロキシ認証が必要な場合は
    /// [ProxyInfo] の username / password フィールドで指定する。
    pub fn proxy(mut self, proxy: ProxyInfo) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// WebSocket 接続が確立するまでの待機時間の上限を設定する。
    ///
    /// この時間内に WebSocket 接続が確立できなかった場合、
    /// 残りのシグナリング URL への接続が試行される。
    pub fn websocket_connection_timeout(mut self, value: Duration) -> Self {
        self.websocket_connection_timeout = value;
        self
    }

    /// 切断時に WebSocket のクローズハンドシェイクを待機する時間の上限を設定する。
    ///
    /// この時間を超えると WebSocket のクローズ完了を待たずに処理を打ち切る。
    pub fn websocket_close_timeout(mut self, value: Duration) -> Self {
        self.websocket_close_timeout = value;
        self
    }

    /// 切断時に DataChannel がすべて閉じられるまでの待機時間の上限を設定する。
    ///
    /// DataChannel シグナリング利用時に、切断処理の一部として
    /// 開いている全 DataChannel のクローズを待機する。
    pub fn disconnect_wait_timeout(mut self, value: Duration) -> Self {
        self.disconnect_wait_timeout = value;
        self
    }

    /// シグナリング用 WebSocket（WSS）接続時のサーバー証明書検証を
    /// スキップするかどうかを指定する。
    ///
    /// true にすると、自己署名証明書や検証不能な証明書を使った
    /// Sora サーバーにも接続できるようになる。
    /// 本番環境ではセキュリティ上の理由から false のまま使うことを推奨する。
    pub fn insecure(mut self, value: bool) -> Self {
        self.tls_config.insecure = value;
        self
    }

    /// TURN-TLS 接続時のサーバー証明書検証をスキップするかどうかを指定する。
    ///
    /// true にすると、自己署名証明書や検証不能な証明書を使った
    /// TURN サーバーにも接続できるようになる。
    /// 本番環境ではセキュリティ上の理由から false のまま使うことを推奨する。
    pub fn turn_tls_insecure(mut self, value: bool) -> Self {
        self.turn_tls_insecure = value;
        self
    }

    /// シグナリング用 WebSocket（WSS）接続時に使用する CA 証明書を
    /// PEM 形式で設定する。
    ///
    /// プライベート CA で署名された Sora サーバー証明書を検証するために使う。
    pub fn ca_cert(mut self, cert: String) -> Self {
        self.tls_config.ca_cert = Some(cert);
        self
    }

    /// TURN-TLS 接続時に使用する CA 証明書を DER エンコードのバイト列で設定する。
    ///
    /// プライベート CA で署名された TURN サーバー証明書を検証するために使う。
    pub fn turn_tls_ca_cert(mut self, der: Vec<u8>) -> Self {
        self.turn_tls_ca_cert = Some(der);
        self
    }

    /// シグナリング用 WebSocket（WSS）接続時に使用する
    /// クライアント証明書と秘密鍵を PEM 形式で設定する。
    ///
    /// クライアント証明書による認証が必要な Sora サーバーに接続する際に使う。
    /// cert と key は必ず両方を指定する必要がある。
    pub fn client_cert(mut self, cert: String, key: String) -> Self {
        self.tls_config.client_cert = Some(cert);
        self.tls_config.client_key = Some(key);
        self
    }

    /// WebSocket 接続時の User-Agent ヘッダ値を設定する。
    ///
    /// この値は Sora サーバーのログや認証ウェブフックに記録される。
    pub fn user_agent(mut self, value: String) -> Self {
        self.user_agent = Some(value);
        self
    }

    /// ビルダーを消費して [SoraConnection] と [SoraConnectionHandle] を生成する。
    ///
    /// 生成された [SoraConnection] は [run](SoraConnection::run) を
    /// 別タスクで実行することで接続を開始する。
    /// [SoraConnectionHandle] は接続の切断や統計情報の取得に使用する。
    pub fn build(self) -> Result<(SoraConnection, SoraConnectionHandle)> {
        SoraConnection::new(self)
    }
}

/// 外部から SoraConnection を制御するためのハンドル。
///
/// `SoraConnection::run()` を別タスクで実行中に、このハンドルを使って
/// 接続の切断や統計情報の取得を行うことができる。
#[derive(Clone)]
pub struct SoraConnectionHandle {
    command_tx: mpsc::UnboundedSender<SoraConnectionCommand>,
}

impl SoraConnectionHandle {
    /// 最初に WebSocket 接続が成功したシグナリング URL を返す。
    ///
    /// `run()` で接続が確立される前は `None` を返す。
    /// `run()` が終了している場合はエラーを返す。
    pub async fn selected_signaling_url(&self) -> Result<Option<String>> {
        self.send_command(
            "selected_signaling_url",
            None,
            SoraConnectionCommand::GetSelectedSignalingUrl,
        )
        .await
    }

    /// 現在接続中のシグナリング URL を返す。
    ///
    /// リダイレクト後はリダイレクト先の URL を返す。
    /// `run()` で接続が確立される前は `None` を返す。
    /// `run()` が終了している場合はエラーを返す。
    pub async fn connected_signaling_url(&self) -> Result<Option<String>> {
        self.send_command(
            "connected_signaling_url",
            None,
            SoraConnectionCommand::GetConnectedSignalingUrl,
        )
        .await
    }

    /// 接続を切断する。
    ///
    /// `run()` が切断要求を受け付けたことを確認してから戻る。
    pub async fn disconnect(&self) -> Result<()> {
        self.send_command("disconnect", None, SoraConnectionCommand::Disconnect)
            .await
    }

    /// DataChannel 経由で JSON-RPC 2.0 リクエストを送信する。
    ///
    /// SDK 内部で JSON-RPC 2.0 メッセージを組み立てて送信する。
    /// `options.notification` が `true` の場合はレスポンスを待たずに即座に `Ok(None)` を返す。
    /// `options.timeout` でレスポンスの待機タイムアウトを指定する (デフォルト 5 秒)。
    ///
    /// 返り値:
    /// - `Ok(Some(RpcResponse::Success { result }))`: 正規の success response
    /// - `Ok(Some(RpcResponse::Error { code, message, data }))`: 正規の remote error response
    /// - `Err(Error::RpcProtocolViolation)`: JSON-RPC 2.0 の要件を満たさない応答で、
    ///   本 SDK が生成した Request ID と相関できる id を持つもの
    /// - `Err(Error::RpcTimeout)`: `options.timeout` の経過
    /// - `Ok(None)`: notification (`options.notification` が `true` の場合)
    pub async fn send_rpc_request(
        &self,
        method: &str,
        params: Option<JsonString>,
        options: RpcRequestOptions,
    ) -> Result<Option<RpcResponse>> {
        self.send_command("send_rpc_request", None, |tx| {
            SoraConnectionCommand::SendRpcRequest {
                method: method.to_string(),
                params,
                notification: options.notification,
                timeout: options.timeout,
                response_tx: tx,
            }
        })
        .await?
    }

    /// DataChannel 経由でメッセージを送信する。
    ///
    /// SDK 内部用ラベル（`signaling`、`stats`、`push`、`notify`、`rpc`）および
    /// `#` プレフィックスのないラベルを渡すと `Error::InvalidDataChannelLabel` を返す。
    /// また、Offer 応答の `data_channels` に含まれていないラベルも同様に返す。
    pub async fn send_message(&self, label: &str, data: &[u8]) -> Result<()> {
        self.send_command("send_message", None, |tx| {
            SoraConnectionCommand::SendMessage {
                label: label.to_string(),
                data: data.to_vec(),
                response_tx: tx,
            }
        })
        .await?
    }

    /// 統計情報を取得する。
    ///
    /// PeerConnection の統計情報を JSON 形式で取得する。
    /// 統計コールバックが 5 秒以内に発火しない場合は `Error::CommandTimeout` を返す。
    pub async fn get_stats(&self) -> Result<JsonString> {
        self.send_command(
            "get_stats",
            Some(Duration::from_secs(5)),
            SoraConnectionCommand::GetStats,
        )
        .await?
    }

    async fn send_command<R>(
        &self,
        command: &'static str,
        timeout: Option<Duration>,
        build: impl FnOnce(oneshot::Sender<R>) -> SoraConnectionCommand,
    ) -> Result<R> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(build(tx))
            .map_err(|e| Error::CommandSendFailed {
                reason: e.to_string(),
                command,
            })?;
        let result = match timeout {
            Some(duration) => match tokio::time::timeout(duration, rx).await {
                Ok(result) => result,
                Err(_) => {
                    rtc_log_warning!("Timed out waiting for {} result", command);
                    return Err(Error::CommandTimeout { command });
                }
            },
            None => rx.await,
        };
        result.map_err(|source| Error::CommandResponseMissing { source, command })
    }
}

/// Sora サーバーとの WebRTC 接続を管理する本体。
///
/// [SoraConnectionBuilder] の [build()](SoraConnectionBuilder::build) で生成し、
/// [run()](Self::run) でメインループを開始する。
/// 別タスクから [SoraConnectionHandle] 経由で制御する。
pub struct SoraConnection {
    data_channels: HashMap<String, ManagedDataChannel>,
    data_channel_configs: Vec<DataChannelConfig>,
    offer_simulcast: bool,
    simulcast_encodings: Vec<SimulcastEncodingConfig>,
    video_sender: Option<RtpSender>,
    command_rx: mpsc::UnboundedReceiver<SoraConnectionCommand>,
    event_tx: mpsc::UnboundedSender<SoraEvent>,
    event_rx: mpsc::UnboundedReceiver<SoraEvent>,
    pending_rpc_responses: HashMap<u64, PendingRpcRequest>,
    rpc_id_counter: u64,
    proxy: Option<ParsedProxyInfo>,
    selected_signaling_url: Option<String>,
    connected_signaling_url: Option<String>,
    // 依存するオブジェクトを先に破棄するため、PeerConnection は後ろに保持する。
    pc: PeerConnection,
    // Observer を保持しておく必要がある (ドロップすると PeerConnection への通知が止まる)。
    // PeerConnection の破棄までは生存させるため、pc の後に保持する。
    #[expect(dead_code)]
    pc_observer: PeerConnectionObserver,
    // context を最後に破棄するため、config は最後に保持する。
    config: SoraConnectionBuilder,
}

struct PendingRpcRequest {
    response_tx: Option<oneshot::Sender<Result<Option<RpcResponse>>>>,
    timeout_handle: JoinHandle<()>,
}

impl Drop for PendingRpcRequest {
    fn drop(&mut self) {
        self.timeout_handle.abort();
    }
}

enum SoraEvent {
    Track(RtpTransceiver),
    RemoveTrack(RtpReceiver),
    SignalingMessage(String),
    DataChannelMessage { label: String, data: Vec<u8> },
    DataChannelRegister(DataChannel),
    DataChannelStateChange(String),
    SendWebSocketMessage(String),
    SendDataChannelMessage { label: String, message: String },
    RpcTimeout { id: u64 },
}

pub(crate) enum SoraConnectionCommand {
    Disconnect(oneshot::Sender<()>),
    GetStats(oneshot::Sender<Result<JsonString>>),
    GetSelectedSignalingUrl(oneshot::Sender<Option<String>>),
    GetConnectedSignalingUrl(oneshot::Sender<Option<String>>),
    SendRpcRequest {
        method: String,
        params: Option<JsonString>,
        notification: bool,
        timeout: Duration,
        response_tx: oneshot::Sender<Result<Option<RpcResponse>>>,
    },
    SendMessage {
        label: String,
        data: Vec<u8>,
        response_tx: oneshot::Sender<Result<()>>,
    },
}

struct TurnTlsCaCertVerifier {
    trust_anchors: Vec<TrustAnchor<'static>>,
}

impl SSLCertificateVerifierHandler for TurnTlsCaCertVerifier {
    fn verify_chain(&mut self, chain: SSLCertChainRef<'_>) -> bool {
        if chain.is_empty() {
            return false;
        }

        let Some(ee_cert_ref) = chain.get(0) else {
            return false;
        };
        let ee_der = ee_cert_ref.to_der();
        let ee_cert_der = CertificateDer::from(ee_der);

        let intermediates: Vec<CertificateDer<'_>> = (1..chain.len())
            .filter_map(|i| chain.get(i))
            .map(|cert| CertificateDer::from(cert.to_der()))
            .collect();

        let Ok(ee) = webpki::EndEntityCert::try_from(&ee_cert_der) else {
            return false;
        };

        let time = UnixTime::now();

        ee.verify_for_usage(
            webpki::ALL_VERIFICATION_ALGS,
            &self.trust_anchors,
            &intermediates,
            time,
            webpki::KeyUsage::server_auth(),
            None,
            None,
        )
        .is_ok()
    }
}

/// `RTCStatsReport` を `JsonString` に変換する。
///
/// `to_json()` で得た文字列を、JSON として妥当であることを検証した `JsonString` にパースする。
fn report_to_json_string(report: &RTCStatsReport) -> Result<JsonString> {
    report
        .to_json()
        .map_err(Error::from)
        .and_then(|s| s.parse())
}

/// SDP 処理 (set remote/local description / create answer) のタイムアウト。
const SDP_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// SetRemoteDescription / SetLocalDescription の完了通知を受け取る Observer。
///
/// どちらの操作も「エラー文字列の `Option` をチャネルで返す」点で同じ形をしているため、
/// 1 つの構造体で 2 つのトレイトを実装して共有する。
struct SetDescriptionObserverHandler {
    tx: mpsc::UnboundedSender<Option<String>>,
}

impl SetRemoteDescriptionObserverHandler for SetDescriptionObserverHandler {
    fn on_set_remote_description_complete(&mut self, error: RtcError) {
        send_set_description_result(&self.tx, error);
    }
}

impl SetLocalDescriptionObserverHandler for SetDescriptionObserverHandler {
    fn on_set_local_description_complete(&mut self, error: RtcError) {
        send_set_description_result(&self.tx, error);
    }
}

/// SDP 適用の完了結果をエラー文字列の `Option` に変換して送信する。
fn send_set_description_result(tx: &mpsc::UnboundedSender<Option<String>>, error: RtcError) {
    let msg = if error.ok() {
        None
    } else {
        Some(error.message().unwrap_or_else(|_| "unknown".to_string()))
    };
    let _ = tx.send(msg);
}

impl SoraConnection {
    /// [SoraConnectionBuilder] を生成する。
    ///
    /// `event_handler` には [SoraConnectionEventHandler] トレイトを実装した型のインスタンスを渡す。
    /// トレイトの全メソッドにデフォルトの空実装が用意されているため、
    /// 必要なメソッドのみオーバーライドすればよい。
    pub fn builder(
        context: Arc<SoraConnectionContext>,
        signaling_urls: Vec<String>,
        channel_id: String,
        role: Role,
        event_handler: impl SoraConnectionEventHandler + 'static,
    ) -> SoraConnectionBuilder {
        SoraConnectionBuilder::new(
            context,
            signaling_urls,
            channel_id,
            role,
            Box::new(event_handler),
        )
    }

    fn new(config: SoraConnectionBuilder) -> Result<(Self, SoraConnectionHandle)> {
        let (command_tx, command_rx) = mpsc::unbounded_channel::<SoraConnectionCommand>();
        let handle = SoraConnectionHandle { command_tx };

        let (event_tx, event_rx) = mpsc::unbounded_channel::<SoraEvent>();
        let pc_factory = config.context.factory();
        let connection_context = config.context.connection_context();
        let event_tx_for_candidate = event_tx.clone();
        let event_tx_for_channel = event_tx.clone();
        let event_tx_for_track = event_tx.clone();
        struct PcObserverHandler {
            event_tx_for_track: mpsc::UnboundedSender<SoraEvent>,
            event_tx_for_candidate: mpsc::UnboundedSender<SoraEvent>,
            event_tx_for_channel: mpsc::UnboundedSender<SoraEvent>,
        }

        impl PeerConnectionObserverHandler for PcObserverHandler {
            fn on_connection_change(&mut self, new_state: PeerConnectionState) {
                rtc_log_info!("PeerConnection state: {:?}", new_state);
            }

            fn on_track(&mut self, transceiver: RtpTransceiver) {
                let _ = self.event_tx_for_track.send(SoraEvent::Track(transceiver));
            }

            fn on_remove_track(&mut self, receiver: RtpReceiver) {
                let _ = self
                    .event_tx_for_track
                    .send(SoraEvent::RemoveTrack(receiver));
            }

            fn on_ice_candidate(&mut self, candidate: IceCandidateRef<'_>) {
                if let Ok(message) = candidate.to_string() {
                    let candidate_message = OutgoingMessage::new_candidate(&message);
                    let _ = self
                        .event_tx_for_candidate
                        .send(SoraEvent::SignalingMessage(
                            Json(candidate_message).to_string(),
                        ));
                }
            }

            fn on_data_channel(&mut self, channel: DataChannel) {
                // DataChannel を登録するためにメインループに送信
                let _ = self
                    .event_tx_for_channel
                    .send(SoraEvent::DataChannelRegister(channel));
            }
        }

        let observer = PeerConnectionObserver::new_with_handler(Box::new(PcObserverHandler {
            event_tx_for_track,
            event_tx_for_candidate,
            event_tx_for_channel,
        }));

        let mut deps = PeerConnectionDependencies::new(&observer);
        if let Some(ca_cert_der) = &config.turn_tls_ca_cert {
            let ca_cert = CertificateDer::from(ca_cert_der.as_slice());
            let anchor = webpki::anchor_from_trusted_cert(&ca_cert)
                .map_err(|e| Error::TurnTlsCaCert {
                    message: format!("{}", e),
                })?
                .to_owned();
            let verifier =
                SSLCertificateVerifier::new_with_handler(Box::new(TurnTlsCaCertVerifier {
                    trust_anchors: vec![anchor],
                }));
            deps.set_tls_cert_verifier(verifier);
        }
        let proxy = config
            .proxy
            .as_ref()
            .map(ParsedProxyInfo::parse)
            .transpose()?;
        if let Some(proxy) = proxy.as_ref() {
            let network_manager = connection_context.default_network_manager();
            let socket_factory = connection_context.default_socket_factory();
            deps.set_proxy(
                network_manager,
                socket_factory,
                proxy.host(),
                proxy.port(),
                proxy.username().unwrap_or(""),
                proxy.password().unwrap_or(""),
                proxy.user_agent(),
            );
        }
        let mut rtc_config = PeerConnectionRtcConfiguration::new();
        let pc = PeerConnection::create(pc_factory, &mut rtc_config, &mut deps)?;

        let connection = Self {
            data_channels: HashMap::new(),
            data_channel_configs: Vec::new(),
            offer_simulcast: false,
            simulcast_encodings: Vec::new(),
            video_sender: None,
            command_rx,
            event_tx,
            event_rx,
            pending_rpc_responses: HashMap::new(),
            rpc_id_counter: 0,
            proxy,
            selected_signaling_url: None,
            connected_signaling_url: None,
            pc,
            pc_observer: observer,
            config,
        };
        Ok((connection, handle))
    }

    /// メインループを開始する。
    ///
    /// シグナリング接続を確立し、WebRTC ネゴシエーションを処理し、受信メッセージを処理する。
    /// 接続が終了するまでブロックするため、別の非同期タスクで呼び出すこと。
    pub async fn run(mut self) -> Result<()> {
        let signaling_urls = self.config.signaling_urls.clone();
        let channel_id = self.config.channel_id.clone();
        let role = self.config.role;
        let client_id = self.config.client_id.clone();
        let bundle_id = self.config.bundle_id.clone();
        let sora_client = crate::version::get_sora_client_name();
        let libwebrtc = crate::version::get_libwebrtc_name();
        let environment = crate::version::get_environment_name();
        let metadata = self.config.metadata.clone();
        let data_channel_signaling = self.config.data_channel_signaling;
        let ignore_disconnect_websocket = self.config.ignore_disconnect_websocket;
        let simulcast = self.config.simulcast;
        let simulcast_request_rid = self.config.simulcast_request_rid.clone();
        let spotlight = self.config.spotlight;
        let spotlight_focus_rid = self.config.spotlight_focus_rid.clone();
        let spotlight_unfocus_rid = self.config.spotlight_unfocus_rid.clone();
        let signaling_notify_metadata = self.config.signaling_notify_metadata.clone();
        let data_channels = self.config.data_channels.clone();
        let forwarding_filters = self.config.forwarding_filters.clone();
        let tls_config = Arc::new(self.config.tls_config.clone());
        let audio = self.config.audio.clone();
        let video = self.config.video.clone();
        let mut handler = self
            .config
            .event_handler
            .take()
            .expect("event_handler must be set in new()");
        let proxy = self.proxy.clone();

        if signaling_urls.is_empty() {
            return Err(Error::SignalingUrlsEmpty);
        }

        // URL リストをランダム化して負荷分散する (Fisher-Yates シャッフル)
        let mut urls = signaling_urls.clone();
        if urls.len() > 1 {
            let rng = SystemRandom::new();
            for i in (1..urls.len()).rev() {
                let mut buf = [0u8; 8];
                rng.fill(&mut buf)
                    .expect("failed to generate random bytes for URL shuffle");
                let j = (u64::from_le_bytes(buf) % (i as u64 + 1)) as usize;
                urls.swap(i, j);
            }
        }

        let websocket_connection_timeout = self.config.websocket_connection_timeout;
        let websocket_close_timeout = self.config.websocket_close_timeout;
        let disconnect_wait_timeout = self.config.disconnect_wait_timeout;
        let user_agent = self
            .config
            .user_agent
            .clone()
            .unwrap_or_else(crate::version::get_sora_client_name);

        let (mut stream, target, selected_url) = connect_signaling_urls(
            &urls,
            proxy.clone(),
            tls_config.clone(),
            websocket_connection_timeout,
        )
        .await?;

        // 選択された URL を記録する
        self.selected_signaling_url = Some(selected_url.clone());
        self.connected_signaling_url = Some(selected_url);

        let event_tx = self.event_tx.clone();

        let display_host = format_bracketed_host(&target.host);
        let scheme = if target.tls { "wss" } else { "ws" };
        rtc_log_info!(
            "Connection target: {}://{}:{}{}",
            scheme,
            display_host,
            target.port,
            target.path
        );

        let host_header = format_host_header(&target.host, target.port, target.tls);
        let options = ClientConnectionOptions::new(&host_header, &target.path)
            .ping_interval(10_000)
            .header("User-Agent", &user_agent);
        let (timer_tx, mut timer_rx) = mpsc::channel::<TimerId>(16);
        let mut timers = TimerManager::new(timer_tx);
        let secure_random = SecureRandom::new();
        let mut ws = WebSocketClientConnection::new(options, secure_random.clone());
        ws.connect()?;
        if flush_ws_output(&mut ws, &mut stream, &mut timers).await? {
            return Ok(());
        }

        let mut redirect_location: Option<String> = None;
        let mut redirect = false;
        let mut use_data_channel_signaling = false;
        let mut websocket_closed = false;
        let mut switched_received = false;
        let mut switched_ignore_disconnect_websocket = false;
        let mut opened_data_channels = HashSet::<String>::new();
        // DataChannel シグナリングへの切替後、WebSocket を自発的に閉じるまでの
        // 待機 (WS_DISCONNECT_DELAY) の開始時刻。設定・リセットはループ冒頭で行う。
        let mut ws_disconnect_delay_start: Option<tokio::time::Instant> = None;
        const WS_DISCONNECT_DELAY: Duration = Duration::from_secs(10);
        let mut buf = vec![0u8; 8192];

        // DataChannel の signaling label で受信した server Close による終了かどうか。
        // server Close はすでに terminal event として確定しているため、
        // 終了処理の WebSocket close handshake で発生するエラーを warning に落とすために使う。
        // また、終了処理の close handshake で Sora の WebSocket Close フレームを受信した
        // 場合に on_websocket_close を通知するかどうかの判定にも使う。
        let mut server_close_received = false;
        // ユーザー主導の切断 (SoraConnectionCommand::Disconnect) による終了かどうか。
        // ユーザーが切断を要求している以上、終了処理の WebSocket close handshake で
        // 発生する I/O エラーは warning に落として run の Ok(()) を覆さないために使う。
        let mut user_initiated_disconnect = false;

        'run_loop: loop {
            // 期限切れの値を sleep_until で持つと即 Ready になりビジーループしてしまうため、
            // 切断待機の開始時刻を毎イテレーション再計算する。
            // resolve_ws_disconnect_delay_start() は、WS close 送信後や、
            // 切替条件が不成立になった場合に None を返す。
            // ws_disconnect_delay_start が None だと pending 処理に入るため
            // ビジーループが回避できる。
            ws_disconnect_delay_start = resolve_ws_disconnect_delay_start(
                ws_disconnect_delay_start,
                ws.state() == ConnectionState::Connected,
                is_switching_ready(
                    switched_ignore_disconnect_websocket,
                    websocket_closed,
                    switched_received,
                    &self.data_channel_configs,
                    &opened_data_channels,
                ),
                tokio::time::Instant::now(),
            );

            tokio::select! {
                read = stream.read(&mut buf), if !websocket_closed => {
                    // ピアが close_notify を送らずに TCP を閉じた場合 UnexpectedEof になるため、
                    // n == 0 と同じ「相手から切られた」扱いに合流させる。
                    let n = match read {
                        Ok(n) => n,
                        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => 0,
                        Err(e) => {
                            if switched_ignore_disconnect_websocket && use_data_channel_signaling {
                                rtc_log_warning!(
                                    "WebSocket read failed; continuing DataChannel signaling: {}",
                                    e
                                );
                                websocket_closed = true;
                                continue;
                            }
                            return Err(e.into());
                        }
                    };
                    if n == 0 {
                        if switched_ignore_disconnect_websocket && use_data_channel_signaling {
                            rtc_log_info!("WebSocket closed; continuing DataChannel signaling");
                            websocket_closed = true;
                            continue;
                        } else {
                            rtc_log_info!("Connection closed");
                            break;
                        }
                    } else {
                        if let Err(e) = ws.feed_recv_buf(&buf[..n], now()?) {
                            if switched_ignore_disconnect_websocket && use_data_channel_signaling {
                                rtc_log_warning!(
                                    "WebSocket frame processing failed; continuing DataChannel signaling: {}",
                                    e
                                );
                                websocket_closed = true;
                                continue;
                            }
                            return Err(e.into());
                        }
                    }
                }
                Some(timer_id) = timer_rx.recv() => {
                    ws.handle_timer(timer_id)?;
                }
                Some(event) = self.event_rx.recv() => {
                    match event {
                        SoraEvent::SignalingMessage(message) => {
                            self.send_message_for_signaling(
                                &mut *handler,
                                &mut ws,
                                use_data_channel_signaling,
                                &message,
                            )?;
                        }
                        SoraEvent::DataChannelMessage { label, data } => {
                            match self
                                .handle_data_channel_message(&mut *handler, &label, &data)
                                .await?
                            {
                                HandleDataChannelMessageResult::Continue => {}
                                HandleDataChannelMessageResult::ServerClose {
                                    code,
                                    reason,
                                } => {
                                    rtc_log_info!(
                                        "Received server close message: code={} reason={}",
                                        code,
                                        reason
                                    );
                                    server_close_received = true;
                                    // server Close は接続全体の終了通知であり、
                                    // 同一 iteration で終了処理へ移行する。
                                    // それ以降の redirect、WebSocket flush、command、
                                    // signaling event は処理しない。
                                    break 'run_loop;
                                }
                            }
                        }
                        SoraEvent::Track(transceiver) => {
                            handler.on_track(transceiver);
                        }
                        SoraEvent::RemoveTrack(receiver) => {
                            handler.on_remove_track(receiver);
                        }
                        SoraEvent::DataChannelRegister(channel) => {
                            let Ok(label) = channel.label() else {
                                continue;
                            };
                            rtc_log_info!("Registered DataChannel '{}'", label);
                            handler.on_data_channel(&label);
                            self.register_data_channel(channel, &event_tx);
                            self.handle_data_channel_state(&mut *handler, &label, &mut opened_data_channels, &mut use_data_channel_signaling, switched_received);
                        }
                        SoraEvent::RpcTimeout { id } => {
                            if let Some(mut pending) = self.pending_rpc_responses.remove(&id) {
                                let _ = pending.response_tx.take().expect("response_tx must exist").send(Err(Error::RpcTimeout));
                            }
                        }
                        SoraEvent::DataChannelStateChange(label) => {
                            self.handle_data_channel_state(&mut *handler, &label, &mut opened_data_channels, &mut use_data_channel_signaling, switched_received);
                        }
                        // このイベントを受信したら WebSocket メッセージを送信する
                        // 送信時のエラーはログだけ出して無視する
                        SoraEvent::SendWebSocketMessage(message) => {
                            if let Err(e) = self.send_websocket_message(&mut ws, &message) {
                                rtc_log_warning!("Failed to send WebSocket message: error={}", e);
                            }
                        }
                        // このイベントを受信したら DataChannel メッセージを送信する
                        // 送信時のエラーはログだけ出して無視する
                        SoraEvent::SendDataChannelMessage { label, message } => {
                            if let Err(e) = self.send_data_channel_message(&label, message.as_bytes()) {
                                rtc_log_warning!("Failed to send DataChannel message: label='{}' error={}", label, e);
                            }
                        }
                    }
                }
                // ハンドル経由での切断要求
                Some(command) = self.command_rx.recv() => {
                    match command {
                        SoraConnectionCommand::Disconnect(ack_tx) => {
                            rtc_log_info!("Received disconnect request");
                            // ユーザー主導の切断として記録し、close handshake 中の
                            // I/O エラーを warning に落とす対象に加える。
                            user_initiated_disconnect = true;
                            // サーバーに切断を通知するため、disconnect メッセージを送信する。
                            // 送信失敗しても切断処理を中断せず、ログに残すだけにする。
                            let disconnect_message =
                                Json(OutgoingMessage::new_disconnect()).to_string();
                            // DataChannel または WebSocket 経由で disconnect メッセージを送信する。
                            if let Err(e) = self.send_message_for_signaling(
                                &mut *handler,
                                &mut ws,
                                use_data_channel_signaling,
                                &disconnect_message,
                            ) {
                                rtc_log_error!("Failed to send disconnect message: {}", e);
                            }

                            // WebSocket シグナリングの場合は、オープン中の DataChannel に
                            // 対して close コールバックを呼ぶ。
                            // DataChannel シグナリングの場合は、run ループ終了後の
                            // クローズ待機 (disconnect_wait_timeout) で close コールバックを
                            // 通知するため、ここではクリアしない。
                            if !use_data_channel_signaling {
                                for label in &opened_data_channels {
                                    rtc_log_info!("DataChannel '{}' closed", label);
                                    handler.on_data_channel_close(label);
                                }
                                opened_data_channels.clear();
                            }
                            let _ = ack_tx.send(());
                            break;
                        }
                        SoraConnectionCommand::GetStats(stats_response_tx) => {
                            // get_stats のコールバック内で直接応答を送信するため、
                            // run ループはブロックされない。
                            let pc = &self.pc;
                            pc.get_stats(move |report| {
                                let result = report_to_json_string(&report);
                                let _ = stats_response_tx.send(result);
                            });
                        }
                        SoraConnectionCommand::GetSelectedSignalingUrl(response_tx) => {
                            let _ = response_tx.send(self.selected_signaling_url.clone());
                        }
                        SoraConnectionCommand::GetConnectedSignalingUrl(response_tx) => {
                            let _ = response_tx.send(self.connected_signaling_url.clone());
                        }
                        SoraConnectionCommand::SendRpcRequest { method, params, notification, timeout, response_tx } => {
                            let (message, rpc_id) = rpc::build_rpc_message(&mut self.rpc_id_counter, &method, params.as_ref(), notification);
                            let result = self.send_data_channel_message("rpc", message.as_bytes());
                            match result {
                                Ok(()) => {
                                    if notification {
                                        let _ = response_tx.send(Ok(None));
                                    } else {
                                        let id = rpc_id.expect("id must exist when notification is false");
                                        let event_tx = event_tx.clone();
                                        let timeout_handle = tokio::spawn(async move {
                                            tokio::time::sleep(timeout).await;
                                            let _ = event_tx.send(SoraEvent::RpcTimeout { id });
                                        });
                                        self.pending_rpc_responses.insert(id, PendingRpcRequest {
                                            response_tx: Some(response_tx),
                                            timeout_handle,
                                        });
                                    }
                                }
                                Err(e) => {
                                    let _ = response_tx.send(Err(e));
                                }
                            }
                        }
                        SoraConnectionCommand::SendMessage { label, data, response_tx } => {
                            let result = self.handle_send_message_command(&label, &data);
                            let _ = response_tx.send(result);
                        }
                    }
                }
                // 切断待機中 (開始時刻が設定されている間) は WS_DISCONNECT_DELAY 秒待つ。
                // 未設定なら pending で何もしない。
                _ = async {
                    if let Some(start) = ws_disconnect_delay_start {
                        tokio::time::sleep_until(start + WS_DISCONNECT_DELAY).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                }
            }

            // websocket_closed=true は WebSocket のソケットが死んでいるか ws 層が
            // failed 状態で、残留イベントの処理は無意味である。
            // 特にプロトコルエラー吸収時は close_internal で state が Closing に
            // 遷移済みのため、残留 Offer/ReOffer イベントを処理すると send_text の
            // check_connected() が失敗して run() が Err を返してしまう。
            // そのため websocket_closed=true の間は ws のイベント処理を丸ごとスキップする。
            if !websocket_closed {
                while let Some(event) = ws.poll_event() {
                    match event {
                        ConnectionEvent::Connected { .. } => {
                            rtc_log_info!("WebSocket connection established");
                            let connect_message = OutgoingMessage::new_connect(
                                &channel_id,
                                role,
                                client_id.clone(),
                                bundle_id.clone(),
                                redirect,
                                sora_client.clone(),
                                libwebrtc.clone(),
                                environment.clone(),
                                metadata.clone(),
                                data_channel_signaling,
                                ignore_disconnect_websocket,
                                simulcast,
                                simulcast_request_rid.clone(),
                                spotlight,
                                spotlight_focus_rid.clone(),
                                spotlight_unfocus_rid.clone(),
                                signaling_notify_metadata.clone(),
                                data_channels.clone(),
                                forwarding_filters.clone(),
                                audio.clone(),
                                video.clone(),
                            );
                            let connect_text = Json(connect_message).to_string();
                            handler.on_signaling_message(
                                SignalingType::WebSocket,
                                SignalingDirection::Sent,
                                &connect_text,
                            );
                            self.send_websocket_message(&mut ws, &connect_text)?;
                        }
                        ConnectionEvent::TextMessage(text) => {
                            rtc_log_info!(
                                "[WebSocket] Received text message of {} bytes",
                                text.len()
                            );
                            let message = match IncomingMessage::parse(&text) {
                                Ok(message) => message,
                                Err(err) => {
                                    rtc_log_error!("Failed to parse JSON message: {}", err);
                                    continue;
                                }
                            };
                            match message.data {
                                IncomingMessageData::Offer {
                                    sdp,
                                    ice_servers,
                                    data_channels,
                                    simulcast,
                                    encodings,
                                } => {
                                    handler.on_signaling_message(
                                        SignalingType::WebSocket,
                                        SignalingDirection::Received,
                                        &text,
                                    );
                                    self.data_channel_configs = data_channels;
                                    self.offer_simulcast = simulcast;
                                    self.simulcast_encodings = encodings;
                                    let answer_sdp = self.handle_offer(&sdp, &ice_servers).await?;
                                    let answer_message = OutgoingMessage::new_answer(&answer_sdp);
                                    let answer_text = Json(answer_message).to_string();
                                    handler.on_signaling_message(
                                        SignalingType::WebSocket,
                                        SignalingDirection::Sent,
                                        &answer_text,
                                    );
                                    self.send_websocket_message(&mut ws, &answer_text)?;
                                }
                                IncomingMessageData::ReOffer { sdp } => {
                                    handler.on_signaling_message(
                                        SignalingType::WebSocket,
                                        SignalingDirection::Received,
                                        &text,
                                    );
                                    let answer_sdp = self.handle_offer(&sdp, &[]).await?;
                                    let reanswer_message =
                                        OutgoingMessage::new_reanswer(&answer_sdp);
                                    let reanswer_text = Json(reanswer_message).to_string();
                                    handler.on_signaling_message(
                                        SignalingType::WebSocket,
                                        SignalingDirection::Sent,
                                        &reanswer_text,
                                    );
                                    self.send_websocket_message(&mut ws, &reanswer_text)?;
                                }
                                IncomingMessageData::Ping { stats } => {
                                    if stats.unwrap_or(false) {
                                        self.request_stats_pong(&event_tx);
                                    } else {
                                        self.send_pong(&event_tx);
                                    }
                                }
                                IncomingMessageData::ReqStats {} => {
                                    // 統計情報を含む stats メッセージを送信
                                    self.request_stats_response(&event_tx);
                                }
                                IncomingMessageData::Notify {} => {
                                    handler.on_notify(&message.message);
                                }
                                IncomingMessageData::Push {} => {
                                    handler.on_push(&message.message);
                                }
                                IncomingMessageData::Switched {
                                    ignore_disconnect_websocket: iws,
                                } => {
                                    switched_received = true;
                                    switched_ignore_disconnect_websocket = iws;
                                    handler.on_switched();
                                    if !use_data_channel_signaling
                                        && is_data_channel_signaling_ready(
                                            switched_received,
                                            &self.data_channel_configs,
                                            &opened_data_channels,
                                        )
                                    {
                                        use_data_channel_signaling = true;
                                    }
                                }
                                IncomingMessageData::Redirect { location } => {
                                    handler.on_signaling_message(
                                        SignalingType::WebSocket,
                                        SignalingDirection::Received,
                                        &text,
                                    );
                                    rtc_log_info!("Received redirect message: {}", location);
                                    redirect_location = Some(location);
                                    break;
                                }
                                IncomingMessageData::Close { .. } => {
                                    rtc_log_info!("Disconnected from Sora server");
                                    break;
                                }
                            }
                        }
                        ConnectionEvent::BinaryMessage(data) => {
                            rtc_log_info!(
                                "[WebSocket] Received binary message: {} bytes",
                                data.len()
                            );
                        }
                        ConnectionEvent::Ping(_) => {
                            rtc_log_info!("[WebSocket] Received Ping");
                        }
                        ConnectionEvent::Pong(_) => {
                            rtc_log_info!("[WebSocket] Received Pong");
                        }
                        ConnectionEvent::Close { code, reason } => {
                            rtc_log_info!("[WebSocket] Received Close: {:?} {}", code, reason);
                            handler.on_websocket_close(code.map(|c| c.0), &reason);
                            break;
                        }
                        ConnectionEvent::StateChanged(state) => {
                            rtc_log_info!("[WebSocket] State: {:?}", state);
                        }
                        ConnectionEvent::Error(err) => {
                            rtc_log_error!("[WebSocket] Error: {}", err);
                        }
                    }
                }
            }

            // redirect メッセージを受信した場合、新しい WebSocket に再接続する
            if let Some(location) = redirect_location.take() {
                // セッション状態をリセットする。
                // 旧接続の状態が redirect 先に持ち越されると、
                // switched フラグや DataChannel 状態が不整合を起こす。
                switched_received = false;
                switched_ignore_disconnect_websocket = false;
                use_data_channel_signaling = false;

                // 古い DataChannel の close 通知をユーザーに送る。
                // クリア前に通知することでハンドラが確実に呼ばれる。
                for label in &opened_data_channels {
                    handler.on_data_channel_close(label);
                }
                opened_data_channels.clear();
                self.data_channels.clear();
                self.data_channel_configs.clear();

                // 旧セッションの RPC リクエストに redirect エラーを通知しクリアする。
                for (_, mut pending) in self.pending_rpc_responses.drain() {
                    if let Some(tx) = pending.response_tx.take() {
                        let _ = tx.send(Err(Error::Redirected));
                    }
                }
                self.rpc_id_counter = 0;

                // 旧セッションの event_rx に滞留したイベントをドレインする。
                // クリア後の data_channels に旧チャネルが再登録されるのを防ぐ。
                while self.event_rx.try_recv().is_ok() {}

                // 古い WebSocket をクローズする
                if ws.state() == ConnectionState::Connected {
                    ws.close(CloseCode::NORMAL, "redirect")?;
                    let _ = flush_ws_output(&mut ws, &mut stream, &mut timers).await;
                }

                // 新しい接続先にリダイレクトする
                let new_target = parse_signaling_url(&location)?;
                let display_host = format_bracketed_host(&new_target.host);
                let scheme = if new_target.tls { "wss" } else { "ws" };
                rtc_log_info!(
                    "Redirect target: {}://{}:{}{}",
                    scheme,
                    display_host,
                    new_target.port,
                    new_target.path
                );

                stream = connect_websocket(
                    &new_target,
                    proxy.as_ref(),
                    &tls_config,
                    websocket_connection_timeout,
                )
                .await?;
                // リダイレクト先の URL を記録する
                self.connected_signaling_url = Some(location);
                let host_header =
                    format_host_header(&new_target.host, new_target.port, new_target.tls);
                let options = ClientConnectionOptions::new(&host_header, &new_target.path)
                    .ping_interval(10_000)
                    .header("User-Agent", &user_agent);
                let (new_timer_tx, new_timer_rx) = mpsc::channel::<TimerId>(16);
                timers = TimerManager::new(new_timer_tx);
                timer_rx = new_timer_rx;
                ws = WebSocketClientConnection::new(options, secure_random.clone());
                ws.connect()?;
                websocket_closed = false;
                redirect = true;
                if flush_ws_output(&mut ws, &mut stream, &mut timers).await? {
                    break;
                }
                continue;
            }

            // DataChannel シグナリングへの切替条件が揃ったら WebSocket を切断する。
            if is_switching_ready(
                switched_ignore_disconnect_websocket,
                websocket_closed,
                switched_received,
                &self.data_channel_configs,
                &opened_data_channels,
            ) && let Some(start) = ws_disconnect_delay_start
                && start.elapsed() >= WS_DISCONNECT_DELAY
                && ws.state() == ConnectionState::Connected
            {
                ws.close(CloseCode::NORMAL, "switching to datachannel")?;
            }

            // websocket_closed=true の間は ws を破棄済みとして扱うため、
            // flush と close 検知をスキップする。吸収後に flush を続けると
            // 死んだソケットへの write 失敗が警告として繰り返し出る。
            if !websocket_closed {
                // WebSocket クローズ検知 (CloseConnection 出力 / ws.state() == Closed) を 1 箇所に集約する。
                // ignore_disconnect_websocket=true 成立時は break せず DataChannel シグナリングを継続する。
                let close_emitted = match flush_ws_output(&mut ws, &mut stream, &mut timers).await {
                    Ok(emitted) => emitted,
                    Err(e) => {
                        if switched_ignore_disconnect_websocket && use_data_channel_signaling {
                            // switched 後の WebSocket I/O 失敗は DataChannel シグナリング継続のため吸収する。
                            // flush 失敗はソケット死を意味し、close 完了と同じく ws 終了として扱うため、
                            // close_emitted=true と同じ扱いで close 検知に合流させる。
                            rtc_log_warning!(
                                "flush WebSocket output failed; continuing DataChannel signaling: {}",
                                e
                            );
                            true
                        } else {
                            return Err(e);
                        }
                    }
                };

                if close_emitted || ws.state() == ConnectionState::Closed {
                    if switched_ignore_disconnect_websocket && use_data_channel_signaling {
                        rtc_log_info!("WebSocket closed; continuing DataChannel signaling");
                        websocket_closed = true;
                        continue;
                    } else {
                        break;
                    }
                }
            }
        }

        // run ループを抜けた時点で終了フェーズに入るため、以後のコマンド送信を拒否する。
        close_command_channel_and_ack_pending_disconnects(
            &mut self.command_rx,
            &mut user_initiated_disconnect,
        )
        .await;

        // DataChannel シグナリングを利用している場合は、
        // disconnect_wait_timeout を上限にクローズ完了を待機する。
        if use_data_channel_signaling && !opened_data_channels.is_empty() {
            let deadline = tokio::time::Instant::now() + disconnect_wait_timeout;
            // .await を超える値を渡す場合は Send である必要があるが、&data_channels は
            // Send でないためエラーになるので、self.data_channels で所有権ごと渡す。
            // これ以降 self.data_channels は利用できないので、
            // 問題になるようならインターフェースを &mut data_channels に変更すること。
            let data_channels = self.data_channels;
            wait_data_channels_close(
                &mut self.event_rx,
                &mut *handler,
                &mut opened_data_channels,
                data_channels,
                deadline,
            )
            .await;
        }

        // websocket_closed=true は WebSocket のソケットが死んでいるか ws 層が failed 状態で、
        // 正常な close handshake が成立しない。この状態で close handshake を実行すると
        // 死んだソケットへの I/O が失敗し、ユーザー主導の正常切断にもかかわらず
        // run() が Err を返してしまうため、close handshake をスキップする。
        if !websocket_closed {
            // close handshake 中の I/O エラーを吸収すべき終了経路を合成して渡す。
            // ignore 構成とユーザー主導の切断は「接続を終了する意思が確定している」
            // 点で共通するため、まとめて 1 つの吸収条件にする。server_close_received
            // も吸収条件に含まれるが、close handshake 中に受信した WebSocket Close
            // フレームで on_websocket_close を通知する判定にも使うため別引数のままにする。
            close_websocket_handshake(
                &mut ws,
                &mut stream,
                &mut timers,
                &mut *handler,
                server_close_received,
                (switched_ignore_disconnect_websocket && use_data_channel_signaling)
                    || user_initiated_disconnect,
                websocket_close_timeout,
            )
            .await?;
        }

        rtc_log_info!("Shutting down");
        Ok(())
    }

    fn add_sender_tracks(&mut self) -> Result<()> {
        if let Some(track) = self.config.sender_video_track.take() {
            let media_track = track.cast_to_media_stream_track();
            let sender = self.add_sender_media_track(&media_track)?;
            self.video_sender = Some(sender);
        }
        if let Some(track) = self.config.sender_audio_track.take() {
            let media_track = track.cast_to_media_stream_track();
            let _ = self.add_sender_media_track(&media_track)?;
        }
        Ok(())
    }

    fn add_sender_media_track(&mut self, media_track: &MediaStreamTrack) -> Result<RtpSender> {
        let pc = &self.pc;
        let mut stream_ids = StringVector::new(0);
        let stream_id = shiguredo_webrtc::random_string(16);
        stream_ids.push(&CxxString::from_str(&stream_id));
        Ok(pc.add_track(media_track, &stream_ids)?)
    }

    fn apply_simulcast_encodings(&mut self) -> Result<()> {
        if self.video_sender.is_none() {
            return Err(Error::SimulcastVideoSenderMissing);
        }

        // 直前の is_none チェックで欠落時は Err を返している。
        let sender = self
            .video_sender
            .as_mut()
            .expect("video_sender must exist after is_none check");
        let mut parameters = sender.get_parameters();
        let mut encodings = RtpEncodingParametersVector::new(0);
        for cfg in &self.simulcast_encodings {
            let mut encoding = RtpEncodingParameters::new();
            encoding.set_rid(&cfg.rid);

            encoding.set_max_bitrate_bps(cfg.max_bitrate);
            encoding.set_min_bitrate_bps(cfg.min_bitrate);
            encoding.set_scale_resolution_down_by(cfg.scale_resolution_down_by);
            encoding.set_max_framerate(cfg.max_framerate);
            if let Some(active) = cfg.active {
                encoding.set_active(active);
            }
            if let Some(adaptive_ptime) = cfg.adaptive_ptime {
                encoding.set_adaptive_ptime(adaptive_ptime);
            }
            encoding.set_scalability_mode(cfg.scalability_mode.as_deref());
            if let Some(v) = &cfg.scale_resolution_down_to {
                let mut resolution = Resolution::new();
                resolution.set_width(v.max_width);
                resolution.set_height(v.max_height);
                encoding.set_scale_resolution_down_to(Some(&resolution));
            }

            encodings.push(&encoding);
        }

        parameters.set_encodings(&encodings);
        sender
            .set_parameters(&parameters)
            .map_err(|source| Error::SimulcastSetParametersFailed { source })?;
        Ok(())
    }

    fn configure_ice_server_urls(
        server_entry: &mut IceServer,
        urls: &[String],
        configurer: Option<&IceServerUrlConfigurer>,
    ) {
        if let Some(configurer) = configurer {
            configurer(server_entry, urls);
            return;
        }
        for url in urls {
            server_entry.add_url(url);
        }
    }

    fn apply_pc_configuration(&mut self, servers: &[IceServerConfig]) -> Result<()> {
        if servers.is_empty() {
            return Ok(());
        }
        let pc = &mut self.pc;
        let mut config = PeerConnectionRtcConfiguration::new();
        for server in servers {
            let mut server_entry = IceServer::new();
            if let Some(user) = &server.username {
                server_entry.set_username(user);
            }
            if let Some(pass) = &server.credential {
                server_entry.set_password(pass);
            }
            if self.config.turn_tls_insecure {
                server_entry.set_tls_cert_policy(TlsCertPolicy::InsecureNoCheck);
            }
            Self::configure_ice_server_urls(
                &mut server_entry,
                &server.urls,
                self.config.ice_server_url_configurer.as_deref(),
            );
            if server_entry.urls_len() == 0 {
                continue;
            }
            config.servers().push(&server_entry);
        }
        pc.set_configuration(&mut config)?;
        Ok(())
    }

    fn send_pong(&self, event_tx: &mpsc::UnboundedSender<SoraEvent>) {
        let message = OutgoingMessage::new_pong(None);
        // pong はシグナリングメッセージとして返信するが、on_signaling_message は発生させない
        let _ = event_tx.send(SoraEvent::SendWebSocketMessage(Json(message).to_string()));
    }

    fn request_stats_pong(&self, event_tx: &mpsc::UnboundedSender<SoraEvent>) {
        let pc = &self.pc;
        let event_tx = event_tx.clone();
        pc.get_stats(move |report| {
            let message = OutgoingMessage::new_pong(report_to_json_string(&report).ok());
            // pong はシグナリングメッセージとして返信するが、on_signaling_message は発生させない
            let _ = event_tx.send(SoraEvent::SendWebSocketMessage(Json(message).to_string()));
        });
    }

    fn request_stats_response(&self, event_tx: &mpsc::UnboundedSender<SoraEvent>) {
        let pc = &self.pc;
        let event_tx = event_tx.clone();
        pc.get_stats(move |report| {
            if let Ok(reports) = report_to_json_string(&report) {
                let message = OutgoingMessage::new_stats(reports);
                // req-stats はシグナリングメッセージとして返信するが、on_signaling_message は発生させない
                let _ = event_tx.send(SoraEvent::SendWebSocketMessage(Json(message).to_string()));
            }
        });
    }

    async fn handle_offer(&mut self, sdp: &str, ice_servers: &[IceServerConfig]) -> Result<String> {
        self.apply_pc_configuration(ice_servers)?;

        let (rem_tx, mut rem_rx) = mpsc::unbounded_channel::<Option<String>>();
        {
            let pc = &self.pc;
            let offer = SessionDescription::new(SdpType::Offer, sdp)?;
            let rem_obs = SetRemoteDescriptionObserver::new_with_handler(Box::new(
                SetDescriptionObserverHandler { tx: rem_tx },
            ));
            pc.set_remote_description(offer, &rem_obs);
        }
        let rem_res = tokio::time::timeout(SDP_OPERATION_TIMEOUT, rem_rx.recv())
            .await
            .map_err(|_| Error::SetRemoteDescriptionTimeout)?
            .ok_or_else(|| Error::SetRemoteDescriptionResponseMissing)?;
        if let Some(err) = rem_res {
            return Err(Error::SetRemoteDescriptionFailed { reason: err });
        }

        if self.config.role.wants_send() {
            self.add_sender_tracks()?;
        }

        if self.offer_simulcast && !self.simulcast_encodings.is_empty() {
            self.apply_simulcast_encodings()?;
        }

        let (ans_tx, mut ans_rx) = mpsc::unbounded_channel::<Result<String>>();

        struct AnsObsHandler {
            tx: mpsc::UnboundedSender<Result<String>>,
        }

        impl CreateSessionDescriptionObserverHandler for AnsObsHandler {
            fn on_success(&mut self, desc: SessionDescription) {
                let sdp = desc.to_string().map_err(Error::Webrtc);
                let _ = self.tx.send(sdp);
            }

            fn on_failure(&mut self, error: RtcError) {
                let msg = error.message().unwrap_or_else(|_| "unknown".to_string());
                let _ = self.tx.send(Err(Error::AnswerFailed { reason: msg }));
            }
        }

        let mut ans_obs =
            CreateSessionDescriptionObserver::new_with_handler(Box::new(AnsObsHandler {
                tx: ans_tx,
            }));

        {
            let pc = &self.pc;
            let mut opts = PeerConnectionOfferAnswerOptions::new();
            pc.create_answer(&mut ans_obs, &mut opts);
        }
        let answer_sdp = tokio::time::timeout(SDP_OPERATION_TIMEOUT, ans_rx.recv())
            .await
            .map_err(|_| Error::AnswerTimeout)?
            .ok_or_else(|| Error::AnswerResponseMissing)??;

        let answer = SessionDescription::new(SdpType::Answer, &answer_sdp)?;
        let (loc_tx, mut loc_rx) = mpsc::unbounded_channel::<Option<String>>();
        let loc_obs = SetLocalDescriptionObserver::new_with_handler(Box::new(
            SetDescriptionObserverHandler { tx: loc_tx },
        ));
        {
            let pc = &self.pc;
            pc.set_local_description(answer, &loc_obs);
        }
        let loc_res = tokio::time::timeout(SDP_OPERATION_TIMEOUT, loc_rx.recv())
            .await
            .map_err(|_| Error::SetLocalDescriptionTimeout)?
            .ok_or_else(|| Error::SetLocalDescriptionResponseMissing)?;
        if let Some(err) = loc_res {
            return Err(Error::SetLocalDescriptionFailed { reason: err });
        }

        Ok(answer_sdp)
    }

    fn register_data_channel(
        &mut self,
        mut channel: DataChannel,
        event_tx: &mpsc::UnboundedSender<SoraEvent>,
    ) {
        let Ok(label) = channel.label() else {
            return;
        };
        let initial_state = channel.state();

        let config = self.data_channel_configs.iter().find(|c| c.label == label);
        let compress = config.is_some_and(|c| c.compress);

        let event_tx_for_observer = event_tx.clone();
        let event_tx_for_message = event_tx.clone();
        let label_for_state = label.clone();
        let label_for_message = label.clone();
        struct DcObsHandler {
            label_for_state: String,
            label_for_message: String,
            event_tx_for_observer: mpsc::UnboundedSender<SoraEvent>,
            event_tx_for_message: mpsc::UnboundedSender<SoraEvent>,
        }

        impl DataChannelObserverHandler for DcObsHandler {
            fn on_state_change(&mut self) {
                let _ = self
                    .event_tx_for_observer
                    .send(SoraEvent::DataChannelStateChange(
                        self.label_for_state.clone(),
                    ));
            }

            fn on_message(&mut self, data: &[u8], _is_binary: bool) {
                let _ = self
                    .event_tx_for_message
                    .send(SoraEvent::DataChannelMessage {
                        label: self.label_for_message.clone(),
                        data: data.to_vec(),
                    });
            }
        }

        let observer = DataChannelObserver::new_with_handler(Box::new(DcObsHandler {
            label_for_state,
            label_for_message,
            event_tx_for_observer,
            event_tx_for_message,
        }));
        channel.register_observer(&observer);

        let managed = ManagedDataChannel {
            channel,
            observer,
            compress,
        };
        self.data_channels.insert(label.clone(), managed);

        // 登録時に既に Open なら通知
        if initial_state == DataChannelState::Open {
            let _ = event_tx.send(SoraEvent::DataChannelStateChange(label));
        }
    }

    fn handle_data_channel_state(
        &self,
        handler: &mut dyn SoraConnectionEventHandler,
        label: &str,
        opened_data_channels: &mut HashSet<String>,
        use_data_channel_signaling: &mut bool,
        switched_received: bool,
    ) {
        if is_data_channel_open(&self.data_channels, label) && !opened_data_channels.contains(label)
        {
            rtc_log_info!("DataChannel '{}' opened", label);
            opened_data_channels.insert(label.to_string());
            handler.on_data_channel_open(label);
            if !*use_data_channel_signaling
                && is_data_channel_signaling_ready(
                    switched_received,
                    &self.data_channel_configs,
                    opened_data_channels,
                )
            {
                *use_data_channel_signaling = true;
            }
        } else if should_notify_close(&self.data_channels, opened_data_channels, label) {
            notify_data_channel_closed(handler, opened_data_channels, label);
        }
    }

    /// 経路に応じてシグナリング用のメッセージを送信する。
    ///
    /// 送信前に `handler.on_signaling_message` を呼んで通知する。
    /// DataChannel シグナリングが有効な状態 (use_data_channel_signaling) なら
    /// signaling DataChannel 経由、そうでなければ WebSocket 経由で送信する。
    /// 経路選択はこの関数に一元化し、呼び出し側はエラーの扱いだけを決める。
    fn send_message_for_signaling<R: RandomSource>(
        &mut self,
        handler: &mut dyn SoraConnectionEventHandler,
        ws: &mut WebSocketClientConnection<R>,
        use_data_channel_signaling: bool,
        message: &str,
    ) -> Result<()> {
        if use_data_channel_signaling {
            handler.on_signaling_message(
                SignalingType::DataChannel,
                SignalingDirection::Sent,
                message,
            );
            self.send_data_channel_message("signaling", message.as_bytes())
        } else if ws.state() == ConnectionState::Connected {
            handler.on_signaling_message(
                SignalingType::WebSocket,
                SignalingDirection::Sent,
                message,
            );
            self.send_websocket_message(ws, message)
        } else {
            rtc_log_warning!(
                "No signaling path available (DataChannel signaling is disabled and WebSocket is not connected); message not sent"
            );
            Ok(())
        }
    }

    /// WebSocket 経由でテキストメッセージを送信する。
    fn send_websocket_message<R: RandomSource>(
        &self,
        ws: &mut WebSocketClientConnection<R>,
        message: &str,
    ) -> Result<()> {
        rtc_log_info!("[WebSocket] Sent text message of {} bytes", message.len());
        ws.send_text(message)?;
        Ok(())
    }

    /// DataChannel 経由でバイナリメッセージを送信する。
    ///
    /// ラベルは送信先 DataChannel を指定する。テキストを送る場合は
    /// `message.as_bytes()` で渡す。
    fn send_data_channel_message(&mut self, label: &str, data: &[u8]) -> Result<()> {
        let managed =
            self.data_channels
                .get_mut(label)
                .ok_or_else(|| Error::DataChannelMissing {
                    label: label.to_string(),
                })?;

        let send_data = if managed.compress {
            compress_zlib(data)?
        } else {
            data.to_vec()
        };

        rtc_log_verbose!(
            "Sent message to DataChannel '{}': {} bytes",
            label,
            send_data.len()
        );

        if !managed.channel.send(&send_data, true) {
            return Err(Error::DataChannelSendFailed);
        }
        Ok(())
    }

    /// SendMessage コマンドのラベル検証と DataChannel への送信を実行する。
    ///
    /// `#` プレフィックス付きで `data_channel_configs` に登録済みのラベルのみ送信を試みる。
    /// それ以外のラベルは `Error::InvalidDataChannelLabel` を返す。
    /// ラベルが登録済みでも実チャネル (`data_channels`) が無い場合は
    /// `Error::DataChannelMissing` を返す。
    fn handle_send_message_command(&mut self, label: &str, data: &[u8]) -> Result<()> {
        if label.starts_with('#') && self.data_channel_configs.iter().any(|c| c.label == label) {
            self.send_data_channel_message(label, data)
        } else {
            Err(Error::InvalidDataChannelLabel {
                label: label.to_string(),
            })
        }
    }

    async fn handle_data_channel_message(
        &mut self,
        handler: &mut dyn SoraConnectionEventHandler,
        label: &str,
        data: &[u8],
    ) -> Result<HandleDataChannelMessageResult> {
        // DataChannel の設定を検索
        let managed = self.data_channels.get(label);
        let compress = managed.is_some_and(|m| m.compress);

        // 圧縮されている場合は展開
        // 展開に失敗した場合は、受信メッセージ 1 件だけを破棄して処理を継続する。
        // DataChannel の本文は利用者由来の任意データを含むため、
        // 1 件の不正な圧縮データで接続全体を終了させない。
        // warning にはどの DataChannel で失敗したかを特定できるようラベルを出す。
        // 圧縮前後の本文と zlib の error message は含めない。
        let message_bytes = if compress {
            match decompress_zlib(data, MAX_DECOMPRESSED_DATA_CHANNEL_MESSAGE_SIZE) {
                Ok(bytes) => bytes,
                Err(_) => {
                    rtc_log_warning!(
                        "Discarded malformed DataChannel message: label='{}' stage=zlib",
                        label
                    );
                    return Ok(HandleDataChannelMessageResult::Continue);
                }
            }
        } else {
            data.to_vec()
        };

        rtc_log_verbose!(
            "Received message from DataChannel '{}': {} bytes",
            label,
            message_bytes.len()
        );

        // DataChannel API コールバック (全ラベル)
        handler.on_data_channel_message(label, &message_bytes);

        match label {
            // signaling, stats, push, notify ラベルの場合はシグナリングメッセージとして処理
            "signaling" | "stats" | "push" | "notify" => {
                let text = String::from_utf8(message_bytes)?;
                // signaling ラベルのみ on_signaling_message を呼ぶ
                if label == "signaling" {
                    handler.on_signaling_message(
                        SignalingType::DataChannel,
                        SignalingDirection::Received,
                        &text,
                    );
                }
                // メッセージをパースして処理
                let incoming = IncomingMessage::parse(&text)?;
                match incoming.data {
                    IncomingMessageData::ReOffer { sdp } => {
                        let answer_sdp = self.handle_offer(&sdp, &[]).await?;
                        let reanswer_message = OutgoingMessage::new_reanswer(&answer_sdp);
                        let reanswer_text = Json(reanswer_message).to_string();
                        handler.on_signaling_message(
                            SignalingType::DataChannel,
                            SignalingDirection::Sent,
                            &reanswer_text,
                        );
                        self.send_data_channel_message("signaling", reanswer_text.as_bytes())?;
                    }
                    IncomingMessageData::Ping { stats } => {
                        if stats.unwrap_or(false) {
                            // pc.get_stats() のコールバックから、event_tx 経由での DataChannel 送信で結果を返す
                            let pc = &self.pc;
                            let event_tx = self.event_tx.clone();
                            pc.get_stats(move |report| {
                                let message =
                                    OutgoingMessage::new_pong(report_to_json_string(&report).ok());
                                let text = Json(message).to_string();
                                let _ = event_tx.send(SoraEvent::SendDataChannelMessage {
                                    label: "signaling".to_string(),
                                    message: text,
                                });
                            });
                        } else {
                            let pong = OutgoingMessage::new_pong(None);
                            let pong_text = Json(pong).to_string();
                            self.send_data_channel_message("signaling", pong_text.as_bytes())?;
                        }
                    }
                    IncomingMessageData::ReqStats {} => {
                        // pc.get_stats() のコールバックから、event_tx 経由での DataChannel 送信で結果を返す
                        let pc = &self.pc;
                        let event_tx = self.event_tx.clone();
                        pc.get_stats(move |report| {
                            if let Ok(reports) = report_to_json_string(&report) {
                                let message = OutgoingMessage::new_stats(reports);
                                let text = Json(message).to_string();
                                let _ = event_tx.send(SoraEvent::SendDataChannelMessage {
                                    label: "stats".to_string(),
                                    message: text,
                                });
                            }
                        });
                    }
                    IncomingMessageData::Notify {} => {
                        handler.on_notify(&text);
                    }
                    IncomingMessageData::Push {} => {
                        handler.on_push(&text);
                    }
                    // signaling label の Close だけを接続全体の終了通知として扱う。
                    // stats / push / notify label に届いた Close は接続を終了させず、
                    // 既存どおり unsupported message として扱う。
                    IncomingMessageData::Close { code, reason } if is_server_close_label(label) => {
                        return Ok(HandleDataChannelMessageResult::ServerClose { code, reason });
                    }
                    _ => {
                        rtc_log_warning!("Received unsupported message via DataChannel");
                    }
                }
            }
            "rpc" => {
                // rpc ラベルの UTF-8 変換失敗は response の id を相関できない入力として、
                // メッセージ 1 件単位に破棄して接続を終了しない。
                // 受信本文はログに含めない。
                let text = match String::from_utf8(message_bytes) {
                    Ok(text) => text,
                    Err(_) => {
                        rtc_log_warning!(
                            "Discarded malformed RPC response: label='{}' stage=utf8",
                            label
                        );
                        return Ok(HandleDataChannelMessageResult::Continue);
                    }
                };
                rtc_log_verbose!(
                    "Received RPC message via DataChannel: label='{}' message={} bytes",
                    label,
                    text.len()
                );
                match RpcResponse::parse(&text) {
                    Ok((Some(id), response)) => {
                        if let Some(mut pending) = self.pending_rpc_responses.remove(&id) {
                            pending.timeout_handle.abort();
                            let _ = pending
                                .response_tx
                                .take()
                                .expect("response_tx must exist")
                                .send(Ok(Some(response)));
                        } else {
                            // 未知 / timeout 済み / 重複の id の正常 response は破棄する。
                            rtc_log_warning!(
                                "Discarded unmatched RPC response: label='{}' stage=unknown-id",
                                label
                            );
                        }
                    }
                    Ok((None, _)) => {
                        // 信頼できる id がない response はメッセージ単位に破棄する。
                        rtc_log_warning!(
                            "Discarded malformed RPC response: label='{}' stage=protocol",
                            label
                        );
                    }
                    Err(err) => {
                        // JSON-RPC 2.0 の要件を満たさない応答は、信頼できる既知 id がある場合のみ
                        // 対応する pending request へ error として通知する。
                        let trusted_id = match &err {
                            Error::RpcProtocolViolation { id: Some(id) } => Some(*id),
                            _ => None,
                        };
                        if let Some(id) = trusted_id
                            && let Some(mut pending) = self.pending_rpc_responses.remove(&id)
                        {
                            pending.timeout_handle.abort();
                            let _ = pending
                                .response_tx
                                .take()
                                .expect("response_tx must exist")
                                .send(Err(err));
                        } else {
                            // JSON syntax error と相関できない protocol violation は破棄する。
                            let stage = if matches!(err, Error::JsonParse(_)) {
                                "syntax"
                            } else {
                                "protocol"
                            };
                            rtc_log_warning!(
                                "Discarded malformed RPC response: label='{}' stage={}",
                                label,
                                stage
                            );
                        }
                    }
                }
            }
            // # で始まるラベルはユーザー定義メッセージとして処理
            label if label.starts_with('#') => {
                handler.on_message(label, &message_bytes);
            }
            _ => {
                rtc_log_warning!("Received unsupported label via DataChannel: {}", label);
            }
        }

        Ok(HandleDataChannelMessageResult::Continue)
    }
}

// -------------------------
// 管理対象 DataChannel
// -------------------------

/// DataChannel の受信メッセージをどう扱うかを表す。
#[derive(Debug, Clone, PartialEq, Eq)]
enum HandleDataChannelMessageResult {
    /// 通常どおり接続を継続する。
    Continue,
    /// Sora からの接続終了通知 (`signaling` label の `{"type":"close"}`)。
    ServerClose {
        /// Sora が通知した Close code。
        code: u16,
        /// Sora が通知した Close reason。
        reason: String,
    },
}

/// DataChannel シグナリングへの切替 readiness を判定する pure なヘルパー。
///
/// 次の 3 条件をすべて満たしたときだけ true を返す:
/// - WebSocket 経由で正式な `switched` メッセージを受信済み
/// - Offer の `data_channels` が空でない
/// - 全設定 DataChannel が Open 済み
fn is_data_channel_signaling_ready(
    switched_received: bool,
    data_channel_configs: &[DataChannelConfig],
    opened_labels: &HashSet<String>,
) -> bool {
    let expected = data_channel_configs.len();
    expected > 0 && switched_received && opened_labels.len() >= expected
}

/// DataChannel シグナリングへの切替が成立し、WebSocket を自発的に閉じられる状態かを返す。
///
/// 次の 3 条件をすべて満たしたときだけ true を返す:
/// - `switched` メッセージで `switched_ignore_disconnect_websocket` が true を受信済み
/// - `websocket_closed` が false
/// - `is_data_channel_signaling_ready` が true (全設定 DataChannel が Open 済み)
fn is_switching_ready(
    switched_ignore_disconnect_websocket: bool,
    websocket_closed: bool,
    switched_received: bool,
    data_channel_configs: &[DataChannelConfig],
    opened_labels: &HashSet<String>,
) -> bool {
    switched_ignore_disconnect_websocket
        && !websocket_closed
        && is_data_channel_signaling_ready(switched_received, data_channel_configs, opened_labels)
}

/// WebSocket 切断待機の開始時刻を返す。
///
/// `ws_connected` が false (close 送信後) か `switching_ready` が false (切替条件不成立) のときは
/// `None` を返す。それ以外は引数の開始時刻を維持し、未設定なら `now` で初期化する。
///
/// 切替条件が一時的に不成立になった場合は `None` を返して開始時刻を破棄するため、
/// 再び成立したときは `now` で再初期化される。このため待機中に条件が崩壊して復帰すると
/// WS_DISCONNECT_DELAY のカウントは 0 からやり直しになる。
fn resolve_ws_disconnect_delay_start(
    ws_disconnect_delay_start: Option<tokio::time::Instant>,
    ws_connected: bool,
    switching_ready: bool,
    now: tokio::time::Instant,
) -> Option<tokio::time::Instant> {
    if !ws_connected || !switching_ready {
        None
    } else {
        ws_disconnect_delay_start.or(Some(now))
    }
}

/// 指定した DataChannel label で受信した Close メッセージを server Close として
/// 扱うかどうかを返す。
///
/// Sora ドキュメント「Sora クライアント要求仕様」の
/// 「DataChannel シグナリングのみ利用時に Sora から切断が発生した際の
///  `"type": "close"` メッセージの送信」に基づき、`signaling` label の Close
/// メッセージだけを接続全体の終了通知として扱う。
/// `stats`、`push`、`notify` label に同じ JSON が届いても接続を終了させず、
/// 既存どおり unsupported message として扱う。
fn is_server_close_label(label: &str) -> bool {
    label == "signaling"
}

/// 指定したラベルの DataChannel が Open 状態かどうかを返す。
fn is_data_channel_open(data_channels: &HashMap<String, ManagedDataChannel>, label: &str) -> bool {
    data_channels
        .get(label)
        .is_some_and(|m| m.channel.state() == DataChannelState::Open)
}

/// 指定したラベルの DataChannel が Closed 状態かどうかを返す。
fn is_data_channel_closed(
    data_channels: &HashMap<String, ManagedDataChannel>,
    label: &str,
) -> bool {
    data_channels
        .get(label)
        .is_some_and(|m| m.channel.state() == DataChannelState::Closed)
}

/// DataChannel の状態遷移イベントを受信したときに、close コールバックを通知して
/// opened_data_channels から remove すべきかを判定する関数。
///
/// チャネルが実際に Closed 状態で、かつ opened_data_channels にラベルが含まれる
/// 場合のみ true を返す。Closing などの途中状態ではまだ閉じていないため false を
/// 返し、誤った close 通知を防ぐ。
fn should_notify_close(
    data_channels: &HashMap<String, ManagedDataChannel>,
    opened_data_channels: &HashSet<String>,
    label: &str,
) -> bool {
    is_data_channel_closed(data_channels, label) && opened_data_channels.contains(label)
}

/// 残りチャネルへ close コールバックを通知し、opened_data_channels をクリアする。
///
/// タイムアウトやイベントチャネルクローズで待機を終了するときに使う。
/// サーバーが DataChannel を閉じないまま切断された場合でも、
/// ユーザーへの close 通知が漏れないようにする。
fn notify_close_for_remaining(
    handler: &mut dyn SoraConnectionEventHandler,
    opened_data_channels: &mut HashSet<String>,
) {
    // イテレート中の opened_data_channels の変更を避けるため、ラベルを先に取り出す。
    let labels: Vec<String> = opened_data_channels.iter().cloned().collect();
    for label in &labels {
        notify_data_channel_closed(handler, opened_data_channels, label);
    }
}

/// DataChannel の close をログ出力し、opened_data_channels から remove してから
/// コールバックを通知する。
fn notify_data_channel_closed(
    handler: &mut dyn SoraConnectionEventHandler,
    opened_data_channels: &mut HashSet<String>,
    label: &str,
) {
    rtc_log_info!("DataChannel '{}' closed", label);
    opened_data_channels.remove(label);
    handler.on_data_channel_close(label);
}

/// DataChannel クローズ待機ループの終了要因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataChannelCloseWaitResult {
    /// 全チャネルの Closed 遷移を確認して待機を完了した。
    AllChannelsClosed,
    /// 待機時間が経過した。
    TimedOut,
    /// イベントチャネルがクローズされた。
    EventChannelClosed,
}

/// DataChannel のクローズを `deadline` まで待機する。
///
/// `event_rx` の状態遷移イベントを処理しながら、`opened_data_channels` が空になるまで待つ。
///
/// - DataChannelStateChange で閉じた DataChannel があった場合、on_data_channel_close を通知する
/// - `deadline` が経過した場合と `event_rx` がクローズされた場合は、
///   残りチャネルへ on_data_channel_close を通知して待機を終了する
async fn wait_data_channels_close(
    event_rx: &mut mpsc::UnboundedReceiver<SoraEvent>,
    handler: &mut dyn SoraConnectionEventHandler,
    opened_data_channels: &mut HashSet<String>,
    data_channels: HashMap<String, ManagedDataChannel>,
    deadline: tokio::time::Instant,
) -> DataChannelCloseWaitResult {
    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(SoraEvent::DataChannelStateChange(label)) => {
                        if should_notify_close(&data_channels, opened_data_channels, &label) {
                            notify_data_channel_closed(handler, opened_data_channels, &label);
                        }
                    }
                    Some(_) => {}
                    None => {
                        // event_rx がクローズされた場合は、残りチャネルへの close 通知を
                        // 行ってから待機を終了する。
                        rtc_log_warning!(
                            "DataChannel close wait aborted because the event channel was closed ({} channels remain)",
                            opened_data_channels.len()
                        );
                        notify_close_for_remaining(handler, opened_data_channels);
                        return DataChannelCloseWaitResult::EventChannelClosed;
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                // タイムアウトした場合も event_rx と同様、残りチャネルへの close 通知を
                // 行ってから待機を終了する。
                rtc_log_warning!(
                    "DataChannel close wait timed out ({} channels remain)",
                    opened_data_channels.len()
                );
                notify_close_for_remaining(handler, opened_data_channels);
                return DataChannelCloseWaitResult::TimedOut;
            }
        }

        if opened_data_channels.is_empty() {
            return DataChannelCloseWaitResult::AllChannelsClosed;
        }
    }
}

/// `command_rx` を閉じ、残っているコマンドを処理する。
///
/// クローズ前に送信されたコマンドは次のように処理する。
/// `Disconnect` には ack を返す (呼び出し側は成功する)。
/// `Disconnect` 以外のコマンドは応答せず破棄する (その呼び出し側は
/// `Error::CommandResponseMissing` になる)。
///
/// ドレインは `recv()` を `None` まで回して行う。UnboundedReceiver の close() のドキュメントには、
/// メッセージを落とさないためには close() 後に recv() を None まで呼ぶことと記述されている。
/// ref: https://docs.rs/tokio/latest/tokio/sync/mpsc/struct.UnboundedReceiver.html#method.close
async fn close_command_channel_and_ack_pending_disconnects(
    command_rx: &mut mpsc::UnboundedReceiver<SoraConnectionCommand>,
    user_initiated_disconnect: &mut bool,
) {
    command_rx.close();
    while let Some(command) = command_rx.recv().await {
        if let SoraConnectionCommand::Disconnect(ack_tx) = command {
            // ユーザー主導の切断として記録し、close handshake 中の
            // I/O エラーを warning に落とす対象に加える。
            *user_initiated_disconnect = true;
            let _ = ack_tx.send(());
        }
        // Disconnect 以外のコマンドは応答せず破棄する。
    }
}

struct ManagedDataChannel {
    channel: DataChannel,
    // Observer を保持しておく必要がある (ドロップすると DataChannel への通知が止まる)
    #[expect(dead_code)]
    observer: DataChannelObserver,
    compress: bool,
}

impl Drop for ManagedDataChannel {
    fn drop(&mut self) {
        // DataChannelObserver を解放する前に、C++ 側の DataChannel から observer を解除する。
        // SctpDataChannel::ObserverAdapter の状態遷移コールバックはネットワークスレッドから
        // シグナリングスレッドへ非同期に配送される。observer を先に解放すると、
        // 配送済みの遅延コールバックが解放済みメモリを参照して SIGSEGV になる。
        // UnregisterObserver で observer_ を null にすることで、遅延コールバックの
        // 実行時に observer 呼び出しがスキップされる。
        self.channel.unregister_observer();
    }
}

const DEFAULT_TLS_PORT: u16 = 443;
const DEFAULT_PLAIN_PORT: u16 = 80;

// DataChannel メッセージの展開後サイズ上限。
// WebSocket 実装の DEFAULT_MAX_DECOMPRESSED_SIZE と同じ 16 MiB にそろえる。
// WebSocket 側の定数は SDK から参照できないため、独立した定数として定義する。
const MAX_DECOMPRESSED_DATA_CHANNEL_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

#[derive(Clone)]
struct SecureRandom {
    rng: SystemRandom,
}

impl SecureRandom {
    fn new() -> Self {
        Self {
            rng: SystemRandom::new(),
        }
    }
}

impl RandomSource for SecureRandom {
    fn masking_key(&mut self) -> [u8; 4] {
        let mut key = [0u8; 4];
        self.rng
            .fill(&mut key)
            .expect("failed to generate masking key: aws-lc-rs SystemRandom::fill failed, OS RNG may be unavailable or exhausted");
        key
    }

    fn nonce(&mut self) -> [u8; 16] {
        let mut nonce = [0u8; 16];
        self.rng
            .fill(&mut nonce)
            .expect("failed to generate nonce: aws-lc-rs SystemRandom::fill failed, OS RNG may be unavailable or exhausted");
        nonce
    }
}

fn now() -> Result<Timestamp> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    Ok(Timestamp::from_millis(millis))
}

fn default_port(tls: bool) -> u16 {
    if tls {
        DEFAULT_TLS_PORT
    } else {
        DEFAULT_PLAIN_PORT
    }
}

fn normalize_host(host: &str) -> Result<String> {
    let host = host.trim();
    if host.is_empty() {
        return Err(Error::HostEmpty);
    }
    if let Some(stripped) = host.strip_prefix('[') {
        let stripped = stripped
            .strip_suffix(']')
            .ok_or_else(|| Error::HostInvalidFormat)?;
        if stripped.is_empty() {
            return Err(Error::HostEmpty);
        }
        return Ok(stripped.to_string());
    }
    Ok(host.to_string())
}

fn format_bracketed_host(host: &str) -> String {
    if host.contains(':') {
        format!("[{}]", host)
    } else {
        host.to_string()
    }
}

fn format_host_header(host: &str, port: u16, tls: bool) -> String {
    let host = format_bracketed_host(host);
    if port == default_port(tls) {
        host
    } else {
        format!("{}:{}", host, port)
    }
}

struct SignalingTarget {
    host: String,
    port: u16,
    path: String,
    tls: bool,
}

/// `ProxyInfo` を解析し、HTTP プロキシ接続に必要な情報に正規化した結果。
///
/// PBT 等の検証目的を主用途として公開している型のため、通常の利用者がこの型を
/// フィールド値の取得は accessor メソッド (`host()` / `port()` / `username()` /
/// `password()` / `user_agent()`) 経由で行う。
#[derive(Clone)]
pub struct ParsedProxyInfo {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) user_agent: String,
}

// 機密情報 (username / password) を Debug 出力時にマスクする。
// ProxyInfo (src/types.rs) と同じパターン。
impl std::fmt::Debug for ParsedProxyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedProxyInfo")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username.as_ref().map(|_| "<redacted>"))
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("user_agent", &self.user_agent)
            .finish()
    }
}

impl ParsedProxyInfo {
    /// プロキシホスト名を返す。
    pub fn host(&self) -> &str {
        &self.host
    }

    /// プロキシポート番号を返す。
    pub fn port(&self) -> u16 {
        self.port
    }

    /// プロキシユーザー名を返す。
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// プロキシパスワードを返す。
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// プロキシ接続時の User-Agent ヘッダ値を返す。
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// `ProxyInfo` を解析し、検証済みのプロキシ接続情報を返す。
    ///
    /// 受理するのは `http://host[:port]` 形式のみで、`https://` / `socks*://` や
    /// userinfo / fragment / query / 非空パスを含む URL は拒否する。
    pub fn parse(proxy: &ProxyInfo) -> Result<ParsedProxyInfo> {
        let uri = Uri::parse(&proxy.url)?;
        let scheme = uri.scheme().ok_or_else(|| Error::UrlMissingScheme)?;
        if !scheme.eq_ignore_ascii_case("http") {
            return Err(Error::ProxyUrlUnsupportedScheme {
                scheme: scheme.to_string(),
            });
        }
        if let Some(authority) = uri.authority()
            && authority.contains('@')
        {
            return Err(Error::ProxyUrlUserinfoNotSupported);
        }
        if uri.fragment().is_some() {
            return Err(Error::ProxyUrlFragmentNotAllowed);
        }
        if uri.query().is_some() {
            return Err(Error::ProxyUrlQueryNotAllowed);
        }
        let path = uri.path();
        if !path.is_empty() && path != "/" {
            return Err(Error::ProxyUrlPathNotAllowed {
                path: path.to_string(),
            });
        }
        let host = uri.host().ok_or_else(|| Error::ProxyUrlMissingHost)?;
        let host = normalize_host(host)?;
        let port = uri.port().unwrap_or_else(|| default_port(false));
        let user_agent = proxy
            .user_agent
            .clone()
            .unwrap_or_else(crate::version::get_sora_client_name);
        Ok(ParsedProxyInfo {
            host,
            port,
            username: proxy.username.clone(),
            password: proxy.password.clone(),
            user_agent,
        })
    }
}

fn parse_signaling_url(url: &str) -> Result<SignalingTarget> {
    let uri = Uri::parse(url)?;
    let scheme = uri.scheme().ok_or_else(|| Error::UrlMissingScheme)?;
    let tls = if scheme.eq_ignore_ascii_case("wss") {
        true
    } else if scheme.eq_ignore_ascii_case("ws") {
        false
    } else {
        return Err(Error::UrlUnsupportedScheme {
            scheme: scheme.to_string(),
        });
    };
    if let Some(authority) = uri.authority()
        && authority.contains('@')
    {
        return Err(Error::UrlUserinfoNotSupported);
    }
    if uri.fragment().is_some() {
        return Err(Error::UrlFragmentNotAllowed);
    }

    let host = uri.host().ok_or_else(|| Error::UrlMissingHost)?;
    let host = normalize_host(host)?;
    let port = uri.port().unwrap_or_else(|| default_port(tls));
    let path = uri.origin_form();

    Ok(SignalingTarget {
        host,
        port,
        path,
        tls,
    })
}

enum ClientStreamInner {
    Tls(Box<TlsStream<TcpStream>>),
    Plain(TcpStream),
}

struct ClientStream {
    inner: ClientStreamInner,
    pending_read: Vec<u8>,
}

impl ClientStream {
    fn new_plain(stream: TcpStream) -> Self {
        Self {
            inner: ClientStreamInner::Plain(stream),
            pending_read: Vec::new(),
        }
    }

    fn new_tls(stream: TlsStream<TcpStream>) -> Self {
        Self {
            inner: ClientStreamInner::Tls(Box::new(stream)),
            pending_read: Vec::new(),
        }
    }

    fn push_pending_read(&mut self, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        if self.pending_read.is_empty() {
            self.pending_read = data;
            return;
        }
        self.pending_read.extend_from_slice(&data);
    }

    fn into_plain_parts(self) -> Option<(TcpStream, Vec<u8>)> {
        match self.inner {
            ClientStreamInner::Plain(stream) => Some((stream, self.pending_read)),
            ClientStreamInner::Tls(_) => None,
        }
    }

    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.pending_read.is_empty() {
            let n = self.pending_read.len().min(buf.len());
            buf[..n].copy_from_slice(&self.pending_read[..n]);
            self.pending_read.drain(..n);
            return Ok(n);
        }
        match &mut self.inner {
            ClientStreamInner::Tls(stream) => stream.read(buf).await,
            ClientStreamInner::Plain(stream) => stream.read(buf).await,
        }
    }

    async fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        match &mut self.inner {
            ClientStreamInner::Tls(stream) => stream.write_all(buf).await,
            ClientStreamInner::Plain(stream) => stream.write_all(buf).await,
        }
    }
}

struct TimerManager {
    ping: Option<JoinHandle<()>>,
    pong_timeout: Option<JoinHandle<()>>,
    close_timeout: Option<JoinHandle<()>>,
    sender: mpsc::Sender<TimerId>,
}

impl TimerManager {
    fn new(sender: mpsc::Sender<TimerId>) -> Self {
        Self {
            ping: None,
            pong_timeout: None,
            close_timeout: None,
            sender,
        }
    }

    fn set_timer(&mut self, id: TimerId, duration: u64) {
        // 同じ id の実行中タイマーが残ったまま上書きすると、古いタイマーの
        // 発火で誤ったタイマーイベントが届くため、上書き前に abort する。
        self.clear_timer(id);
        let sender = self.sender.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(duration)).await;
            let _ = sender.send(id).await;
        });
        match id {
            TimerId::Ping => self.ping = Some(handle),
            TimerId::PongTimeout => self.pong_timeout = Some(handle),
            TimerId::CloseTimeout => self.close_timeout = Some(handle),
        }
    }

    fn clear_timer(&mut self, id: TimerId) {
        let handle = match id {
            TimerId::Ping => &mut self.ping,
            TimerId::PongTimeout => &mut self.pong_timeout,
            TimerId::CloseTimeout => &mut self.close_timeout,
        };
        if let Some(handle) = handle.take() {
            handle.abort();
        }
    }
}

impl Drop for TimerManager {
    fn drop(&mut self) {
        if let Some(h) = self.ping.take() {
            h.abort();
        }
        if let Some(h) = self.pong_timeout.take() {
            h.abort();
        }
        if let Some(h) = self.close_timeout.take() {
            h.abort();
        }
    }
}

/// insecure モード用の証明書検証器。全ての証明書を受け入れる。
#[derive(Debug)]
struct NoServerCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// PEM 文字列から証明書の一覧をパースする。
fn parse_certs_from_pem(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::ClientCertParse)?;
    if certs.is_empty() {
        return Err(Error::ClientCertParse);
    }
    Ok(certs)
}

/// PEM 文字列から秘密鍵をパースする。
fn parse_private_key_from_pem(pem: &str) -> Result<PrivateKeyDer<'static>> {
    PrivateKeyDer::from_pem_slice(pem.as_bytes()).map_err(|_| Error::ClientKeyParse)
}

/// TlsConfig から rustls の ClientConfig を構築する。
fn build_tls_client_config(tls_config: &TlsConfig) -> Result<ClientConfig> {
    // client_cert と client_key の片方だけ設定されている場合はエラー
    if tls_config.client_cert.is_some() != tls_config.client_key.is_some() {
        return Err(Error::ClientCertKeyIncomplete);
    }

    let builder = if tls_config.insecure {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoServerCertVerifier))
    } else if let Some(ref ca_pem) = tls_config.ca_cert {
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(ca_pem.as_bytes())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::CaCertParse)?;
        if certs.is_empty() {
            return Err(Error::CaCertParse);
        }
        let mut root_store = rustls::RootCertStore::empty();
        for cert in certs {
            root_store.add(cert).map_err(|_| Error::CaCertParse)?;
        }
        ClientConfig::builder().with_root_certificates(root_store)
    } else {
        return build_tls_client_config_with_client_auth(
            ClientConfig::with_platform_verifier()?,
            tls_config,
        );
    };

    let config = if let (Some(cert_pem), Some(key_pem)) =
        (&tls_config.client_cert, &tls_config.client_key)
    {
        let certs = parse_certs_from_pem(cert_pem)?;
        let key = parse_private_key_from_pem(key_pem)?;
        builder.with_client_auth_cert(certs, key)?
    } else {
        builder.with_no_client_auth()
    };

    Ok(config)
}

/// プラットフォーム verifier で構築済みの ClientConfig にクライアント証明書を適用する。
///
/// `ClientConfig::with_platform_verifier()` は `with_client_auth_cert()` 等のビルダーを
/// 経由せず直接 `ClientConfig` を返すため、ダミーの `ClientConfig` を
/// `with_client_auth_cert()` で構築して `client_auth_cert_resolver` を移す。
fn build_tls_client_config_with_client_auth(
    mut config: ClientConfig,
    tls_config: &TlsConfig,
) -> Result<ClientConfig> {
    if let (Some(cert_pem), Some(key_pem)) = (&tls_config.client_cert, &tls_config.client_key) {
        let certs = parse_certs_from_pem(cert_pem)?;
        let key = parse_private_key_from_pem(key_pem)?;
        // ダミーの ClientConfig を構築して client_auth_cert_resolver を取り出す
        let dummy = ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_client_auth_cert(certs, key)?;
        config.client_auth_cert_resolver = dummy.client_auth_cert_resolver;
    }
    Ok(config)
}

async fn connect_websocket(
    target: &SignalingTarget,
    proxy: Option<&ParsedProxyInfo>,
    tls_config: &TlsConfig,
    timeout: Duration,
) -> Result<ClientStream> {
    let deadline = tokio::time::Instant::now() + timeout;
    if let Some(proxy) = proxy {
        rtc_log_info!(
            "Connecting via HTTP proxy: {}:{}",
            format_bracketed_host(proxy.host()),
            proxy.port
        );
        let tcp_stream = connect_tcp(proxy.host(), proxy.port(), deadline).await?;
        let mut stream = ClientStream::new_plain(tcp_stream);
        connect_http_proxy_tunnel(&mut stream, target, proxy, deadline).await?;
        if target.tls {
            let (tcp_stream, pending) = stream
                .into_plain_parts()
                .expect("BUG: stream must be plain after proxy connection");
            // TLS 接続では ClientHello をクライアントが先に送るため、
            // CONNECT 200 応答直後にサーバーからバイトが届くことはありえない。
            // 余剰バイトはプロキシの応答注入であるため接続を拒否する。
            if !pending.is_empty() {
                return Err(Error::ProxyConnectUnexpectedTrailingData);
            }
            let tls_stream = connect_tls(&target.host, tcp_stream, tls_config, deadline).await?;
            let stream = ClientStream::new_tls(tls_stream);
            Ok(stream)
        } else {
            Ok(stream)
        }
    } else {
        let tcp_stream = connect_tcp(&target.host, target.port, deadline).await?;
        if target.tls {
            let tls_stream = connect_tls(&target.host, tcp_stream, tls_config, deadline).await?;
            Ok(ClientStream::new_tls(tls_stream))
        } else {
            Ok(ClientStream::new_plain(tcp_stream))
        }
    }
}

async fn connect_tcp(host: &str, port: u16, deadline: tokio::time::Instant) -> Result<TcpStream> {
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| Error::DnsResolve {
            host: host.to_string(),
            source: e,
        })?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(Error::NoResolvedAddress {
            host: host.to_string(),
            port,
        });
    }
    let tcp_stream = tokio::time::timeout_at(deadline, TcpStream::connect(addrs.as_slice()))
        .await
        .map_err(|_| Error::TcpConnectTimeout {
            host: host.to_string(),
            port,
        })?
        .map_err(|e| Error::TcpConnect {
            host: host.to_string(),
            port,
            source: e,
        })?;
    Ok(tcp_stream)
}

async fn connect_tls(
    host: &str,
    tcp_stream: TcpStream,
    tls_config: &TlsConfig,
    deadline: tokio::time::Instant,
) -> Result<TlsStream<TcpStream>> {
    let client_config = build_tls_client_config(tls_config)?;
    let connector = TlsConnector::from(Arc::new(client_config));
    let server_name = ServerName::try_from(host.to_string())?;
    tokio::time::timeout_at(deadline, connector.connect(server_name, tcp_stream))
        .await
        .map_err(|_| Error::TlsConnectTimeout {
            host: host.to_string(),
        })?
        .map_err(|e| Error::TlsConnect {
            host: host.to_string(),
            source: e,
        })
}

fn build_proxy_connect_request(
    target: &SignalingTarget,
    proxy: &ParsedProxyInfo,
) -> Result<Vec<u8>> {
    let authority = format!("{}:{}", format_bracketed_host(&target.host), target.port);
    let mut request = Request::new("CONNECT", &authority)?
        .header("Host", &authority)?
        .header("User-Agent", proxy.user_agent())?;
    if proxy.username().is_some() || proxy.password().is_some() {
        let username = proxy.username().unwrap_or("");
        let password = proxy.password().unwrap_or("");
        let auth = BasicAuth::new(username, password)?;
        let header = auth.to_header_value();
        request = request.header("Proxy-Authorization", &header)?;
    }
    Ok(request.encode()?)
}

fn ensure_proxy_connect_status_success(status_code: u16, reason_phrase: &str) -> Result<()> {
    if (200..300).contains(&status_code) {
        Ok(())
    } else {
        Err(Error::ProxyConnectStatusNotSuccessful {
            status_code,
            reason_phrase: reason_phrase.to_string(),
        })
    }
}

async fn connect_http_proxy_tunnel(
    stream: &mut ClientStream,
    target: &SignalingTarget,
    proxy: &ParsedProxyInfo,
    deadline: tokio::time::Instant,
) -> Result<()> {
    let request = build_proxy_connect_request(target, proxy)?;
    stream.write_all(&request).await?;

    let mut decoder = ResponseDecoder::new();
    decoder.set_request_method("CONNECT");
    let mut buf = vec![0u8; 8192];
    loop {
        // CONNECT 応答を返さないプロキシで応答待ちが永久に続かないように、
        // read を接続全体の期限 (deadline) で囲む。
        let n = tokio::time::timeout_at(deadline, stream.read(&mut buf))
            .await
            .map_err(|_| Error::ProxyConnectTimeout {
                host: proxy.host().to_string(),
                port: proxy.port(),
            })??;
        if n == 0 {
            return Err(Error::ProxyConnectResponseMissing);
        }
        decoder.feed(&buf[..n])?;
        if let Some((head, _body_kind)) = decoder.decode_headers()? {
            ensure_proxy_connect_status_success(head.status_code(), head.reason_phrase())?;
            let remaining = decoder.take_remaining();
            stream.push_pending_read(remaining);
            return Ok(());
        }
    }
}

/// 複数のシグナリング URL に同時に接続を試み、最初に成功した接続を返す。
///
/// 戻り値は (ストリーム, パース済みターゲット, 選択された URL) のタプル。
async fn connect_signaling_urls(
    urls: &[String],
    proxy: Option<ParsedProxyInfo>,
    tls_config: Arc<TlsConfig>,
    timeout: Duration,
) -> Result<(ClientStream, SignalingTarget, String)> {
    let mut join_set = tokio::task::JoinSet::new();
    for url in urls.iter().cloned() {
        let proxy = proxy.clone();
        let tls_config = tls_config.clone();
        join_set.spawn(async move {
            let result = async {
                let target = parse_signaling_url(&url)?;
                let display_host = format_bracketed_host(&target.host);
                let scheme = if target.tls { "wss" } else { "ws" };
                rtc_log_info!(
                    "Connection attempt: {}://{}:{}{}",
                    scheme,
                    display_host,
                    target.port,
                    target.path
                );
                let stream =
                    connect_websocket(&target, proxy.as_ref(), &tls_config, timeout).await?;
                Ok::<_, Error>((stream, target))
            }
            .await;
            (url, result)
        });
    }

    let mut errors = Vec::new();
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok((url, Ok((stream, target)))) => {
                let display_host = format_bracketed_host(&target.host);
                let scheme = if target.tls { "wss" } else { "ws" };
                rtc_log_info!(
                    "Connection established: {}://{}:{}{}",
                    scheme,
                    display_host,
                    target.port,
                    target.path
                );
                // 残りの接続試行をキャンセルする
                join_set.abort_all();
                return Ok((stream, target, url));
            }
            Ok((url, Err(e))) => {
                rtc_log_warning!("Connection failed: {}: {}", url, e);
                errors.push((url, e.to_string()));
            }
            Err(join_err) => {
                if !join_err.is_cancelled() {
                    errors.push(("(unknown)".to_string(), join_err.to_string()));
                }
            }
        }
    }

    Err(Error::AllSignalingUrlsFailed { errors })
}

async fn flush_ws_output<R: RandomSource>(
    ws: &mut WebSocketClientConnection<R>,
    stream: &mut ClientStream,
    timers: &mut TimerManager,
) -> Result<bool> {
    while let Some(output) = ws.poll_output() {
        match output {
            ConnectionOutput::SendData(buf) => {
                stream.write_all(&buf).await?;
            }
            ConnectionOutput::SetTimer {
                id,
                duration_millis,
            } => {
                timers.set_timer(id, duration_millis);
            }
            ConnectionOutput::ClearTimer { id } => {
                timers.clear_timer(id);
            }
            ConnectionOutput::CloseConnection => {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// WebSocket 接続の close handshake を実行する。
///
/// WebSocket が `Connected` の場合のみ Close フレーム (NORMAL, "shutdown") を送信し、
/// 相手からの Close フレームまたは TCP の EOF を待って接続を終了する。Close フレーム
/// の送受信が完了するか相手が EOF を返すと `Ok(())` を返す。
///
/// WebSocket が `Connected` 以外の場合は何もせず `Ok(())` を返す。
///
/// close handshake 全体は `websocket_close_timeout` を上限とする。タイムアウトした
/// 場合は警告を出力して `Ok(())` を返す。
///
/// close handshake 中に I/O エラーが発生した場合は、`server_close_received` または
/// `absorb_close_handshake_errors` が `true` なら警告を出力して `Ok(())` を返し、
/// どちらも `false` なら `Err` を返す。
///
/// 引数:
/// - `server_close_received`: DataChannel 経由の server Close による終了かどうか。
///   相手からの Close フレームを `handler.on_websocket_close` で通知するかの判定と、
///   I/O エラーを警告に落とすかの判定に使う。
/// - `absorb_close_handshake_errors`: close handshake 中の I/O エラーを警告に落として
///   `Ok(())` を返すべきかどうか。呼び出し元が
///   `(switched_ignore_disconnect_websocket && use_data_channel_signaling) ||
///   user_initiated_disconnect` の結果を渡す。
async fn close_websocket_handshake<R: RandomSource>(
    ws: &mut WebSocketClientConnection<R>,
    stream: &mut ClientStream,
    timers: &mut TimerManager,
    handler: &mut dyn SoraConnectionEventHandler,
    server_close_received: bool,
    absorb_close_handshake_errors: bool,
    websocket_close_timeout: Duration,
) -> Result<()> {
    if ws.state() == ConnectionState::Connected {
        // server Close は server_close_received で終了経路が確定しているため、
        // この後始末で発生するエラーは無視してよい。
        // ignore 構成とユーザー主導の切断は、切断とサーバー側の RST が同時に
        // 起きた場合に websocket_closed が立つ前に close handshake へ入り、
        // 死んだソケットへの書き込みが失敗することがある。
        // いずれも接続を終了する意思が確定しているため、close handshake の失敗は
        // warning に落として run の Ok(()) を覆さない。
        let close_result = tokio::time::timeout(websocket_close_timeout, async {
            ws.close(CloseCode::NORMAL, "shutdown")?;
            loop {
                if flush_ws_output(ws, stream, timers).await? {
                    return Ok::<_, Error>(());
                }
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await?;
                if n == 0 {
                    return Ok(());
                }
                ws.feed_recv_buf(&buf[..n], now()?)?;
                // server Close の終了処理では、Sora が送信した WebSocket Close
                // フレームを検出して on_websocket_close を通知する。
                // server Close メッセージと WebSocket Close フレームは別経路で
                // 届くため、処理順のレースで Close フレームが未処理のまま残る
                // 場合がある。ここで読み取ることで、on_websocket_close を
                // 決定的に 1 回だけ通知する。
                // server Close 以外の終了経路では従来どおりイベントを破棄する。
                while let Some(event) = ws.poll_event() {
                    if server_close_received && let ConnectionEvent::Close { code, reason } = event
                    {
                        handler.on_websocket_close(code.map(|c| c.0), &reason);
                    }
                }
                if ws.state() == ConnectionState::Closed {
                    return Ok(());
                }
            }
        })
        .await;
        match close_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if server_close_received || absorb_close_handshake_errors {
                    rtc_log_warning!("WebSocket close handshake failed: {}", e);
                } else {
                    return Err(e);
                }
            }
            Err(_) => {
                rtc_log_warning!("WebSocket close timed out");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_info_with_url(url: String) -> ProxyInfo {
        ProxyInfo {
            url,
            ..Default::default()
        }
    }

    fn is_turn_tcp_or_udp_url(url: &str) -> bool {
        let Ok(uri) = Uri::parse(url) else {
            return false;
        };
        let Some(scheme) = uri.scheme() else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("turn") && !scheme.eq_ignore_ascii_case("turns") {
            return false;
        }
        uri.query()
            .and_then(|query| {
                query
                    .split('&')
                    .find_map(|param| param.strip_prefix("transport="))
            })
            .is_some_and(|transport| {
                transport.eq_ignore_ascii_case("tcp") || transport.eq_ignore_ascii_case("udp")
            })
    }

    #[test]
    fn ice_server_url_configurer_none_adds_all_urls() {
        let mut server_entry = IceServer::new();
        let urls = vec![
            "stun:stun.example.com:3478".to_string(),
            "turn:turn.example.com:3478?transport=udp".to_string(),
            "turns:turn.example.com:443?transport=tcp".to_string(),
        ];
        SoraConnection::configure_ice_server_urls(&mut server_entry, &urls, None);

        assert_eq!(server_entry.urls_len(), urls.len());
    }

    #[test]
    fn ice_server_url_configurer_can_add_only_turn_tcp_udp_urls() {
        let mut server_entry = IceServer::new();
        let urls = vec![
            "stun:stun.example.com:3478".to_string(),
            "turn:turn.example.com:3478?transport=udp".to_string(),
            "turn:turn.example.com:3478?transport=tcp".to_string(),
            "turn:turn.example.com:3478".to_string(),
            "turns:turn.example.com:443?transport=tcp".to_string(),
        ];
        let configurer: Box<IceServerUrlConfigurer> = Box::new(|server_entry, urls| {
            for url in urls {
                if is_turn_tcp_or_udp_url(url) {
                    server_entry.add_url(url);
                }
            }
        });
        SoraConnection::configure_ice_server_urls(&mut server_entry, &urls, Some(&configurer));
        assert_eq!(server_entry.urls_len(), 3);
    }

    #[test]
    fn ice_server_url_configurer_skips_server_when_no_url_is_added() {
        let mut server_entry = IceServer::new();
        let urls = vec![
            "stun:stun.example.com:3478".to_string(),
            "stuns:stun.example.com:5349".to_string(),
        ];
        let configurer: Box<IceServerUrlConfigurer> = Box::new(|_, _| {});
        SoraConnection::configure_ice_server_urls(&mut server_entry, &urls, Some(&configurer));
        assert_eq!(server_entry.urls_len(), 0);
    }

    #[test]
    fn parse_proxy_info_uses_default_user_agent_when_absent() {
        let proxy = proxy_info_with_url("http://proxy.example.com:8080".to_string());
        let parsed = ParsedProxyInfo::parse(&proxy).expect("proxy URL の解析に失敗しました");
        assert_eq!(parsed.user_agent(), crate::version::get_sora_client_name());
    }

    #[test]
    fn parse_proxy_info_preserves_empty_user_agent_when_present() {
        let proxy = ProxyInfo {
            url: "http://proxy.example.com:8080".to_string(),
            user_agent: Some(String::new()),
            ..Default::default()
        };
        let parsed = ParsedProxyInfo::parse(&proxy).expect("proxy URL の解析に失敗しました");
        assert_eq!(parsed.user_agent(), "");
    }

    #[test]
    fn parsed_proxy_info_debug_masks_credentials() {
        let parsed = ParsedProxyInfo {
            host: "proxy.example.com".to_string(),
            port: 8080,
            username: Some("secret_user".to_string()),
            password: Some("secret_pass".to_string()),
            user_agent: "ua".to_string(),
        };
        let debug_str = format!("{:?}", parsed);
        assert!(debug_str.contains("<redacted>"));
        assert!(!debug_str.contains("secret_user"));
        assert!(!debug_str.contains("secret_pass"));
        assert!(debug_str.contains("proxy.example.com"));
        assert!(debug_str.contains("8080"));
    }

    #[test]
    fn build_proxy_connect_request_omits_proxy_auth_when_credentials_absent() {
        let target = SignalingTarget {
            host: "sora.example.com".to_string(),
            port: 443,
            path: "/signaling".to_string(),
            tls: true,
        };
        let proxy = ParsedProxyInfo {
            host: "proxy.example.com".to_string(),
            port: 8080,
            username: None,
            password: None,
            user_agent: "ua-test".to_string(),
        };
        let encoded = build_proxy_connect_request(&target, &proxy)
            .expect("CONNECT リクエスト生成に失敗しました");
        let request = String::from_utf8(encoded).expect("HTTP リクエストが UTF-8 ではありません");
        assert!(!request.contains("Proxy-Authorization:"));
        assert!(!request.contains("Content-Length:"));
        assert!(!request.contains("Transfer-Encoding:"));
    }

    #[test]
    fn build_proxy_connect_request_includes_proxy_auth_for_explicit_empty_credentials() {
        let target = SignalingTarget {
            host: "sora.example.com".to_string(),
            port: 443,
            path: "/signaling".to_string(),
            tls: true,
        };
        let proxy = ParsedProxyInfo {
            host: "proxy.example.com".to_string(),
            port: 8080,
            username: Some(String::new()),
            password: Some(String::new()),
            user_agent: "ua-test".to_string(),
        };
        let encoded = build_proxy_connect_request(&target, &proxy)
            .expect("CONNECT リクエスト生成に失敗しました");
        let request = String::from_utf8(encoded).expect("HTTP リクエストが UTF-8 ではありません");
        assert!(request.contains("Proxy-Authorization: Basic Og=="));
        assert!(!request.contains("Content-Length:"));
        assert!(!request.contains("Transfer-Encoding:"));
    }

    #[tokio::test]
    async fn url_getters_return_send_error_after_run_loop_stops() {
        let (command_tx, command_rx) = mpsc::unbounded_channel::<SoraConnectionCommand>();
        drop(command_rx);
        let handle = SoraConnectionHandle { command_tx };

        let selected_error = handle
            .selected_signaling_url()
            .await
            .expect_err("selected_signaling_url は失敗する必要があります");
        assert!(matches!(
            selected_error,
            Error::CommandSendFailed {
                command: "selected_signaling_url",
                ..
            }
        ));

        let connected_error = handle
            .connected_signaling_url()
            .await
            .expect_err("connected_signaling_url は失敗する必要があります");
        assert!(matches!(
            connected_error,
            Error::CommandSendFailed {
                command: "connected_signaling_url",
                ..
            }
        ));
    }

    #[test]
    fn connect_response_decoder_accepts_2xx() {
        let mut decoder = ResponseDecoder::new();
        decoder.set_request_method("CONNECT");
        decoder
            .feed(b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: test\r\n\r\n")
            .expect("レスポンス feed に失敗しました");
        let (head, body_kind) = decoder
            .decode_headers()
            .expect("レスポンスヘッダーの decode に失敗しました")
            .expect("レスポンスヘッダーが完成していません");
        assert_eq!(body_kind, shiguredo_http11::BodyKind::Tunnel);
        ensure_proxy_connect_status_success(head.status_code(), head.reason_phrase())
            .expect("2xx は成功扱いである必要があります");
    }

    #[test]
    fn connect_response_decoder_rejects_non_2xx() {
        let mut decoder = ResponseDecoder::new();
        decoder.set_request_method("CONNECT");
        decoder
            .feed(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n")
            .expect("レスポンス feed に失敗しました");
        let (head, body_kind) = decoder
            .decode_headers()
            .expect("レスポンスヘッダーの decode に失敗しました")
            .expect("レスポンスヘッダーが完成していません");
        assert!(matches!(
            body_kind,
            shiguredo_http11::BodyKind::ContentLength(0)
        ));
        let err = ensure_proxy_connect_status_success(head.status_code(), head.reason_phrase())
            .expect_err("非 2xx は失敗扱いである必要があります");
        assert!(matches!(
            err,
            Error::ProxyConnectStatusNotSuccessful {
                status_code: 407,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn connect_http_proxy_tunnel_times_out_when_proxy_never_responds() {
        // CONNECT 応答を返さないプロキシを実 TCP リスナーで再現する。
        // accept 後に CONNECT リクエストを読み込んだまま、応答を返さず接続を保持し続ける。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("テスト用 TCP リスナーのバインドに失敗しました");
        let listener_addr = listener
            .local_addr()
            .expect("テスト用 TCP リスナーのアドレス取得に失敗しました");
        let proxy_hold_task = tokio::spawn(async move {
            let (mut stream, _peer_addr) = listener
                .accept()
                .await
                .expect("テスト用 TCP リスナーでの accept に失敗しました");
            let mut buf = [0u8; 4096];
            // CONNECT リクエストを受信した後、応答を返さずに接続を保持し続ける。
            let _ = stream.read(&mut buf).await;
            std::future::pending::<()>().await;
        });

        let target = SignalingTarget {
            host: "sora.example.com".to_string(),
            port: 443,
            path: "/signaling".to_string(),
            tls: true,
        };
        let proxy = ParsedProxyInfo {
            host: listener_addr.ip().to_string(),
            port: listener_addr.port(),
            username: None,
            password: None,
            user_agent: "ua-test".to_string(),
        };
        let tcp_stream = tokio::net::TcpStream::connect(listener_addr)
            .await
            .expect("テスト用 TCP リスナーへの接続に失敗しました");
        let mut stream = ClientStream::new_plain(tcp_stream);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        let err = connect_http_proxy_tunnel(&mut stream, &target, &proxy, deadline)
            .await
            .expect_err("CONNECT 応答待ちがタイムアウトする必要があります");
        let message = err.to_string();
        let expected_host = listener_addr.ip().to_string();
        let expected_port = listener_addr.port();
        let Error::ProxyConnectTimeout { host, port } = err else {
            panic!("ProxyConnectTimeout が返る必要があります: {message}");
        };
        assert_eq!(
            host, expected_host,
            "ProxyConnectTimeout の host が期待値と一致しません: {message}"
        );
        assert_eq!(
            port, expected_port,
            "ProxyConnectTimeout の port が期待値と一致しません: {message}"
        );
        assert!(
            message.contains(&format!("{expected_host}:{expected_port}")),
            "タイムアウトメッセージに host:port が含まれていません: {message}"
        );

        proxy_hold_task.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timer_manager_drop_aborts_all_timers() {
        let (timer_tx, mut timer_rx) = mpsc::channel::<TimerId>(16);
        let mut timers = TimerManager::new(timer_tx);
        timers.set_timer(TimerId::Ping, 50);
        timers.set_timer(TimerId::PongTimeout, 50);
        timers.set_timer(TimerId::CloseTimeout, 50);
        drop(timers);
        let mut received = Vec::new();
        while let Ok(Some(id)) =
            tokio::time::timeout(std::time::Duration::from_millis(150), timer_rx.recv()).await
        {
            received.push(id);
        }
        assert!(
            received.len() < 3,
            "全タイマーがメッセージを送信した場合、abort されていないことを示す: {} 件",
            received.len()
        );
    }

    #[test]
    fn timer_manager_drop_with_no_timers_does_not_panic() {
        let (timer_tx, _timer_rx) = mpsc::channel::<TimerId>(16);
        let timers = TimerManager::new(timer_tx);
        drop(timers);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timer_manager_clear_timer_then_drop_does_not_panic() {
        let (timer_tx, _timer_rx) = mpsc::channel::<TimerId>(16);
        let mut timers = TimerManager::new(timer_tx);
        timers.set_timer(TimerId::Ping, 1000);
        timers.set_timer(TimerId::PongTimeout, 1000);
        timers.set_timer(TimerId::CloseTimeout, 1000);
        timers.clear_timer(TimerId::Ping);
        timers.clear_timer(TimerId::PongTimeout);
        timers.clear_timer(TimerId::CloseTimeout);
        drop(timers);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timer_manager_zero_duration_timer_does_not_panic_on_drop() {
        let (timer_tx, _timer_rx) = mpsc::channel::<TimerId>(16);
        let mut timers = TimerManager::new(timer_tx);
        timers.set_timer(TimerId::Ping, 0);
        timers.set_timer(TimerId::PongTimeout, 0);
        timers.set_timer(TimerId::CloseTimeout, 0);
        tokio::task::yield_now().await;
        drop(timers);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn timer_manager_set_timer_same_id_aborts_previous() {
        let (timer_tx, mut timer_rx) = mpsc::channel::<TimerId>(16);
        let mut timers = TimerManager::new(timer_tx);
        // 同じ id を 2 回設定すると最初のタイマーは abort される。
        timers.set_timer(TimerId::Ping, 50);
        timers.set_timer(TimerId::Ping, 100);
        // 最初のタイマーの発火時刻 (50ms) を過ぎても 1 件目は届かない。
        // 仮想時刻 (start_paused) を使うため CI 負荷で sleep がオーバーシュートしても
        // 決定論的に検証できる。
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert!(
            matches!(timer_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "上書き前のタイマーが abort されていません"
        );
        // 2 件目のタイマーの発火時刻を過ぎると 1 件だけ届く。
        let received = tokio::time::timeout(Duration::from_millis(200), timer_rx.recv())
            .await
            .expect("タイマーイベントが届きませんでした")
            .expect("タイマーイベントの受信に失敗しました");
        assert_eq!(received, TimerId::Ping, "受信した TimerId が一致しません");
        assert!(
            matches!(timer_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "同じ id のタイマーイベントが 2 件届いています"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_rpc_request_drop_aborts_timeout_handle() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SoraEvent>();
        let (response_tx, _response_rx) = oneshot::channel();
        let timeout_handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = event_tx.send(SoraEvent::RpcTimeout { id: 42 });
        });
        let pending = PendingRpcRequest {
            response_tx: Some(response_tx),
            timeout_handle,
        };
        drop(pending);
        if let Ok(Some(SoraEvent::RpcTimeout { id: 42 })) =
            tokio::time::timeout(std::time::Duration::from_millis(150), event_rx.recv()).await
        {
            panic!("RpcTimeout が届いた。timeout_handle が abort されていない");
        }
    }

    #[test]
    fn secure_random_masking_key_returns_valid_data() {
        let mut sr = SecureRandom::new();
        let key1 = sr.masking_key();
        let key2 = sr.masking_key();
        assert_ne!(
            key1, key2,
            "masking_key の連続呼び出しで異なる値が返る必要がある"
        );
    }

    #[test]
    fn secure_random_nonce_returns_valid_data() {
        let mut sr = SecureRandom::new();
        let nonce1 = sr.nonce();
        let nonce2 = sr.nonce();
        assert_ne!(
            nonce1, nonce2,
            "nonce の連続呼び出しで異なる値が返る必要がある"
        );
    }

    #[test]
    fn now_returns_ok_timestamp() {
        let result = super::now();
        assert!(result.is_ok(), "now() は Ok を返す必要があります");
    }

    /// ユーザー主導の切断の close handshake が、サーバー側から RST で切断された
    /// dead socket への書き込みで I/O エラーになっても `Ok(())` を返すべきことを
    /// 検証する。
    ///
    /// e2e-tests/tests/messaging.rs の `test_messaging_sendrecv` で macOS self-hosted の
    /// CI に一度だけ観測された「クライアント 2 の disconnect が
    /// `Io(Os { code: 32, kind: BrokenPipe, message: "Broken pipe" })` で失敗する」
    /// 現象の再現を意図する。
    ///
    /// クライアント 1 の切断を契機にサーバーがクライアント 2 の WebSocket を
    /// RST で切断すると、クライアント 2 の run ループはそれを検知する前に
    /// disconnect コマンドで抜けて close handshake に入る。このとき
    /// `server_close_received` も ignore 構成でもないため、従来は close handshake の
    /// 書き込みエラーがそのまま `run()` の `Err` になり、ユーザーが切断を要求した
    /// にもかかわらず disconnect が失敗してしまう。
    ///
    /// 本テストは実 WebSocket コネクションをローカルの TCP 上で確立し、
    /// サーバー側ソケットを RST で切断してから `close_websocket_handshake` を
    /// 実行することで、この経路を決定的に再現する。ユーザー主導の切断は
    /// `server_close_received=false` かつ吸収条件の合成 bool を `true` (ユーザー主導の
    /// 切断を表す) にして表す。モックやスタブは使わない。
    #[tokio::test]
    async fn close_websocket_handshake_does_not_fail_user_disconnect_on_dead_socket() {
        use shiguredo_websocket::{
            ConnectionOutput, ServerConnectionOptions, WebSocketServerConnection,
        };

        // 実 TCP ペアを用意する。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("TCP リスナーの起動に失敗しました");
        let addr = listener
            .local_addr()
            .expect("リスナーアドレスの取得に失敗しました");
        let client_tcp = TcpStream::connect(addr)
            .await
            .expect("クライアント側 TCP の接続に失敗しました");
        let (mut server_tcp, _) = listener
            .accept()
            .await
            .expect("サーバー側 TCP の受付に失敗しました");

        // クライアント側の WebSocket を Connected まで駆動する。
        let (timer_tx, _timer_rx) = mpsc::channel::<TimerId>(16);
        let mut timers = TimerManager::new(timer_tx);
        let mut client_stream = ClientStream::new_plain(client_tcp);
        let options = ClientConnectionOptions::new("127.0.0.1", "/signaling");
        let mut client_ws = WebSocketClientConnection::new(options, SecureRandom::new());
        client_ws
            .connect()
            .expect("WebSocket クライアントの接続開始に失敗しました");
        // ハンドシェイクリクエストを送信する。
        flush_ws_output(&mut client_ws, &mut client_stream, &mut timers)
            .await
            .expect("ハンドシェイクリクエストの送信に失敗しました");

        // サーバー側でリクエストを読み、自動受理して 101 応答を返す。
        let mut server_ws = WebSocketServerConnection::new(ServerConnectionOptions::new());
        let mut buf = vec![0u8; 8192];
        loop {
            let n = server_tcp
                .read(&mut buf)
                .await
                .expect("サーバー側の読み込みに失敗しました");
            assert!(n > 0, "ハンドシェイクリクエストが届きませんでした");
            server_ws
                .feed_recv_buf(&buf[..n])
                .expect("ハンドシェイクリクエストの処理に失敗しました");
            if server_ws.handshake_request().is_some() {
                break;
            }
        }
        server_ws
            .accept_handshake_auto()
            .expect("ハンドシェイクの自動受理に失敗しました");
        while let Some(output) = server_ws.poll_output() {
            if let ConnectionOutput::SendData(data) = output {
                server_tcp
                    .write_all(&data)
                    .await
                    .expect("101 応答の送信に失敗しました");
            }
        }

        // クライアント側で 101 応答を読み、Connected になることを確認する。
        let n = client_stream
            .read(&mut buf)
            .await
            .expect("クライアント側の読み込みに失敗しました");
        assert!(n > 0, "101 応答が届きませんでした");
        client_ws
            .feed_recv_buf(&buf[..n], now().expect("現在時刻の取得に失敗しました"))
            .expect("101 応答の処理に失敗しました");
        assert_eq!(
            client_ws.state(),
            ConnectionState::Connected,
            "WebSocket クライアントが Connected になっていません"
        );

        // サーバー側ソケットを SO_LINGER=0 付きでクローズし、RST で切断する。
        let std_server = server_tcp
            .into_std()
            .expect("サーバー側 TCP の std 変換に失敗しました");
        let server_socket = socket2::Socket::from(std_server);
        server_socket
            .set_linger(Some(Duration::ZERO))
            .expect("SO_LINGER の設定に失敗しました");
        drop(server_socket);
        drop(server_ws);
        // RST がクライアント側に届くのを待つ。
        tokio::time::sleep(Duration::from_millis(100)).await;

        // ユーザー主導の切断 (server_close_received=false、吸収条件の合成 bool=true) の
        // 条件で close handshake を実行する。これは messaging テストのクライアント 2
        // と同じ条件である。ユーザーが切断を要求している以上、close handshake の
        // 失敗は warning に落として Ok(()) を返すべきである。
        let mut handler = RecordingHandler::default();
        let result = close_websocket_handshake(
            &mut client_ws,
            &mut client_stream,
            &mut timers,
            &mut handler,
            false,
            true,
            Duration::from_secs(3),
        )
        .await;
        assert!(
            result.is_ok(),
            "ユーザー主導の切断の close handshake は Ok を返すべきですが Err になりました: {:?}",
            result
        );
    }

    #[test]
    fn send_message_rejects_unknown_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_data_channel_config(&mut connection, "#chat");
        let result = connection.handle_send_message_command("#unknown", b"data");
        assert!(
            matches!(result, Err(Error::InvalidDataChannelLabel { label }) if label == "#unknown"),
            "未登録ラベルは InvalidDataChannelLabel になるべき"
        );
    }

    #[test]
    fn send_message_rejects_signaling_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_data_channel_config(&mut connection, "#chat");
        let result = connection.handle_send_message_command("signaling", b"data");
        assert!(
            matches!(result, Err(Error::InvalidDataChannelLabel { label }) if label == "signaling"),
            "signaling ラベルは InvalidDataChannelLabel になるべき"
        );
    }

    #[test]
    fn send_message_rejects_stats_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_data_channel_config(&mut connection, "#chat");
        let result = connection.handle_send_message_command("stats", b"data");
        assert!(
            matches!(result, Err(Error::InvalidDataChannelLabel { label }) if label == "stats"),
            "stats ラベルは InvalidDataChannelLabel になるべき"
        );
    }

    #[test]
    fn send_message_rejects_push_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_data_channel_config(&mut connection, "#chat");
        let result = connection.handle_send_message_command("push", b"data");
        assert!(
            matches!(result, Err(Error::InvalidDataChannelLabel { label }) if label == "push"),
            "push ラベルは InvalidDataChannelLabel になるべき"
        );
    }

    #[test]
    fn send_message_rejects_notify_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_data_channel_config(&mut connection, "#chat");
        let result = connection.handle_send_message_command("notify", b"data");
        assert!(
            matches!(result, Err(Error::InvalidDataChannelLabel { label }) if label == "notify"),
            "notify ラベルは InvalidDataChannelLabel になるべき"
        );
    }

    #[test]
    fn send_message_rejects_rpc_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_data_channel_config(&mut connection, "#chat");
        let result = connection.handle_send_message_command("rpc", b"data");
        assert!(
            matches!(result, Err(Error::InvalidDataChannelLabel { label }) if label == "rpc"),
            "rpc ラベルは InvalidDataChannelLabel になるべき"
        );
    }

    #[test]
    fn send_message_rejects_empty_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_data_channel_config(&mut connection, "#chat");
        let result = connection.handle_send_message_command("", b"data");
        assert!(
            matches!(result, Err(Error::InvalidDataChannelLabel { label }) if label.is_empty()),
            "空ラベルは InvalidDataChannelLabel になるべき"
        );
    }

    #[test]
    fn send_message_rejects_all_labels_when_no_data_channels_configured() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let result = connection.handle_send_message_command("#chat", b"data");
        assert!(
            matches!(result, Err(Error::InvalidDataChannelLabel { label }) if label == "#chat"),
            "data_channels 未設定時は通常の # ラベルでも InvalidDataChannelLabel になるべき"
        );
    }

    #[test]
    fn send_message_returns_data_channel_missing_when_channel_not_registered() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_data_channel_config(&mut connection, "#chat");
        let result = connection.handle_send_message_command("#chat", b"data");
        assert!(
            matches!(result, Err(Error::DataChannelMissing { label }) if label == "#chat"),
            "config に登録済みでも実チャネル未登録なら DataChannelMissing になるべき"
        );
    }

    #[test]
    fn server_close_label_returns_true_for_signaling() {
        assert!(is_server_close_label("signaling"));
    }

    #[test]
    fn server_close_label_returns_false_for_other_labels() {
        for label in ["stats", "push", "notify"] {
            assert!(
                !is_server_close_label(label),
                "label={label} の Close は接続を終了させない"
            );
        }
    }

    fn data_channel_config(labels: &[&str]) -> Vec<DataChannelConfig> {
        labels
            .iter()
            .map(|label| DataChannelConfig {
                label: (*label).to_string(),
                compress: false,
                direction: "sendrecv".to_string(),
            })
            .collect()
    }

    fn opened_labels(labels: &[&str]) -> HashSet<String> {
        labels.iter().map(|label| (*label).to_string()).collect()
    }

    #[test]
    fn data_channel_signaling_ready_is_false_without_switched() {
        let configs = data_channel_config(&["signaling"]);
        let opened = opened_labels(&["signaling"]);
        assert!(
            !is_data_channel_signaling_ready(false, &configs, &opened),
            "switched 未受信の場合は全設定チャンネルが Open 済みでも readiness は false"
        );
    }

    #[test]
    fn data_channel_signaling_ready_is_false_with_partial_open() {
        let configs = data_channel_config(&["signaling", "#messaging"]);
        let opened = opened_labels(&["signaling"]);
        assert!(
            !is_data_channel_signaling_ready(true, &configs, &opened),
            "switched 受信済みでも一部の設定チャンネルが未 Open なら readiness は false"
        );
    }

    #[test]
    fn data_channel_signaling_ready_is_true_with_all_open() {
        let configs = data_channel_config(&["signaling", "#messaging"]);
        let opened = opened_labels(&["signaling", "#messaging"]);
        assert!(
            is_data_channel_signaling_ready(true, &configs, &opened),
            "switched 受信済みかつ全設定チャンネルが Open 済みなら readiness は true"
        );
    }

    #[test]
    fn data_channel_signaling_ready_is_false_with_empty_configs() {
        let configs = data_channel_config(&[]);
        let opened = opened_labels(&[]);
        assert!(
            !is_data_channel_signaling_ready(true, &configs, &opened),
            "data_channels が空の構成では readiness は false"
        );
    }

    #[test]
    fn data_channel_signaling_ready_is_false_after_redirect_reset() {
        let configs = data_channel_config(&["signaling", "#messaging"]);
        let opened = opened_labels(&[]);
        assert!(
            !is_data_channel_signaling_ready(false, &configs, &opened),
            "redirect 相当として switched と opened label を初期化すると readiness は false に戻る"
        );
    }

    #[test]
    fn switching_ready_is_false_without_switched() {
        let configs = data_channel_config(&["signaling"]);
        let opened = opened_labels(&["signaling"]);
        assert!(
            !is_switching_ready(false, false, true, &configs, &opened),
            "switched 未受信 (switched_ignore_disconnect_websocket=false) なら切替条件は false"
        );
    }

    #[test]
    fn switching_ready_is_false_when_websocket_closed() {
        let configs = data_channel_config(&["signaling"]);
        let opened = opened_labels(&["signaling"]);
        assert!(
            !is_switching_ready(true, true, true, &configs, &opened),
            "websocket_closed=true なら切替条件は false"
        );
    }

    #[test]
    fn switching_ready_is_false_with_partial_open() {
        let configs = data_channel_config(&["signaling", "#messaging"]);
        let opened = opened_labels(&["signaling"]);
        assert!(
            !is_switching_ready(true, false, true, &configs, &opened),
            "一部の設定チャンネルが未 Open なら切替条件は false"
        );
    }

    #[test]
    fn switching_ready_is_true_with_all_conditions() {
        let configs = data_channel_config(&["signaling", "#messaging"]);
        let opened = opened_labels(&["signaling", "#messaging"]);
        assert!(
            is_switching_ready(true, false, true, &configs, &opened),
            "switched 受信済み・websocket 未クローズ・全チャンネル Open なら切替条件は true"
        );
    }

    #[test]
    fn resolve_ws_disconnect_delay_start_returns_none_after_close_sent() {
        let now = tokio::time::Instant::now();
        // close 送信後 (ws_connected=false) は、開始時刻が設定済み (Some(過去)) でも
        // None を返して待機を止める必要がある。
        let start = now - tokio::time::Duration::from_secs(10);
        assert_eq!(
            resolve_ws_disconnect_delay_start(Some(start), false, true, now),
            None,
            "close 送信後は None を返す必要があります"
        );
    }

    #[test]
    fn resolve_ws_disconnect_delay_start_returns_none_when_switching_condition_broken() {
        let now = tokio::time::Instant::now();
        // 切替条件不成立 (switching_ready=false) は、state が Connected でも
        // None を返して待機を止める必要がある。
        let start = now - tokio::time::Duration::from_secs(10);
        assert_eq!(
            resolve_ws_disconnect_delay_start(Some(start), true, false, now),
            None,
            "切替条件不成立時は None を返す必要があります"
        );
    }

    #[test]
    fn resolve_ws_disconnect_delay_start_keeps_existing_start_while_waiting() {
        let now = tokio::time::Instant::now();
        // 待機中 (ws_connected=true かつ switching_ready=true) は None を返さない。
        // 既存の開始時刻を維持し、now で置き換えて待機期限を延ばさない。
        let start = now - tokio::time::Duration::from_secs(3);
        assert_eq!(
            resolve_ws_disconnect_delay_start(Some(start), true, true, now),
            Some(start),
            "待機中は既存の開始時刻を維持する必要があります"
        );
    }

    #[test]
    fn resolve_ws_disconnect_delay_start_keeps_expired_start_while_waiting() {
        let now = tokio::time::Instant::now();
        // 待機中は開始時刻が既に期限超過 (Some(過去)) でも None にしない。
        // resolve は WS_DISCONNECT_DELAY の経過判定を持たず、経過は呼び出し側の
        // 切替ブロックが行う。期限経過時に None を返すように後退すると close が
        // 永遠に送信されなくなってしまうため、維持することを固定する。
        let start = now - tokio::time::Duration::from_secs(11);
        assert_eq!(
            resolve_ws_disconnect_delay_start(Some(start), true, true, now),
            Some(start),
            "待機中は期限超過の開始時刻でも維持する必要があります"
        );
    }

    #[test]
    fn resolve_ws_disconnect_delay_start_sets_now_when_start_is_missing() {
        let now = tokio::time::Instant::now();
        // 待機中に開始時刻が未設定 (None) でも、now で初期化した開始時刻を返す。
        assert_eq!(
            resolve_ws_disconnect_delay_start(None, true, true, now),
            Some(now),
            "未設定なら now を開始時刻として設定する必要があります"
        );
    }

    /// DataChannel が Closed 状態に遷移するまで待つ。
    ///
    /// libwebrtc の close() は非同期遷移のため、固定 sleep ではなく
    /// state() が Closed になるまでポーリングして決定的に待つ。
    async fn wait_data_channel_closed(connection: &mut SoraConnection, label: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if connection.data_channels[label].channel.state() == DataChannelState::Closed {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "DataChannel '{}' が Closed 状態に遷移しませんでした",
                label
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn should_notify_close_ignores_non_closed_state() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_compressed_data_channel(&mut connection, "signaling");
        let opened = opened_labels(&["signaling"]);
        // register 直後のチャネルは Closed 以外の状態 (テスト環境では Connecting) であり、
        // 閉じたとは判定されない。Closing を含む Closed 以外の状態で remove しないことを
        // 検証する (テスト環境では Closing 状態を作れないため)。
        assert!(
            !should_notify_close(&connection.data_channels, &opened, "signaling"),
            "Closed 以外の状態では remove してはなりません"
        );
    }

    #[tokio::test]
    async fn should_notify_close_accepts_closed_state() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_compressed_data_channel(&mut connection, "signaling");
        // close() を呼ぶと Closed 状態に遷移する (テスト環境でも観測できる)。
        connection.data_channels["signaling"].channel.close();
        wait_data_channel_closed(&mut connection, "signaling").await;
        let opened = opened_labels(&["signaling"]);
        assert!(
            should_notify_close(&connection.data_channels, &opened, "signaling"),
            "Closed 状態では remove する必要があります"
        );
    }

    #[tokio::test]
    async fn should_notify_close_ignores_unopened_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        // opened_data_channels に含まれない #chat を Closed 状態にする。
        // #chat が Closed でも opened に含まれなければ remove されないことを確認する。
        register_compressed_data_channel(&mut connection, "#chat");
        connection.data_channels["#chat"].channel.close();
        wait_data_channel_closed(&mut connection, "#chat").await;
        let opened = opened_labels(&["signaling"]);
        assert!(
            !should_notify_close(&connection.data_channels, &opened, "#chat"),
            "opened_data_channels に含まれない label は remove してはなりません"
        );
    }

    #[tokio::test]
    async fn wait_close_loop_keeps_waiting_on_non_closed_state_event() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_compressed_data_channel(&mut connection, "signaling");
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SoraEvent>();
        let mut handler = RecordingHandler::default();
        let mut opened = opened_labels(&["signaling"]);

        // Closed 以外の状態 (register 直後は Connecting) の StateChange イベントを送信する。
        // テスト環境では Closing 状態を作れないため、Closed 以外を代表する Connecting で
        // 「Closed でない状態のイベントでは remove されない」ことを検証する。
        event_tx
            .send(SoraEvent::DataChannelStateChange("signaling".to_string()))
            .expect("Closed 以外の状態のイベントの送信に失敗しました");
        // イベントを処理させた後、event_rx をクローズして待機を終了させる。
        drop(event_tx);
        let data_channels = connection.data_channels;

        let result = wait_data_channels_close(
            &mut event_rx,
            &mut handler,
            &mut opened,
            data_channels,
            tokio::time::Instant::now() + tokio::time::Duration::from_secs(60),
        )
        .await;

        // Closed 以外の状態のイベントでは remove されないため、待機は AllChannelsClosed
        // ではなく event_rx クローズで終了する必要がある。
        assert_eq!(
            result,
            DataChannelCloseWaitResult::EventChannelClosed,
            "Closed 以外の状態のイベントで AllChannelsClosed として終了してはいけません"
        );
        // close 通知は event_rx クローズ時の残りチャネル通知のみ (1 回)。
        assert_eq!(
            handler.data_channel_close_count, 1,
            "close 通知は残りチャネルへの通知 1 回だけである必要があります"
        );
        assert_eq!(
            handler.data_channel_close_labels,
            vec!["signaling".to_string()],
            "close 通知の label が一致する必要があります"
        );
        assert!(
            opened.is_empty(),
            "event_rx クローズ時の残りチャネル通知で opened がクリアされる必要があります"
        );
    }

    #[tokio::test]
    async fn wait_close_loop_notifies_close_when_all_channels_closed() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_compressed_data_channel(&mut connection, "signaling");
        register_compressed_data_channel(&mut connection, "#chat");
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SoraEvent>();
        let mut handler = RecordingHandler::default();
        let mut opened = opened_labels(&["signaling", "#chat"]);

        // 全チャネルを close() して Closed 状態に遷移させる。
        connection.data_channels["signaling"].channel.close();
        connection.data_channels["#chat"].channel.close();
        wait_data_channel_closed(&mut connection, "signaling").await;
        wait_data_channel_closed(&mut connection, "#chat").await;

        // 全チャネルの Closed イベントを送信する。
        event_tx
            .send(SoraEvent::DataChannelStateChange("signaling".to_string()))
            .expect("signaling の Closed イベントの送信に失敗しました");
        event_tx
            .send(SoraEvent::DataChannelStateChange("#chat".to_string()))
            .expect("#chat の Closed イベントの送信に失敗しました");
        let data_channels = connection.data_channels;

        let result = wait_data_channels_close(
            &mut event_rx,
            &mut handler,
            &mut opened,
            data_channels,
            tokio::time::Instant::now() + tokio::time::Duration::from_secs(60),
        )
        .await;

        assert_eq!(
            result,
            DataChannelCloseWaitResult::AllChannelsClosed,
            "全チャネル Closed なら待機が正常終了する必要があります"
        );
        assert!(
            opened.is_empty(),
            "全チャネル Closed 後に opened がクリアされる必要があります"
        );
        assert_eq!(
            handler.data_channel_close_count, 2,
            "各チャネルへ close 通知が 1 回ずつ呼ばれる必要があります"
        );
        assert_eq!(
            handler.data_channel_close_labels,
            vec!["signaling".to_string(), "#chat".to_string()],
            "close 通知の label が受信順と一致する必要があります"
        );
    }

    #[tokio::test]
    async fn close_command_channel_acks_queued_disconnect() {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<SoraConnectionCommand>();
        // 終了フェーズに入る直前に 2 回目の disconnect() が送信された状況を作る。
        let (ack_tx, ack_rx) = oneshot::channel();
        command_tx
            .send(SoraConnectionCommand::Disconnect(ack_tx))
            .expect("Disconnect コマンドの送信に失敗しました");
        let mut user_initiated_disconnect = false;

        close_command_channel_and_ack_pending_disconnects(
            &mut command_rx,
            &mut user_initiated_disconnect,
        )
        .await;

        // クローズ前に送信された Disconnect には ack が返る必要がある。
        ack_rx
            .await
            .expect("disconnect の ack が送信される必要があります");
        // キュー済み Disconnect を ack した場合はユーザー主導の切断として扱う。
        assert!(
            user_initiated_disconnect,
            "キュー済み Disconnect の ack 時は user_initiated_disconnect が立つ必要があります"
        );
        // close() 後の送信は失敗する (CommandSendFailed になる)。
        let (ack_tx2, _ack_rx2) = oneshot::channel();
        assert!(
            command_tx
                .send(SoraConnectionCommand::Disconnect(ack_tx2))
                .is_err(),
            "close() 後の送信は失敗する必要があります"
        );
    }

    #[tokio::test]
    async fn close_command_channel_discards_non_disconnect_command() {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<SoraConnectionCommand>();
        // 終了フェーズに入る直前に GetStats が送信された状況を作る。
        let (stats_tx, stats_rx) = oneshot::channel();
        command_tx
            .send(SoraConnectionCommand::GetStats(stats_tx))
            .expect("GetStats コマンドの送信に失敗しました");
        let mut user_initiated_disconnect = false;

        close_command_channel_and_ack_pending_disconnects(
            &mut command_rx,
            &mut user_initiated_disconnect,
        )
        .await;

        // GetStats には応答が返らず、呼び出し側は CommandResponseMissing になる
        // (response_tx が send されずに drop されるため RecvError になる)。
        assert!(
            stats_rx.await.is_err(),
            "Disconnect 以外のコマンドには応答してはいけません"
        );
        // Disconnect 以外のコマンドのみの場合はユーザー主導の切断として扱わない。
        assert!(
            !user_initiated_disconnect,
            "Disconnect 以外のコマンドでは user_initiated_disconnect が立たない必要があります"
        );
    }

    #[tokio::test]
    async fn wait_close_loop_distinguishes_event_channel_closed() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_compressed_data_channel(&mut connection, "signaling");
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SoraEvent>();
        let mut handler = RecordingHandler::default();
        let mut opened = opened_labels(&["signaling"]);

        // event_rx をクローズして待機を終了させる。
        drop(event_tx);
        let data_channels = connection.data_channels;

        let result = wait_data_channels_close(
            &mut event_rx,
            &mut handler,
            &mut opened,
            data_channels,
            tokio::time::Instant::now() + tokio::time::Duration::from_secs(60),
        )
        .await;

        // event_rx クローズはタイムアウトとは区別された終了要因で返る。
        assert_eq!(
            result,
            DataChannelCloseWaitResult::EventChannelClosed,
            "event_rx クローズは EventChannelClosed として終了する必要があります"
        );
        // クローズ時もタイムアウト時と同じく残りチャネルへ close 通知される。
        assert_eq!(
            handler.data_channel_close_count, 1,
            "event_rx クローズ時に残りチャネルへ close 通知される必要があります"
        );
        assert!(
            opened.is_empty(),
            "event_rx クローズ時の残りチャネル通知で opened がクリアされる必要があります"
        );
    }

    #[tokio::test]
    async fn wait_close_loop_distinguishes_timeout_and_notifies_remaining() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_compressed_data_channel(&mut connection, "signaling");
        register_compressed_data_channel(&mut connection, "#chat");
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SoraEvent>();
        let mut handler = RecordingHandler::default();
        let mut opened = opened_labels(&["signaling", "#chat"]);

        // 期限切れを即座に発生させるため、過去の時刻を deadline にする。
        let deadline = tokio::time::Instant::now() - tokio::time::Duration::from_secs(1);
        // event_rx を開いたままにして、クローズ終了と区別する。
        let _event_tx = event_tx;
        let data_channels = connection.data_channels;

        let result = wait_data_channels_close(
            &mut event_rx,
            &mut handler,
            &mut opened,
            data_channels,
            deadline,
        )
        .await;

        // タイムアウトは event_rx クローズとは区別された終了要因で返る。
        assert_eq!(
            result,
            DataChannelCloseWaitResult::TimedOut,
            "期限切れは TimedOut として終了する必要があります"
        );
        // タイムアウト時に残りチャネルへ close 通知される現状維持の挙動。
        assert_eq!(
            handler.data_channel_close_count, 2,
            "タイムアウト時に残りチャネルへ close 通知される必要があります"
        );
        assert!(
            opened.is_empty(),
            "タイムアウト時に opened がクリアされる必要があります"
        );
    }

    use shiguredo_webrtc::DataChannelInit;

    /// 実際の `SoraConnectionContext` と `SoraConnection` を構築するテスト用ヘルパー。
    fn build_test_connection(
        handler: impl SoraConnectionEventHandler + 'static,
    ) -> (SoraConnection, SoraConnectionHandle) {
        let context =
            SoraConnectionContext::new().expect("SoraConnectionContext の作成に失敗しました");
        SoraConnection::builder(
            context,
            vec!["wss://example.com/signaling".to_string()],
            "test-channel".to_string(),
            Role::RecvOnly,
            handler,
        )
        .build()
        .expect("SoraConnection の生成に失敗しました")
    }

    /// callback の呼び出しを記録するテスト用ハンドラ。
    #[derive(Default)]
    struct RecordingHandler {
        data_channel_message_count: usize,
        data_channel_close_count: usize,
        data_channel_close_labels: Vec<String>,
        signaling_received_count: usize,
        notify_count: usize,
        push_count: usize,
        message_count: usize,
        message_data: Vec<Vec<u8>>,
    }

    impl SoraConnectionEventHandler for RecordingHandler {
        fn on_signaling_message(
            &mut self,
            signaling_type: SignalingType,
            direction: SignalingDirection,
            _text: &str,
        ) {
            if signaling_type == SignalingType::DataChannel
                && direction == SignalingDirection::Received
            {
                self.signaling_received_count += 1;
            }
        }

        fn on_notify(&mut self, _text: &str) {
            self.notify_count += 1;
        }

        fn on_push(&mut self, _text: &str) {
            self.push_count += 1;
        }

        fn on_message(&mut self, _label: &str, data: &[u8]) {
            self.message_count += 1;
            self.message_data.push(data.to_vec());
        }

        fn on_data_channel_message(&mut self, _label: &str, _data: &[u8]) {
            self.data_channel_message_count += 1;
        }

        fn on_data_channel_close(&mut self, label: &str) {
            self.data_channel_close_count += 1;
            self.data_channel_close_labels.push(label.to_string());
        }
    }

    /// DataChannel の設定 (`data_channel_configs`) だけを登録するテスト用ヘルパー。
    /// 実チャネルは作成しないため、`DataChannelMissing` エラーパスの再現に使う。
    fn register_data_channel_config(connection: &mut SoraConnection, label: &str) {
        connection.data_channel_configs.push(DataChannelConfig {
            label: label.to_string(),
            compress: true,
            direction: "sendrecv".to_string(),
        });
    }

    /// compress 有効の DataChannel を実 PeerConnection 経由で登録するテスト用ヘルパー。
    fn register_compressed_data_channel(connection: &mut SoraConnection, label: &str) {
        register_data_channel_config(connection, label);
        let mut init = DataChannelInit::new();
        let channel = connection
            .pc
            .create_data_channel(label, &mut init)
            .expect("DataChannel の生成に失敗しました");
        let event_tx = connection.event_tx.clone();
        connection.register_data_channel(channel, &event_tx);
    }

    #[tokio::test]
    async fn zlib_failure_then_same_label_then_other_label() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_compressed_data_channel(&mut connection, "signaling");
        register_compressed_data_channel(&mut connection, "push");
        let mut handler = RecordingHandler::default();

        // 1. zlib 展開に失敗する message
        let result = connection
            .handle_data_channel_message(&mut handler, "signaling", &[0x00, 0x01, 0x02, 0x03])
            .await;
        assert!(
            result.is_ok(),
            "zlib 展開失敗は Ok(Continue) を返す必要があります"
        );
        assert_eq!(
            handler.data_channel_message_count, 0,
            "zlib 展開失敗では callback を呼ばない必要があります"
        );

        // 2. 同じ label の正常 message
        let compressed = compress_zlib(b"{\"type\":\"notify\"}").expect("zlib 圧縮に失敗しました");
        let result = connection
            .handle_data_channel_message(&mut handler, "signaling", &compressed)
            .await;
        assert!(result.is_ok(), "正常 message は Ok を返す必要があります");
        assert_eq!(
            handler.notify_count, 1,
            "同じ label の正常 message が semantic callback まで到達する必要があります"
        );
        // 3. 別 label の正常 message
        let compressed = compress_zlib(b"{\"type\":\"push\"}").expect("zlib 圧縮に失敗しました");
        let result = connection
            .handle_data_channel_message(&mut handler, "push", &compressed)
            .await;
        assert!(
            result.is_ok(),
            "別 label の正常 message は Ok を返す必要があります"
        );
        assert_eq!(
            handler.push_count, 1,
            "別 label の正常 message が semantic callback まで到達する必要があります"
        );
    }

    #[tokio::test]
    async fn zlib_failure_is_discarded_per_message() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        register_compressed_data_channel(&mut connection, "signaling");

        let valid = compress_zlib(b"{\"type\":\"notify\"}").expect("zlib 圧縮に失敗しました");

        // 不正 header
        let invalid_header = vec![0x00, 0x01, 0x02, 0x03];
        // truncated stream
        let truncated = valid[..valid.len() - 1].to_vec();
        // Adler-32 不一致
        let mut adler_mismatch = valid.clone();
        let last = adler_mismatch.len() - 1;
        adler_mismatch[last] ^= 0xFF;
        // 展開後サイズ上限を超える入力
        let oversized = compress_zlib(&vec![b'a'; MAX_DECOMPRESSED_DATA_CHANNEL_MESSAGE_SIZE + 1])
            .expect("zlib 圧縮に失敗しました");

        for bad in [invalid_header, truncated, adler_mismatch, oversized] {
            let mut handler = RecordingHandler::default();
            let result = connection
                .handle_data_channel_message(&mut handler, "signaling", &bad)
                .await;
            assert!(
                result.is_ok(),
                "不正な圧縮データは Ok(Continue) を返す必要があります"
            );
            assert_eq!(
                handler.data_channel_message_count, 0,
                "不正な圧縮データでは callback を呼ばない必要があります"
            );
        }

        // 同じ DataChannel と接続は維持される
        let mut handler = RecordingHandler::default();
        let result = connection
            .handle_data_channel_message(&mut handler, "signaling", &valid)
            .await;
        assert!(
            result.is_ok(),
            "後続の正常 message は Ok を返す必要があります"
        );
        assert_eq!(
            handler.notify_count, 1,
            "破棄の後も同じ DataChannel の後続 message を処理できる必要があります"
        );
    }

    #[tokio::test]
    async fn user_defined_label_binary_message_is_forwarded() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();
        let data = [0xDE, 0xAD, 0xBE, 0xEF];

        let result = connection
            .handle_data_channel_message(&mut handler, "#messaging", &data)
            .await;
        assert!(
            result.is_ok(),
            "任意 binary data は Ok を返す必要があります"
        );
        assert_eq!(
            handler.data_channel_message_count, 1,
            "on_data_channel_message が 1 回呼ばれる必要があります"
        );
        assert_eq!(
            handler.message_count, 1,
            "on_message が 1 回呼ばれる必要があります"
        );
        assert_eq!(
            handler.message_data,
            vec![data.to_vec()],
            "on_message に本文がそのまま渡される必要があります"
        );
    }

    #[tokio::test]
    async fn unsupported_label_binary_message_does_not_terminate_connection() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();
        let data = [0xFF, 0xFE, 0xFD];

        let result = connection
            .handle_data_channel_message(&mut handler, "unknown-label", &data)
            .await;
        assert!(
            result.is_ok(),
            "未対応 label の binary data は Ok を返す必要があります"
        );
        assert_eq!(
            handler.data_channel_message_count, 1,
            "on_data_channel_message が 1 回呼ばれる必要があります"
        );
        assert_eq!(
            handler.message_count, 0,
            "未対応 label では on_message を呼ばない必要があります"
        );
    }

    #[tokio::test]
    async fn signaling_invalid_utf8_remains_fatal() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let result = connection
            .handle_data_channel_message(&mut handler, "signaling", &[0xFF, 0xFE, 0xFD])
            .await;
        assert!(
            result.is_err(),
            "signaling label の不正 UTF-8 は run の終了原因のままにする必要があります"
        );
        assert_eq!(
            handler.data_channel_message_count, 1,
            "on_data_channel_message は zlib 展開成功時に 1 回呼ばれる必要があります"
        );
        assert_eq!(
            handler.signaling_received_count, 0,
            "UTF-8 変換失敗時は on_signaling_message を呼ばない必要があります"
        );
    }

    #[tokio::test]
    async fn signaling_json_syntax_error_remains_fatal() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let result = connection
            .handle_data_channel_message(&mut handler, "signaling", b"this is not json")
            .await;
        assert!(
            result.is_err(),
            "signaling label の JSON syntax error は run の終了原因のままにする必要があります"
        );
        assert_eq!(
            handler.data_channel_message_count, 1,
            "on_data_channel_message は zlib 展開成功時に 1 回呼ばれる必要があります"
        );
        assert_eq!(
            handler.signaling_received_count, 1,
            "UTF-8 成功時は JSON parse の成否にかかわらず on_signaling_message を 1 回呼ぶ必要があります"
        );
    }

    #[tokio::test]
    async fn reoffer_with_invalid_sdp_returns_error() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();
        let message = br#"{"type":"re-offer","sdp":"this is not a valid SDP"}"#;

        let result = connection
            .handle_data_channel_message(&mut handler, "signaling", message)
            .await;
        assert!(
            result.is_err(),
            "parse 成功後の不正 SDP は SDP 適用 error になる必要があります"
        );
    }

    #[tokio::test]
    async fn ping_without_signaling_data_channel_returns_data_channel_missing() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let result = connection
            .handle_data_channel_message(&mut handler, "signaling", br#"{"type":"ping"}"#)
            .await;
        assert!(
            matches!(
                result,
                Err(Error::DataChannelMissing { label }) if label == "signaling"
            ),
            "signaling DataChannel 未登録の ping は DataChannelMissing になる必要があります"
        );
    }

    /// `SoraConnectionCommand::GetStats` を受信して応答を送らず、
    /// 受信した oneshot の Sender をテスト側へ渡すサーバーを生成する。
    ///
    /// 戻り値は `SoraConnectionHandle` と、サーバーが受信した Sender を受け取るための channel。
    /// コールバックの発火タイミングはテスト側が制御する。
    fn spawn_get_stats_server() -> (
        SoraConnectionHandle,
        mpsc::UnboundedReceiver<oneshot::Sender<Result<JsonString>>>,
    ) {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<SoraConnectionCommand>();
        let handle = SoraConnectionHandle { command_tx };
        let (sender_tx, sender_rx) =
            mpsc::unbounded_channel::<oneshot::Sender<Result<JsonString>>>();
        tokio::spawn(async move {
            while let Some(command) = command_rx.recv().await {
                if let SoraConnectionCommand::GetStats(tx) = command {
                    let _ = sender_tx.send(tx);
                }
            }
        });
        (handle, sender_rx)
    }

    /// get_stats の応答待機が、コールバック不発時にタイムアウトして
    /// `Error::CommandTimeout` を返すことを確認する。
    ///
    /// あわせて、タイムアウト後に遅延して届くコールバック送信が無視されることを
    /// 確認する。タイムアウトで受信側がドロップ済みのため、送信は失敗する。
    #[tokio::test]
    async fn get_stats_times_out_and_ignores_late_callback() {
        let (handle, mut sender_rx) = spawn_get_stats_server();

        // コールバックが発火しないままタイムアウトする。
        let result = handle
            .send_command(
                "get_stats",
                Some(Duration::from_millis(50)),
                SoraConnectionCommand::GetStats,
            )
            .await;
        assert!(
            matches!(
                result,
                Err(Error::CommandTimeout { command }) if command == "get_stats"
            ),
            "コールバック不発時は Error::CommandTimeout になる必要があります"
        );

        // タイムアウト後に遅延してコールバックが発火しても、受信側は
        // ドロップ済みのため送信は失敗し、無視される。
        let tx = sender_rx
            .recv()
            .await
            .expect("GetStats コマンドがサーバーに到達していません");
        let json = "{}"
            .parse::<JsonString>()
            .expect("JSON のパースに失敗しました");
        let send_result = tx.send(Ok(json));
        assert!(
            send_result.is_err(),
            "タイムアウト後のコールバック送信は失敗 (無視) される必要があります"
        );
    }

    /// get_stats の応答待機が、コールバック発火時に結果を返すことを確認する。
    #[tokio::test]
    async fn get_stats_returns_result_when_callback_fires() {
        let (handle, mut sender_rx) = spawn_get_stats_server();
        let json = "{}"
            .parse::<JsonString>()
            .expect("JSON のパースに失敗しました");
        let expected = json.clone();

        // get_stats の呼び出しと並行して、サーバー経由でコールバック相当の応答を送る。
        let send_task = tokio::spawn(async move {
            let tx = sender_rx
                .recv()
                .await
                .expect("GetStats コマンドがサーバーに到達していません");
            let _ = tx.send(Ok(expected));
        });
        let result = handle.get_stats().await;
        send_task
            .await
            .expect("コールバック相当の送信タスクが失敗しました");

        assert_eq!(
            result
                .as_ref()
                .expect("コールバック発火時は Ok を返す必要があります")
                .to_string(),
            json.to_string(),
            "get_stats の結果はコールバックが送信した値と一致する必要があります"
        );
    }

    /// 実 oneshot channel と timeout task を持つ pending request を登録する。
    ///
    /// timeout task は run loop の `SendRpcRequest` 処理と同じく、
    /// 経過後に `SoraEvent::RpcTimeout` を送信する。
    fn insert_pending_rpc_with_timeout(
        connection: &mut SoraConnection,
        id: u64,
        timeout: Duration,
    ) -> oneshot::Receiver<Result<Option<RpcResponse>>> {
        let (response_tx, response_rx) = oneshot::channel();
        let event_tx = connection.event_tx.clone();
        let timeout_handle = tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            let _ = event_tx.send(SoraEvent::RpcTimeout { id });
        });
        connection.pending_rpc_responses.insert(
            id,
            PendingRpcRequest {
                response_tx: Some(response_tx),
                timeout_handle,
            },
        );
        response_rx
    }

    /// デフォルトの短いタイムアウトで pending request を登録する。
    fn insert_pending_rpc(
        connection: &mut SoraConnection,
        id: u64,
    ) -> oneshot::Receiver<Result<Option<RpcResponse>>> {
        insert_pending_rpc_with_timeout(connection, id, Duration::from_millis(50))
    }

    /// 指定 id の pending の timeout task が abort され、
    /// その id の `RpcTimeout` が届かないことを確認する。
    ///
    /// 他の pending 由来の `RpcTimeout` は無視する。
    async fn assert_no_rpc_timeout_for_id(connection: &mut SoraConnection, id: u64) {
        // 50ms のタイムアウトを 4 倍以上待っても、abort されていれば event は届かない。
        tokio::time::sleep(Duration::from_millis(200)).await;
        while let Ok(event) = connection.event_rx.try_recv() {
            if let SoraEvent::RpcTimeout { id: received_id } = event {
                assert_ne!(
                    received_id, id,
                    "id={id} の RpcTimeout が届きました。timeout task が abort されていません"
                );
            }
        }
    }

    #[tokio::test]
    async fn rpc_known_id_success_completes_only_corresponding_pending() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let rx1 = insert_pending_rpc(&mut connection, 1);
        let mut rx2 = insert_pending_rpc(&mut connection, 2);

        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#,
            )
            .await;
        assert!(result.is_ok(), "正常 response は Ok を返す必要があります");

        let result = rx1.await.expect("id=1 の pending が完了しませんでした");
        let response = result.expect("id=1 の pending が error で完了しました");
        let Some(RpcResponse::Success { result }) = response else {
            panic!("Success を期待しましたが別の response になりました");
        };
        assert_eq!(
            result.to_string(),
            r#"{"ok":true}"#,
            "result が一致しません"
        );

        // 対応する pending だけが remove され、他は維持される。
        assert!(
            !connection.pending_rpc_responses.contains_key(&1),
            "id=1 の pending が remove されていません"
        );
        assert!(
            connection.pending_rpc_responses.contains_key(&2),
            "id=2 の pending が変更されています"
        );
        assert!(
            matches!(rx2.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "id=2 の response channel を完了しない必要があります"
        );

        // timeout task が abort され、id=1 の RpcTimeout が届かない。
        assert_no_rpc_timeout_for_id(&mut connection, 1).await;
    }

    #[tokio::test]
    async fn rpc_known_id_remote_error_completes_only_corresponding_pending() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let rx1 = insert_pending_rpc(&mut connection, 1);
        let mut rx2 = insert_pending_rpc(&mut connection, 2);

        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"custom message","data":{"k":1}}}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "正常 remote error は Ok を返す必要があります"
        );

        let result = rx1.await.expect("id=1 の pending が完了しませんでした");
        let Ok(Some(RpcResponse::Error {
            code,
            message,
            data,
        })) = result
        else {
            panic!("Error を期待しましたが別の response になりました");
        };
        assert_eq!(code, -32000, "code が一致しません");
        assert_eq!(message, "custom message", "message が一致しません");
        assert_eq!(
            data.map(|d| d.to_string()),
            Some(r#"{"k":1}"#.to_string()),
            "data が一致しません"
        );

        assert!(
            !connection.pending_rpc_responses.contains_key(&1),
            "id=1 の pending が remove されていません"
        );
        assert!(
            connection.pending_rpc_responses.contains_key(&2),
            "id=2 の pending が変更されています"
        );
        assert!(
            matches!(rx2.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "id=2 の response channel を完了しない必要があります"
        );

        assert_no_rpc_timeout_for_id(&mut connection, 1).await;
    }

    #[tokio::test]
    async fn rpc_known_id_protocol_violation_completes_only_corresponding_pending() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let rx1 = insert_pending_rpc(&mut connection, 1);
        let mut rx2 = insert_pending_rpc(&mut connection, 2);

        // jsonrpc が不正だが id=1 は有効な u64 id のため、対応する pending へ通知される。
        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0x","id":1,"result":null}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "protocol violation でも handle_data_channel_message は Ok を返す必要があります"
        );

        let result = rx1.await.expect("id=1 の pending が完了しませんでした");
        assert!(
            matches!(result, Err(Error::RpcProtocolViolation { id: Some(1) })),
            "RpcProtocolViolation で完了する必要があります"
        );

        assert!(
            !connection.pending_rpc_responses.contains_key(&1),
            "id=1 の pending が remove されていません"
        );
        assert!(
            connection.pending_rpc_responses.contains_key(&2),
            "id=2 の pending が変更されています"
        );
        assert!(
            matches!(rx2.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "id=2 の response channel を完了しない必要があります"
        );

        assert_no_rpc_timeout_for_id(&mut connection, 1).await;
    }

    #[tokio::test]
    async fn rpc_untrusted_id_response_keeps_all_pending() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let mut rx1 = insert_pending_rpc(&mut connection, 1);

        // id が String の正常 response は SDK の Request ID と相関できないため破棄される。
        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0","id":"x","result":null}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "信頼できない id の response は Ok を返す必要があります"
        );
        assert!(
            connection.pending_rpc_responses.contains_key(&1),
            "pending を変更しない必要があります"
        );

        // timeout task は維持されるため、timeout 経過後に RpcTimeout が届く。
        let event = tokio::time::timeout(Duration::from_millis(300), connection.event_rx.recv())
            .await
            .expect("timeout task が abort されています")
            .expect("RpcTimeout が届きませんでした");
        assert!(
            matches!(event, SoraEvent::RpcTimeout { id: 1 }),
            "id=1 の RpcTimeout を期待しました"
        );
        // 破棄された response は channel を完了しない。
        assert!(
            matches!(rx1.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "response channel を完了しない必要があります"
        );
    }

    #[tokio::test]
    async fn rpc_unknown_id_response_keeps_other_pending() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let mut rx1 = insert_pending_rpc(&mut connection, 1);

        // id=99 は pending に存在しない正常 response のため破棄される。
        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0","id":99,"result":null}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "未知 id の response は Ok を返す必要があります"
        );
        assert!(
            connection.pending_rpc_responses.contains_key(&1),
            "他の pending を変更しない必要があります"
        );
        assert!(
            matches!(rx1.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "破棄された response は channel を完了しない必要があります"
        );
    }

    #[tokio::test]
    async fn rpc_timed_out_id_response_keeps_other_pending() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let rx1 = insert_pending_rpc(&mut connection, 1);
        let mut rx2 = insert_pending_rpc(&mut connection, 2);

        // id=1 は timeout 済みとして pending から取り除かれている状態を再現する。
        connection.pending_rpc_responses.remove(&1);

        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0","id":1,"result":null}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "timeout 済み id の response は Ok を返す必要があります"
        );
        assert!(
            connection.pending_rpc_responses.contains_key(&2),
            "他の pending を変更しない必要があります"
        );
        assert!(
            matches!(rx2.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "他の pending の response channel を完了しない必要があります"
        );
        // 破棄された response は channel を完了しない (remove 済みのため Closed)。
        assert!(
            rx1.await.is_err(),
            "response channel を完了しない必要があります"
        );
    }

    #[tokio::test]
    async fn rpc_duplicated_response_keeps_other_pending() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let rx1 = insert_pending_rpc(&mut connection, 1);
        let mut rx2 = insert_pending_rpc(&mut connection, 2);

        // 1 回目は id=1 の pending を完了する。
        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0","id":1,"result":null}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "1 回目の response は Ok を返す必要があります"
        );
        let result = rx1.await.expect("id=1 の pending が完了しませんでした");
        assert!(
            result.is_ok(),
            "1 回目の response は success で完了する必要があります"
        );

        // 2 回目は id=1 の pending が既に無いため破棄され、他を変更しない。
        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0","id":1,"result":null}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "同じ id の重複 response は Ok を返す必要があります"
        );
        assert!(
            connection.pending_rpc_responses.contains_key(&2),
            "他の pending を変更しない必要があります"
        );
        assert!(
            matches!(rx2.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "他の pending の response channel を完了しない必要があります"
        );
    }

    #[tokio::test]
    async fn rpc_protocol_violation_does_not_terminate_connection() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        // protocol violation
        let rx1 = insert_pending_rpc(&mut connection, 1);
        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0x","id":1,"result":null}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "protocol violation は Ok を返す必要があります"
        );
        let result = rx1.await.expect("id=1 の pending が完了しませんでした");
        assert!(
            matches!(result, Err(Error::RpcProtocolViolation { .. })),
            "RpcProtocolViolation で完了する必要があります"
        );

        // 同じ DataChannel の正常 response を処理できる。
        let rx2 = insert_pending_rpc(&mut connection, 2);
        let result = connection
            .handle_data_channel_message(
                &mut handler,
                "rpc",
                br#"{"jsonrpc":"2.0","id":2,"result":null}"#,
            )
            .await;
        assert!(
            result.is_ok(),
            "protocol violation 後の正常 response は Ok を返す必要があります"
        );
        let result = rx2.await.expect("id=2 の pending が完了しませんでした");
        assert!(
            result.is_ok(),
            "id=2 の正常 response は success で完了する必要があります"
        );

        // 別 DataChannel の正常 message を処理できる。
        let result = connection
            .handle_data_channel_message(&mut handler, "push", br#"{"type":"push"}"#)
            .await;
        assert!(
            result.is_ok(),
            "別 DataChannel の正常 message は Ok を返す必要があります"
        );
        assert_eq!(
            handler.push_count, 1,
            "on_push が 1 回呼ばれる必要があります"
        );
    }

    #[tokio::test]
    async fn rpc_utf8_and_syntax_error_keep_pending_and_return_ok() {
        let (mut connection, _handle) = build_test_connection(RecordingHandler::default());
        let mut handler = RecordingHandler::default();

        let mut rx1 = insert_pending_rpc(&mut connection, 1);
        let mut rx2 = insert_pending_rpc(&mut connection, 2);

        // UTF-8 変換失敗は破棄され、Ok を返す。
        let result = connection
            .handle_data_channel_message(&mut handler, "rpc", &[0xFF, 0xFE, 0xFD])
            .await;
        assert!(result.is_ok(), "UTF-8 error は Ok を返す必要があります");

        // JSON syntax error は破棄され、Ok を返す。
        let result = connection
            .handle_data_channel_message(&mut handler, "rpc", b"this is not json")
            .await;
        assert!(
            result.is_ok(),
            "JSON syntax error は Ok を返す必要があります"
        );

        // 全 pending と timeout task が維持される。
        assert!(
            connection.pending_rpc_responses.contains_key(&1),
            "id=1 の pending を変更しない必要があります"
        );
        assert!(
            connection.pending_rpc_responses.contains_key(&2),
            "id=2 の pending を変更しない必要があります"
        );
        assert!(
            matches!(rx1.try_recv(), Err(oneshot::error::TryRecvError::Empty))
                && matches!(rx2.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "破棄された response は response channel を完了しない必要があります"
        );
    }
}
