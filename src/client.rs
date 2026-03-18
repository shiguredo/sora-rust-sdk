//! SoraClient 本体と接続制御の実装。
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_lc_rs::rand::{SecureRandom as AwsSecureRandom, SystemRandom};
use nojson::Json;
use rand::seq::SliceRandom;
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
    PeerConnectionRtcConfiguration, PeerConnectionState, Resolution, RtcError,
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

use crate::client_context::SoraClientContext;
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
use shiguredo_webrtc::{rtc_log_error, rtc_log_info, rtc_log_warning};

/// WebSocket (シグナリング接続) の TLS 設定。
///
/// TURN-TLS の TLS 設定は `SoraClientBuilder::turn_tls_insecure()` / `turn_tls_ca_cert()` で行う。
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    /// サーバー証明書の検証をスキップする。
    pub insecure: bool,
    /// クライアント証明書 (PEM 形式)。
    pub client_cert: Option<String>,
    /// クライアント秘密鍵 (PEM 形式)。
    pub client_key: Option<String>,
    /// CA 証明書 (PEM 形式)。
    pub ca_cert: Option<String>,
}

pub struct SoraClientBuilder {
    context: Arc<SoraClientContext>,
    signaling_urls: Vec<String>,
    channel_id: String,
    role: Role,

    #[allow(clippy::type_complexity)]
    on_signaling_message: Arc<dyn Fn(SignalingType, SignalingDirection, &str) + Send + Sync>,
    on_notify: Arc<dyn Fn(&str) + Send + Sync>,
    on_push: Arc<dyn Fn(&str) + Send + Sync>,
    on_track: Arc<dyn Fn(RtpTransceiver) + Send + Sync>,
    on_remove_track: Arc<dyn Fn(RtpReceiver) + Send + Sync>,
    on_switched: Arc<dyn Fn() + Send + Sync>,
    #[allow(clippy::type_complexity)]
    on_websocket_close: Arc<dyn Fn(Option<u16>, &str) + Send + Sync>,

    // メッセージングレイヤー
    #[allow(clippy::type_complexity)]
    on_message: Arc<dyn Fn(&str, &[u8]) + Send + Sync>,

    // DataChannel API レイヤー
    on_data_channel: Arc<dyn Fn(&str) + Send + Sync>,
    on_data_channel_open: Arc<dyn Fn(&str) + Send + Sync>,
    #[allow(clippy::type_complexity)]
    on_data_channel_message: Arc<dyn Fn(&str, &[u8]) + Send + Sync>,
    on_data_channel_close: Arc<dyn Fn(&str) + Send + Sync>,
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
    proxy: Option<ProxyInfo>,
    websocket_connection_timeout: Duration,
    websocket_close_timeout: Duration,
    disconnect_wait_timeout: Duration,
    tls_config: TlsConfig,
    user_agent: Option<String>,
}

impl SoraClientBuilder {
    fn new(
        context: Arc<SoraClientContext>,
        signaling_urls: Vec<String>,
        channel_id: String,
        role: Role,
    ) -> Self {
        Self {
            context,
            signaling_urls,
            channel_id,
            role,
            on_signaling_message: Arc::new(|_, _, _| {}),
            on_notify: Arc::new(|_| {}),
            on_push: Arc::new(|_| {}),
            on_track: Arc::new(|_| {}),
            on_remove_track: Arc::new(|_| {}),
            on_switched: Arc::new(|| {}),
            on_websocket_close: Arc::new(|_, _| {}),
            on_message: Arc::new(|_, _| {}),
            on_data_channel: Arc::new(|_| {}),
            on_data_channel_open: Arc::new(|_| {}),
            on_data_channel_message: Arc::new(|_, _| {}),
            on_data_channel_close: Arc::new(|_| {}),
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
            proxy: None,
            websocket_connection_timeout: Duration::from_secs(30),
            websocket_close_timeout: Duration::from_secs(3),
            disconnect_wait_timeout: Duration::from_secs(5),
            tls_config: TlsConfig::default(),
            user_agent: None,
        }
    }

    pub fn on_signaling_message<F>(mut self, handler: F) -> Self
    where
        F: Fn(SignalingType, SignalingDirection, &str) + Send + Sync + 'static,
    {
        self.on_signaling_message = Arc::new(handler);
        self
    }

    pub fn on_notify<F>(mut self, handler: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.on_notify = Arc::new(handler);
        self
    }

    pub fn on_push<F>(mut self, handler: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.on_push = Arc::new(handler);
        self
    }

    pub fn on_track<F>(mut self, handler: F) -> Self
    where
        F: Fn(RtpTransceiver) + Send + Sync + 'static,
    {
        self.on_track = Arc::new(handler);
        self
    }

    pub fn on_remove_track<F>(mut self, handler: F) -> Self
    where
        F: Fn(RtpReceiver) + Send + Sync + 'static,
    {
        self.on_remove_track = Arc::new(handler);
        self
    }

    pub fn on_switched<F>(mut self, handler: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.on_switched = Arc::new(handler);
        self
    }

    pub fn on_websocket_close<F>(mut self, handler: F) -> Self
    where
        F: Fn(Option<u16>, &str) + Send + Sync + 'static,
    {
        self.on_websocket_close = Arc::new(handler);
        self
    }

    pub fn on_message<F>(mut self, handler: F) -> Self
    where
        F: Fn(&str, &[u8]) + Send + Sync + 'static,
    {
        self.on_message = Arc::new(handler);
        self
    }

    pub fn on_data_channel<F>(mut self, handler: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.on_data_channel = Arc::new(handler);
        self
    }

    pub fn on_data_channel_open<F>(mut self, handler: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.on_data_channel_open = Arc::new(handler);
        self
    }

    pub fn on_data_channel_message<F>(mut self, handler: F) -> Self
    where
        F: Fn(&str, &[u8]) + Send + Sync + 'static,
    {
        self.on_data_channel_message = Arc::new(handler);
        self
    }

    pub fn on_data_channel_close<F>(mut self, handler: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.on_data_channel_close = Arc::new(handler);
        self
    }

    pub fn sender_video_track(mut self, track: VideoTrack) -> Self {
        self.sender_video_track = Some(track);
        self
    }

    pub fn sender_audio_track(mut self, track: AudioTrack) -> Self {
        self.sender_audio_track = Some(track);
        self
    }

    pub fn client_id(mut self, client_id: String) -> Self {
        self.client_id = Some(client_id);
        self
    }

    pub fn bundle_id(mut self, bundle_id: String) -> Self {
        self.bundle_id = Some(bundle_id);
        self
    }

    pub fn metadata(mut self, metadata: JsonString) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn audio(mut self, audio: Audio) -> Self {
        self.audio = Some(audio);
        self
    }

    pub fn video(mut self, video: Video) -> Self {
        self.video = Some(video);
        self
    }

    pub fn data_channel_signaling(mut self, value: bool) -> Self {
        self.data_channel_signaling = Some(value);
        self
    }

    pub fn ignore_disconnect_websocket(mut self, value: bool) -> Self {
        self.ignore_disconnect_websocket = Some(value);
        self
    }

    pub fn simulcast(mut self, value: bool) -> Self {
        self.simulcast = Some(value);
        self
    }

    pub fn simulcast_request_rid(mut self, value: String) -> Self {
        self.simulcast_request_rid = Some(value);
        self
    }

    pub fn spotlight(mut self, value: bool) -> Self {
        self.spotlight = Some(value);
        self
    }

    pub fn spotlight_focus_rid(mut self, value: String) -> Self {
        self.spotlight_focus_rid = Some(value);
        self
    }

    pub fn spotlight_unfocus_rid(mut self, value: String) -> Self {
        self.spotlight_unfocus_rid = Some(value);
        self
    }

    pub fn signaling_notify_metadata(mut self, value: JsonString) -> Self {
        self.signaling_notify_metadata = Some(value);
        self
    }

    pub fn data_channels(mut self, value: Vec<ConnectDataChannel>) -> Self {
        self.data_channels = Some(value);
        self
    }

    pub fn forwarding_filters(mut self, value: Vec<ForwardingFilter>) -> Self {
        self.forwarding_filters = Some(value);
        self
    }

    pub fn turn_tls_insecure(mut self, value: bool) -> Self {
        self.turn_tls_insecure = value;
        self
    }

    /// TURN-TLS の CA 証明書を DER エンコードで設定する。
    pub fn turn_tls_ca_cert(mut self, der: Vec<u8>) -> Self {
        self.turn_tls_ca_cert = Some(der);
        self
    }

    pub fn proxy(mut self, proxy: ProxyInfo) -> Self {
        self.proxy = Some(proxy);
        self
    }

    pub fn websocket_connection_timeout(mut self, value: Duration) -> Self {
        self.websocket_connection_timeout = value;
        self
    }

    pub fn websocket_close_timeout(mut self, value: Duration) -> Self {
        self.websocket_close_timeout = value;
        self
    }

    pub fn disconnect_wait_timeout(mut self, value: Duration) -> Self {
        self.disconnect_wait_timeout = value;
        self
    }

    /// サーバー証明書の検証をスキップする。
    pub fn insecure(mut self, value: bool) -> Self {
        self.tls_config.insecure = value;
        self
    }

    /// クライアント証明書と秘密鍵を設定する (PEM 形式)。
    pub fn client_cert(mut self, cert: String, key: String) -> Self {
        self.tls_config.client_cert = Some(cert);
        self.tls_config.client_key = Some(key);
        self
    }

    /// CA 証明書を設定する (PEM 形式)。
    pub fn ca_cert(mut self, cert: String) -> Self {
        self.tls_config.ca_cert = Some(cert);
        self
    }

    /// WebSocket 接続時の User-Agent ヘッダを設定する。
    ///
    /// 未設定の場合はデフォルト値 ("Sora Rust SDK {version}") が使用される。
    pub fn user_agent(mut self, value: String) -> Self {
        self.user_agent = Some(value);
        self
    }

    pub fn build(self) -> Result<(SoraClient, SoraClientHandle)> {
        SoraClient::new(self)
    }
}

/// 外部から SoraClient を制御するためのハンドル。
///
/// `SoraClient::run()` を別タスクで実行中に、このハンドルを使って
/// 接続の切断や統計情報の取得を行うことができる。
#[derive(Clone)]
pub struct SoraClientHandle {
    command_tx: mpsc::UnboundedSender<SoraClientCommand>,
}

impl SoraClientHandle {
    /// 最初に WebSocket 接続が成功したシグナリング URL を返す。
    ///
    /// `run()` で接続が確立される前は `None` を返す。
    /// `run()` が終了している場合はエラーを返す。
    pub async fn selected_signaling_url(&self) -> Result<Option<String>> {
        self.send_command(
            "selected_signaling_url",
            SoraClientCommand::GetSelectedSignalingUrl,
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
            SoraClientCommand::GetConnectedSignalingUrl,
        )
        .await
    }

    /// 接続を切断する。
    ///
    /// `run()` が切断要求を受け付けたことを確認してから戻る。
    pub async fn disconnect(&self) -> Result<()> {
        self.send_command("disconnect", SoraClientCommand::Disconnect)
            .await
    }

    /// DataChannel 経由で JSON-RPC 2.0 リクエストを送信する。
    ///
    /// SDK 内部で JSON-RPC 2.0 メッセージを組み立てて送信する。
    /// `options.notification` が `true` の場合はレスポンスを待たずに即座に `Ok(None)` を返す。
    /// `options.timeout` でレスポンスの待機タイムアウトを指定する (デフォルト 5 秒)。
    pub async fn send_rpc_request(
        &self,
        method: &str,
        params: Option<JsonString>,
        options: RpcRequestOptions,
    ) -> Result<Option<RpcResponse>> {
        self.send_command("send_rpc_request", |tx| SoraClientCommand::SendRpcRequest {
            method: method.to_string(),
            params,
            notification: options.notification,
            timeout: options.timeout,
            response_tx: tx,
        })
        .await?
    }

    /// DataChannel 経由でメッセージを送信する。
    ///
    /// `#` プレフィックス付きラベルのユーザー定義 DataChannel にバイナリデータを送信する。
    pub async fn send_message(&self, label: &str, data: &[u8]) -> Result<()> {
        self.send_command("send_message", |tx| SoraClientCommand::SendMessage {
            label: label.to_string(),
            data: data.to_vec(),
            response_tx: tx,
        })
        .await?
    }

    /// 統計情報を取得する。
    ///
    /// PeerConnection の統計情報を JSON 形式で取得する。
    pub async fn get_stats(&self) -> Result<JsonString> {
        self.send_command("get_stats", SoraClientCommand::GetStats)
            .await?
    }

    async fn send_command<R>(
        &self,
        command: &'static str,
        build: impl FnOnce(oneshot::Sender<R>) -> SoraClientCommand,
    ) -> Result<R> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(build(tx))
            .map_err(|source| Error::CommandSendFailed { source, command })?;
        rx.await
            .map_err(|source| Error::CommandResponseMissing { source, command })
    }
}

pub struct SoraClient {
    data_channels: HashMap<String, ManagedDataChannel>,
    data_channel_configs: Vec<DataChannelConfig>,
    pc: PeerConnection,
    // Observer を保持しておく必要がある (ドロップすると PeerConnection への通知が止まる)
    #[allow(dead_code)]
    pc_observer: PeerConnectionObserver,
    config: SoraClientBuilder,
    offer_simulcast: bool,
    simulcast_encodings: Vec<SimulcastEncodingConfig>,
    video_sender: Option<RtpSender>,
    command_rx: mpsc::UnboundedReceiver<SoraClientCommand>,
    event_tx: mpsc::UnboundedSender<SoraEvent>,
    event_rx: mpsc::UnboundedReceiver<SoraEvent>,
    pending_rpc_responses: HashMap<u64, PendingRpcRequest>,
    rpc_id_counter: u64,
    proxy: Option<ParsedProxyInfo>,
    selected_signaling_url: Option<String>,
    connected_signaling_url: Option<String>,
}

struct PendingRpcRequest {
    response_tx: oneshot::Sender<Result<Option<RpcResponse>>>,
    timeout_handle: JoinHandle<()>,
}

enum SoraEvent {
    SignalingMessage(String),
    DataChannelMessage { label: String, data: Vec<u8> },
    DataChannelRegister(DataChannel),
    DataChannelStateChange(String),
    RpcTimeout { id: u64 },
}

pub enum SoraClientCommand {
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

impl SoraClient {
    pub fn builder(
        context: Arc<SoraClientContext>,
        signaling_urls: Vec<String>,
        channel_id: String,
        role: Role,
    ) -> SoraClientBuilder {
        SoraClientBuilder::new(context, signaling_urls, channel_id, role)
    }

    fn new(config: SoraClientBuilder) -> Result<(Self, SoraClientHandle)> {
        let (command_tx, command_rx) = mpsc::unbounded_channel::<SoraClientCommand>();
        let handle = SoraClientHandle { command_tx };

        let (event_tx, event_rx) = mpsc::unbounded_channel::<SoraEvent>();
        let pc_factory = config.context.factory();
        let connection_context = config.context.connection_context();
        let on_track = config.on_track.clone();
        let on_remove_track = config.on_remove_track.clone();
        let event_tx_for_candidate = event_tx.clone();
        let event_tx_for_channel = event_tx.clone();
        struct PcObserverHandler {
            on_track: Arc<dyn Fn(RtpTransceiver) + Send + Sync>,
            on_remove_track: Arc<dyn Fn(RtpReceiver) + Send + Sync>,
            event_tx_for_candidate: mpsc::UnboundedSender<SoraEvent>,
            event_tx_for_channel: mpsc::UnboundedSender<SoraEvent>,
        }

        impl PeerConnectionObserverHandler for PcObserverHandler {
            fn on_connection_change(&mut self, new_state: PeerConnectionState) {
                rtc_log_info!("PeerConnection の状態: {:?}", new_state);
            }

            fn on_track(&mut self, transceiver: RtpTransceiver) {
                (self.on_track)(transceiver);
            }

            fn on_remove_track(&mut self, receiver: RtpReceiver) {
                (self.on_remove_track)(receiver);
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
            on_track,
            on_remove_track,
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
                &proxy.host,
                proxy.port,
                proxy.username.as_deref().unwrap_or(""),
                proxy.password.as_deref().unwrap_or(""),
                &proxy.user_agent,
            );
        }
        let mut rtc_config = PeerConnectionRtcConfiguration::new();
        let pc = PeerConnection::create(pc_factory, &mut rtc_config, &mut deps)?;

        let client = Self {
            data_channels: HashMap::new(),
            data_channel_configs: Vec::new(),
            pc,
            pc_observer: observer,
            config,
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
        };
        Ok((client, handle))
    }

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
        let on_signaling_message = self.config.on_signaling_message.clone();
        let on_notify = self.config.on_notify.clone();
        let on_push = self.config.on_push.clone();
        let on_switched = self.config.on_switched.clone();
        let on_websocket_close = self.config.on_websocket_close.clone();
        let on_message = self.config.on_message.clone();
        let on_data_channel = self.config.on_data_channel.clone();
        let on_data_channel_open = self.config.on_data_channel_open.clone();
        let on_data_channel_message = self.config.on_data_channel_message.clone();
        let on_data_channel_close = self.config.on_data_channel_close.clone();
        let proxy = self.proxy.clone();

        if signaling_urls.is_empty() {
            return Err(Error::SignalingUrlsEmpty);
        }

        // URL リストをランダム化して負荷分散する
        let mut urls = signaling_urls.clone();
        urls.shuffle(&mut rand::rng());

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
            "接続先: {}://{}:{}{}",
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
        let mut ws = WebSocketClientConnection::new(options, SecureRandom);
        ws.connect()?;
        if flush_ws_output(&mut ws, &mut stream, &mut timers).await? {
            return Ok(());
        }

        let mut redirect_location: Option<String> = None;
        let mut redirect = false;
        let mut use_datachannel_signaling = false;
        let mut websocket_closed = false;
        let mut switched_received = false;
        let mut switched_ignore_disconnect_websocket = false;
        let mut opened_datachannels = HashSet::<String>::new();
        let mut ws_disconnect_delay_start: Option<tokio::time::Instant> = None;
        const WS_DISCONNECT_DELAY: Duration = Duration::from_secs(10);
        let mut buf = vec![0u8; 8192];

        loop {
            tokio::select! {
                read = stream.read(&mut buf), if !websocket_closed => {
                    let n = read?;
                    if n == 0 {
                        if switched_ignore_disconnect_websocket {
                            rtc_log_info!(
                                "WebSocket 接続が閉じられました (DataChannel シグナリングを継続)"
                            );
                            websocket_closed = true;
                        } else {
                            rtc_log_info!("接続が閉じられました");
                            break;
                        }
                    } else {
                        ws.feed_recv_buf(&buf[..n], now())?;
                    }
                }
                Some(timer_id) = timer_rx.recv() => {
                    ws.handle_timer(timer_id)?;
                }
                Some(event) = self.event_rx.recv() => {
                    match event {
                        SoraEvent::SignalingMessage(message) => {
                            if use_datachannel_signaling {
                                on_signaling_message(SignalingType::DataChannel, SignalingDirection::Sent, &message);
                                self.send_signaling_message(&message)?;
                            } else if ws.state() == ConnectionState::Connected {
                                on_signaling_message(SignalingType::WebSocket, SignalingDirection::Sent, &message);
                                send_text(&mut ws, &message)?;
                            }
                        }
                        SoraEvent::DataChannelMessage { label, data } => {
                            self.handle_datachannel_message(
                                &label,
                                &data,
                                &on_signaling_message,
                                &on_notify,
                                &on_push,
                                &on_message,
                                &on_data_channel_message,
                            )
                            .await?;
                        }
                        SoraEvent::DataChannelRegister(channel) => {
                            let Ok(label) = channel.label() else {
                                continue;
                            };
                            rtc_log_info!("DataChannel '{}' を登録しました", label);
                            on_data_channel(&label);
                            self.register_data_channel(channel, &event_tx);
                            self.handle_datachannel_state(&label, &on_data_channel_open, &on_data_channel_close, &mut opened_datachannels, &mut use_datachannel_signaling);
                        }
                        SoraEvent::RpcTimeout { id } => {
                            if let Some(pending) = self.pending_rpc_responses.remove(&id) {
                                let _ = pending.response_tx.send(Err(Error::RpcTimeout));
                            }
                        }
                        SoraEvent::DataChannelStateChange(label) => {
                            self.handle_datachannel_state(&label, &on_data_channel_open, &on_data_channel_close, &mut opened_datachannels, &mut use_datachannel_signaling);
                        }
                    }
                }
                // ハンドル経由での切断要求
                Some(command) = self.command_rx.recv() => {
                    match command {
                        SoraClientCommand::Disconnect(ack_tx) => {
                            rtc_log_info!("切断要求を受信しました");
                            let _ = ack_tx.send(());
                            // オープン中の DataChannel に対して close コールバックを呼ぶ
                            for label in &opened_datachannels {
                                rtc_log_info!("DataChannel '{}' がクローズしました", label);
                                on_data_channel_close(label);
                            }
                            break;
                        }
                        SoraClientCommand::GetStats(stats_response_tx) => {
                            let stats = self.get_stats().await;
                            let _ = stats_response_tx.send(stats);
                        }
                        SoraClientCommand::GetSelectedSignalingUrl(response_tx) => {
                            let _ = response_tx.send(self.selected_signaling_url.clone());
                        }
                        SoraClientCommand::GetConnectedSignalingUrl(response_tx) => {
                            let _ = response_tx.send(self.connected_signaling_url.clone());
                        }
                        SoraClientCommand::SendRpcRequest { method, params, notification, timeout, response_tx } => {
                            let (message, rpc_id) = rpc::build_rpc_message(&mut self.rpc_id_counter, &method, params.as_ref(), notification);
                            let result = self.send_datachannel_message("rpc", &message);
                            match result {
                                Ok(()) => {
                                    if notification {
                                        let _ = response_tx.send(Ok(None));
                                    } else {
                                        let id = rpc_id.expect("notification でない場合は id が存在する");
                                        let event_tx = event_tx.clone();
                                        let timeout_handle = tokio::spawn(async move {
                                            tokio::time::sleep(timeout).await;
                                            let _ = event_tx.send(SoraEvent::RpcTimeout { id });
                                        });
                                        self.pending_rpc_responses.insert(id, PendingRpcRequest {
                                            response_tx,
                                            timeout_handle,
                                        });
                                    }
                                }
                                Err(e) => {
                                    let _ = response_tx.send(Err(e));
                                }
                            }
                        }
                        SoraClientCommand::SendMessage { label, data, response_tx } => {
                            let result = self.send_data_channel_message(&label, &data);
                            let _ = response_tx.send(result);
                        }
                    }
                }
                // WebSocket 切断待機中は期限まで待機
                _ = async {
                    if let Some(start) = ws_disconnect_delay_start {
                        tokio::time::sleep_until(start + WS_DISCONNECT_DELAY).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                }
            }

            while let Some(event) = ws.poll_event() {
                match event {
                    ConnectionEvent::Connected { .. } => {
                        rtc_log_info!("WebSocket 接続が完了しました");
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
                        on_signaling_message(
                            SignalingType::WebSocket,
                            SignalingDirection::Sent,
                            &connect_text,
                        );
                        send_text(&mut ws, &connect_text)?;
                    }
                    ConnectionEvent::TextMessage(text) => {
                        rtc_log_info!("[WebSocket] テキストメッセージを受信しました: {}", text);
                        let message = match IncomingMessage::parse(&text) {
                            Ok(message) => message,
                            Err(err) => {
                                rtc_log_error!("JSON メッセージの解析に失敗しました: {}", err);
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
                                on_signaling_message(
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
                                on_signaling_message(
                                    SignalingType::WebSocket,
                                    SignalingDirection::Sent,
                                    &answer_text,
                                );
                                send_text(&mut ws, &answer_text)?;
                            }
                            IncomingMessageData::ReOffer { sdp, ice_servers } => {
                                on_signaling_message(
                                    SignalingType::WebSocket,
                                    SignalingDirection::Received,
                                    &text,
                                );
                                let answer_sdp = self.handle_offer(&sdp, &ice_servers).await?;
                                let reanswer_message = OutgoingMessage::new_reanswer(&answer_sdp);
                                let reanswer_text = Json(reanswer_message).to_string();
                                on_signaling_message(
                                    SignalingType::WebSocket,
                                    SignalingDirection::Sent,
                                    &reanswer_text,
                                );
                                send_text(&mut ws, &reanswer_text)?;
                            }
                            IncomingMessageData::Ping { stats } => {
                                if stats.unwrap_or(false) {
                                    if self.request_stats_pong(&event_tx).is_err() {
                                        // エラー時は通常の pong を送信する
                                        self.send_pong(&event_tx);
                                    }
                                } else {
                                    self.send_pong(&event_tx);
                                }
                            }
                            IncomingMessageData::ReqStats {} => {
                                // 統計情報を含む stats メッセージを送信
                                self.request_stats_response(&event_tx);
                            }
                            IncomingMessageData::Notify {} => {
                                on_notify(&message.message);
                            }
                            IncomingMessageData::Push {} => {
                                on_push(&message.message);
                            }
                            IncomingMessageData::Switched {
                                ignore_disconnect_websocket: iws,
                            } => {
                                switched_received = true;
                                switched_ignore_disconnect_websocket = iws;
                                on_switched();
                            }
                            IncomingMessageData::Redirect { location } => {
                                on_signaling_message(
                                    SignalingType::WebSocket,
                                    SignalingDirection::Received,
                                    &text,
                                );
                                rtc_log_info!("リダイレクトメッセージを受信しました: {}", location);
                                redirect_location = Some(location);
                                break;
                            }
                            IncomingMessageData::Close {} => {
                                rtc_log_info!("Sora サーバーから切断されました");
                                break;
                            }
                        }
                    }
                    ConnectionEvent::BinaryMessage(data) => {
                        rtc_log_info!(
                            "[WebSocket] バイナリメッセージを受信しました: {} bytes",
                            data.len()
                        );
                    }
                    ConnectionEvent::Ping(_) => {
                        rtc_log_info!("[WebSocket] Ping を受信しました");
                    }
                    ConnectionEvent::Pong(_) => {
                        rtc_log_info!("[WebSocket] Pong を受信しました");
                    }
                    ConnectionEvent::Close { code, reason } => {
                        rtc_log_info!("[WebSocket] Close を受信しました: {:?} {}", code, reason);
                        on_websocket_close(code.map(|c| c.0), &reason);
                        break;
                    }
                    ConnectionEvent::StateChanged(state) => {
                        rtc_log_info!("[WebSocket] 状態: {:?}", state);
                    }
                    ConnectionEvent::Error(err) => {
                        rtc_log_error!("[WebSocket] エラー: {}", err);
                    }
                }
            }

            // redirect メッセージを受信した場合、新しい WebSocket に再接続する
            if let Some(location) = redirect_location.take() {
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
                    "リダイレクト先: {}://{}:{}{}",
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
                ws = WebSocketClientConnection::new(options, SecureRandom);
                ws.connect()?;
                websocket_closed = false;
                redirect = true;
                if flush_ws_output(&mut ws, &mut stream, &mut timers).await? {
                    break;
                }
                continue;
            }

            // DataChannel シグナリングへの切替条件が揃ったら WebSocket を切断する
            if switched_received && switched_ignore_disconnect_websocket && !websocket_closed {
                let expected = self.data_channel_configs.len();
                let opened = opened_datachannels.len();
                let all_open = expected > 0 && opened >= expected;

                if all_open && ws_disconnect_delay_start.is_none() {
                    ws_disconnect_delay_start = Some(tokio::time::Instant::now());
                }

                if let Some(start) = ws_disconnect_delay_start
                    && start.elapsed() >= WS_DISCONNECT_DELAY
                    && ws.state() == ConnectionState::Connected
                {
                    ws.close(CloseCode::NORMAL, "switching to datachannel")?;
                }
            }

            if let Ok(true) = flush_ws_output(&mut ws, &mut stream, &mut timers).await {
                break;
            }

            if ws.state() == ConnectionState::Closed {
                break;
            }
        }

        // DataChannel が使用されている場合、クローズ完了を待機する
        if use_datachannel_signaling && !opened_datachannels.is_empty() {
            let deadline = tokio::time::Instant::now() + disconnect_wait_timeout;
            while !opened_datachannels.is_empty() {
                let timeout_result = tokio::time::timeout_at(deadline, async {
                    while let Some(event) = self.event_rx.recv().await {
                        if let SoraEvent::DataChannelStateChange(label) = event {
                            return label;
                        }
                    }
                    String::new()
                })
                .await;
                match timeout_result {
                    Ok(label) if !label.is_empty() => {
                        if opened_datachannels.remove(&label) {
                            rtc_log_info!("DataChannel '{}' がクローズしました", label);
                            on_data_channel_close(&label);
                        }
                    }
                    _ => {
                        rtc_log_warning!(
                            "切断待機がタイムアウトしました (残り {} チャネル)",
                            opened_datachannels.len()
                        );
                        break;
                    }
                }
            }
        }

        if ws.state() == ConnectionState::Connected {
            ws.close(CloseCode::NORMAL, "shutdown")?;
            let close_result = tokio::time::timeout(websocket_close_timeout, async {
                loop {
                    if flush_ws_output(&mut ws, &mut stream, &mut timers).await? {
                        return Ok::<_, Error>(());
                    }
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await?;
                    if n == 0 {
                        return Ok(());
                    }
                    ws.feed_recv_buf(&buf[..n], now())?;
                    while let Some(_event) = ws.poll_event() {}
                    if ws.state() == ConnectionState::Closed {
                        return Ok(());
                    }
                }
            })
            .await;
            if close_result.is_err() {
                rtc_log_warning!("WebSocket クローズがタイムアウトしました");
            }
        }

        rtc_log_info!("終了します");
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

        let sender = self.video_sender.as_mut().unwrap();
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

    fn apply_pc_configuration(&mut self, servers: &[IceServerConfig]) -> Result<()> {
        if servers.is_empty() {
            return Ok(());
        }
        let pc = &mut self.pc;
        let mut config = PeerConnectionRtcConfiguration::new();
        for server in servers {
            let mut server_entry = IceServer::new();
            for url in &server.urls {
                server_entry.add_url(url);
            }
            if let Some(user) = &server.username {
                server_entry.set_username(user);
            }
            if let Some(pass) = &server.credential {
                server_entry.set_password(pass);
            }
            if self.config.turn_tls_insecure {
                server_entry.set_tls_cert_policy(TlsCertPolicy::InsecureNoCheck);
            }
            config.servers().push(&server_entry);
        }
        pc.set_configuration(&mut config)?;
        Ok(())
    }

    fn send_pong(&self, event_tx: &mpsc::UnboundedSender<SoraEvent>) {
        let message = OutgoingMessage::new_pong(None);
        let _ = event_tx.send(SoraEvent::SignalingMessage(Json(message).to_string()));
    }

    fn request_stats_pong(&self, event_tx: &mpsc::UnboundedSender<SoraEvent>) -> Result<()> {
        let pc = &self.pc;
        let event_tx = event_tx.clone();
        pc.get_stats(move |report| {
            let message = OutgoingMessage::new_pong(
                report
                    .to_json()
                    .map_err(Error::from)
                    .and_then(|s| s.parse())
                    .ok(),
            );
            let _ = event_tx.send(SoraEvent::SignalingMessage(Json(message).to_string()));
        });
        Ok(())
    }

    fn request_stats_response(&self, event_tx: &mpsc::UnboundedSender<SoraEvent>) {
        let pc = &self.pc;
        let event_tx = event_tx.clone();
        pc.get_stats(move |report| {
            if let Ok(reports) = report.to_json()
                && let Ok(reports) = reports.parse()
            {
                let message = OutgoingMessage::new_stats(reports);
                let _ = event_tx.send(SoraEvent::SignalingMessage(Json(message).to_string()));
            }
        });
    }

    async fn get_stats(&mut self) -> Result<JsonString> {
        let pc = &self.pc;
        let (tx, rx) = oneshot::channel();
        pc.get_stats(move |report| {
            let r = report.to_json();
            let _ = tx.send(r);
        });
        let json = rx.await.map_err(|e| Error::CommandResponseMissing {
            source: e,
            command: "get_stats",
        })??;
        json.parse()
    }

    async fn handle_offer(&mut self, sdp: &str, ice_servers: &[IceServerConfig]) -> Result<String> {
        self.apply_pc_configuration(ice_servers)?;

        let (rem_tx, mut rem_rx) = mpsc::unbounded_channel::<Option<String>>();
        {
            let pc = &self.pc;
            let offer = SessionDescription::new(SdpType::Offer, sdp)?;
            struct RemObsHandler {
                tx: mpsc::UnboundedSender<Option<String>>,
            }

            impl SetRemoteDescriptionObserverHandler for RemObsHandler {
                fn on_set_remote_description_complete(&mut self, error: RtcError) {
                    let msg = if error.ok() {
                        None
                    } else {
                        Some(error.message().unwrap_or_else(|_| "unknown".to_string()))
                    };
                    let _ = self.tx.send(msg);
                }
            }

            let rem_obs = SetRemoteDescriptionObserver::new_with_handler(Box::new(RemObsHandler {
                tx: rem_tx,
            }));
            pc.set_remote_description(offer, &rem_obs);
        }
        let rem_res = tokio::time::timeout(Duration::from_secs(5), rem_rx.recv())
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
        let answer_sdp = tokio::time::timeout(Duration::from_secs(5), ans_rx.recv())
            .await
            .map_err(|_| Error::AnswerTimeout)?
            .ok_or_else(|| Error::AnswerResponseMissing)??;

        let answer = SessionDescription::new(SdpType::Answer, &answer_sdp)?;
        let (loc_tx, mut loc_rx) = mpsc::unbounded_channel::<Option<String>>();
        struct LocObsHandler {
            tx: mpsc::UnboundedSender<Option<String>>,
        }

        impl SetLocalDescriptionObserverHandler for LocObsHandler {
            fn on_set_local_description_complete(&mut self, error: RtcError) {
                let msg = if error.ok() {
                    None
                } else {
                    Some(error.message().unwrap_or_else(|_| "unknown".to_string()))
                };
                let _ = self.tx.send(msg);
            }
        }

        let loc_obs =
            SetLocalDescriptionObserver::new_with_handler(Box::new(LocObsHandler { tx: loc_tx }));
        {
            let pc = &self.pc;
            pc.set_local_description(answer, &loc_obs);
        }
        let loc_res = tokio::time::timeout(Duration::from_secs(5), loc_rx.recv())
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

    fn is_datachannel_open(&self, label: &str) -> bool {
        self.data_channels
            .get(label)
            .is_some_and(|m| m.channel.state() == DataChannelState::Open)
    }

    fn is_datachannel_closed(&self, label: &str) -> bool {
        self.data_channels
            .get(label)
            .is_some_and(|m| m.channel.state() == DataChannelState::Closed)
    }

    fn handle_datachannel_state(
        &self,
        label: &str,
        on_data_channel_open: &Arc<dyn Fn(&str) + Send + Sync>,
        on_data_channel_close: &Arc<dyn Fn(&str) + Send + Sync>,
        opened_datachannels: &mut HashSet<String>,
        use_datachannel_signaling: &mut bool,
    ) {
        if self.is_datachannel_open(label) && !opened_datachannels.contains(label) {
            rtc_log_info!("DataChannel '{}' がオープンしました", label);
            opened_datachannels.insert(label.to_string());
            on_data_channel_open(label);
            if opened_datachannels.len() == self.data_channel_configs.len() {
                *use_datachannel_signaling = true;
            }
        } else if self.is_datachannel_closed(label) && opened_datachannels.contains(label) {
            rtc_log_info!("DataChannel '{}' がクローズしました", label);
            opened_datachannels.remove(label);
            on_data_channel_close(label);
        }
    }

    fn send_signaling_message(&mut self, message: &str) -> Result<()> {
        self.send_datachannel_message("signaling", message)
    }

    fn send_stats_message(&mut self, message: &str) -> Result<()> {
        self.send_datachannel_message("stats", message)
    }

    fn send_datachannel_message(&mut self, label: &str, message: &str) -> Result<()> {
        let managed =
            self.data_channels
                .get_mut(label)
                .ok_or_else(|| Error::DataChannelMissing {
                    label: label.to_string(),
                })?;

        let data = if managed.compress {
            compress_zlib(message.as_bytes())?
        } else {
            message.as_bytes().to_vec()
        };

        rtc_log_info!("DataChannel '{}' にメッセージを送信: {}", label, &message);

        if !managed.channel.send(&data, true) {
            return Err(Error::DataChannelSendFailed);
        }
        Ok(())
    }

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

        rtc_log_info!(
            "DataChannel '{}' にバイナリメッセージを送信: {} bytes",
            label,
            send_data.len()
        );

        if !managed.channel.send(&send_data, true) {
            return Err(Error::DataChannelSendFailed);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    async fn handle_datachannel_message(
        &mut self,
        label: &str,
        data: &[u8],
        on_signaling_message: &Arc<dyn Fn(SignalingType, SignalingDirection, &str) + Send + Sync>,
        on_notify: &Arc<dyn Fn(&str) + Send + Sync>,
        on_push: &Arc<dyn Fn(&str) + Send + Sync>,
        on_message: &Arc<dyn Fn(&str, &[u8]) + Send + Sync>,
        on_data_channel_message: &Arc<dyn Fn(&str, &[u8]) + Send + Sync>,
    ) -> Result<()> {
        // DataChannel の設定を検索
        let managed = self.data_channels.get(label);
        let compress = managed.is_some_and(|m| m.compress);

        // 圧縮されている場合は展開
        let message_bytes = if compress {
            decompress_zlib(data)?
        } else {
            data.to_vec()
        };

        rtc_log_info!(
            "DataChannel '{}' からメッセージを受信: {}",
            label,
            String::from_utf8_lossy(&message_bytes)
        );

        // DataChannel API コールバック (全ラベル)
        on_data_channel_message(label, &message_bytes);

        match label {
            // signaling, stats, push, notify ラベルの場合はシグナリングメッセージとして処理
            "signaling" | "stats" | "push" | "notify" => {
                let text = String::from_utf8(message_bytes)?;
                // signaling ラベルのみ on_signaling_message を呼ぶ
                if label == "signaling" {
                    on_signaling_message(
                        SignalingType::DataChannel,
                        SignalingDirection::Received,
                        &text,
                    );
                }
                // メッセージをパースして処理
                let incoming = IncomingMessage::parse(&text)?;
                match incoming.data {
                    IncomingMessageData::ReOffer { sdp, ice_servers } => {
                        let answer_sdp = self.handle_offer(&sdp, &ice_servers).await?;
                        let reanswer_message = OutgoingMessage::new_reanswer(&answer_sdp);
                        let reanswer_text = Json(reanswer_message).to_string();
                        on_signaling_message(
                            SignalingType::DataChannel,
                            SignalingDirection::Sent,
                            &reanswer_text,
                        );
                        self.send_signaling_message(&reanswer_text)?;
                    }
                    IncomingMessageData::Ping { .. } => {
                        let pong = OutgoingMessage::new_pong(None);
                        let pong_text = Json(pong).to_string();
                        on_signaling_message(
                            SignalingType::DataChannel,
                            SignalingDirection::Sent,
                            &pong_text,
                        );
                        self.send_signaling_message(&pong_text)?;
                    }
                    IncomingMessageData::ReqStats {} => {
                        // 統計情報を含む stats メッセージを送信（stats チャンネル経由）
                        let reports = self.get_stats().await;
                        if let Ok(reports) = reports {
                            let stats = OutgoingMessage::new_stats(reports);
                            self.send_stats_message(&Json(stats).to_string())?;
                        }
                    }
                    IncomingMessageData::Notify {} => {
                        on_notify(&text);
                    }
                    IncomingMessageData::Push {} => {
                        on_push(&text);
                    }
                    _ => {
                        rtc_log_warning!("DataChannel 経由で未対応のメッセージを受信しました");
                    }
                }
            }
            "rpc" => {
                let text = String::from_utf8(message_bytes)?;
                rtc_log_info!("DataChannel 経由で RPC メッセージを受信しました");
                let (id, response) = RpcResponse::parse(&text)?;
                if let Some(id) = id
                    && let Some(pending) = self.pending_rpc_responses.remove(&id)
                {
                    pending.timeout_handle.abort();
                    let _ = pending.response_tx.send(Ok(Some(response)));
                }
            }
            // # で始まるラベルはユーザー定義メッセージとして処理
            label if label.starts_with('#') => {
                on_message(label, &message_bytes);
            }
            _ => {
                rtc_log_warning!("DataChannel 経由で未対応のラベルを受信しました: {}", label);
            }
        }

        Ok(())
    }
}

impl Drop for SoraClient {
    fn drop(&mut self) {
        // DataChannel は PeerConnection のシグナリングスレッドを使用するため、
        // PeerConnection より先にドロップする必要がある
        self.data_channels.clear();
        self.video_sender.take();
    }
}

// -------------------------
// 管理対象 DataChannel
// -------------------------

struct ManagedDataChannel {
    channel: DataChannel,
    // Observer を保持しておく必要がある (ドロップすると DataChannel への通知が止まる)
    #[allow(dead_code)]
    observer: DataChannelObserver,
    compress: bool,
}

const DEFAULT_TLS_PORT: u16 = 443;
const DEFAULT_PLAIN_PORT: u16 = 80;

struct SecureRandom;

impl RandomSource for SecureRandom {
    fn masking_key(&mut self) -> [u8; 4] {
        let mut key = [0u8; 4];
        SystemRandom::new()
            .fill(&mut key)
            .expect("failed to generate masking key");
        key
    }

    fn nonce(&mut self) -> [u8; 16] {
        let mut nonce = [0u8; 16];
        SystemRandom::new()
            .fill(&mut nonce)
            .expect("failed to generate nonce");
        nonce
    }
}

fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    Timestamp::from_millis(millis)
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

#[derive(Debug, Clone)]
struct ParsedProxyInfo {
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    user_agent: String,
}

impl ParsedProxyInfo {
    fn parse(proxy: &ProxyInfo) -> Result<ParsedProxyInfo> {
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
            "HTTP Proxy 経由で接続します: {}:{}",
            format_bracketed_host(&proxy.host),
            proxy.port
        );
        let tcp_stream = connect_tcp(&proxy.host, proxy.port, deadline).await?;
        let mut stream = ClientStream::new_plain(tcp_stream);
        connect_http_proxy_tunnel(&mut stream, target, proxy).await?;
        if target.tls {
            let (tcp_stream, pending) = stream
                .into_plain_parts()
                .expect("BUG: proxy 接続後は plain stream のはずです");
            let tls_stream = connect_tls(&target.host, tcp_stream, tls_config, deadline).await?;
            let mut stream = ClientStream::new_tls(tls_stream);
            stream.push_pending_read(pending);
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
    let (ipv4_addrs, ipv6_addrs): (Vec<_>, Vec<_>) =
        addrs.into_iter().partition(|addr| addr.is_ipv4());
    let ordered_addrs = [ipv4_addrs, ipv6_addrs].concat();

    let tcp_stream =
        tokio::time::timeout_at(deadline, TcpStream::connect(ordered_addrs.as_slice()))
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
    let mut request = Request::new("CONNECT", &authority)
        .header("Host", &authority)
        .header("User-Agent", &proxy.user_agent);
    if proxy.username.is_some() || proxy.password.is_some() {
        let username = proxy.username.as_deref().unwrap_or("");
        let password = proxy.password.as_deref().unwrap_or("");
        let auth = BasicAuth::new(username, password)?;
        let header = auth.to_header_value();
        request = request.header("Proxy-Authorization", &header);
    }
    Ok(request.encode())
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
) -> Result<()> {
    let request = build_proxy_connect_request(target, proxy)?;
    stream.write_all(&request).await?;

    let mut decoder = ResponseDecoder::new();
    decoder.set_request_method("CONNECT");
    let mut buf = vec![0u8; 8192];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(Error::ProxyConnectResponseMissing);
        }
        decoder.feed(&buf[..n])?;
        if let Some((head, _body_kind)) = decoder.decode_headers()? {
            ensure_proxy_connect_status_success(head.status_code, &head.reason_phrase)?;
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
                    "接続試行: {}://{}:{}{}",
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
                    "接続成功: {}://{}:{}{}",
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
                rtc_log_warning!("接続失敗: {}: {}", url, e);
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

fn send_text<R: RandomSource>(ws: &mut WebSocketClientConnection<R>, text: &str) -> Result<()> {
    rtc_log_info!("[WebSocket] テキストメッセージを送信しました: {}", text);
    ws.send_text(text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::future::Future;

    fn proxy_info_with_url(url: String) -> ProxyInfo {
        ProxyInfo {
            url,
            ..Default::default()
        }
    }

    fn block_on_test(fut: impl Future<Output = ()>) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("テスト用 runtime の構築に失敗しました");
        runtime.block_on(fut);
    }

    proptest! {
        #[test]
        fn url_getters_roundtrip_command_response(
            selected in "[ -~]{1,64}",
            connected in "[ -~]{1,64}"
        ) {
            block_on_test(async move {
                let (command_tx, mut command_rx) = mpsc::unbounded_channel::<SoraClientCommand>();
                let handle = SoraClientHandle { command_tx };

                let selected_value = selected.clone();
                let connected_value = connected.clone();
                let worker = tokio::spawn(async move {
                    while let Some(command) = command_rx.recv().await {
                        match command {
                            SoraClientCommand::GetSelectedSignalingUrl(response_tx) => {
                                let _ = response_tx.send(Some(selected_value.clone()));
                            }
                            SoraClientCommand::GetConnectedSignalingUrl(response_tx) => {
                                let _ = response_tx.send(Some(connected_value.clone()));
                            }
                            _ => {}
                        }
                    }
                });

                let actual_selected = handle
                    .selected_signaling_url()
                    .await
                    .expect("selected_signaling_url の取得に失敗しました");
                let actual_connected = handle
                    .connected_signaling_url()
                    .await
                    .expect("connected_signaling_url の取得に失敗しました");

                assert_eq!(actual_selected, Some(selected));
                assert_eq!(actual_connected, Some(connected));

                drop(handle);
                worker
                    .await
                    .expect("URL 応答タスクが panic しました");
            });
        }

        #[test]
        fn parse_proxy_url_accepts_http(
            label in "[a-z][a-z0-9]{0,15}",
            port in 1u16..=65535
        ) {
            let proxy = proxy_info_with_url(format!("http://{label}:{port}"));
            let parsed = ParsedProxyInfo::parse(&proxy).expect("http proxy URL の解析に失敗しました");
            prop_assert_eq!(parsed.host, label);
            prop_assert_eq!(parsed.port, port);
        }

        #[test]
        fn parse_proxy_url_rejects_https(
            label in "[a-z][a-z0-9]{0,15}",
            port in 1u16..=65535
        ) {
            let proxy = proxy_info_with_url(format!("https://{label}:{port}"));
            let err = ParsedProxyInfo::parse(&proxy).expect_err("https proxy URL は拒否される必要があります");
            match err {
                Error::ProxyUrlUnsupportedScheme { .. } => {}
                _ => prop_assert!(false),
            }
        }

        #[test]
        fn parse_proxy_url_rejects_socks(
            label in "[a-z][a-z0-9]{0,15}",
            port in 1u16..=65535,
            scheme in prop_oneof![Just("socks"), Just("socks4"), Just("socks5")]
        ) {
            let proxy = proxy_info_with_url(format!("{scheme}://{label}:{port}"));
            let err = ParsedProxyInfo::parse(&proxy).expect_err("socks proxy URL は拒否される必要があります");
            match err {
                Error::ProxyUrlUnsupportedScheme { .. } => {}
                _ => prop_assert!(false),
            }
        }

        #[test]
        fn parse_proxy_url_rejects_userinfo(
            label in "[a-z][a-z0-9]{0,15}",
            port in 1u16..=65535
        ) {
            let proxy = proxy_info_with_url(format!("http://user:pass@{label}:{port}"));
            let err = ParsedProxyInfo::parse(&proxy).expect_err("userinfo 付き proxy URL は拒否される必要があります");
            prop_assert!(matches!(err, Error::ProxyUrlUserinfoNotSupported));
        }
    }

    #[test]
    fn parse_proxy_info_uses_default_user_agent_when_absent() {
        let proxy = proxy_info_with_url("http://proxy.example.com:8080".to_string());
        let parsed = ParsedProxyInfo::parse(&proxy).expect("proxy URL の解析に失敗しました");
        assert_eq!(parsed.user_agent, crate::version::get_sora_client_name());
    }

    #[test]
    fn parse_proxy_info_preserves_empty_user_agent_when_present() {
        let proxy = ProxyInfo {
            url: "http://proxy.example.com:8080".to_string(),
            user_agent: Some(String::new()),
            ..Default::default()
        };
        let parsed = ParsedProxyInfo::parse(&proxy).expect("proxy URL の解析に失敗しました");
        assert_eq!(parsed.user_agent, "");
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
        let (command_tx, command_rx) = mpsc::unbounded_channel::<SoraClientCommand>();
        drop(command_rx);
        let handle = SoraClientHandle { command_tx };

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
        ensure_proxy_connect_status_success(head.status_code, &head.reason_phrase)
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
        let err = ensure_proxy_connect_status_success(head.status_code, &head.reason_phrase)
            .expect_err("非 2xx は失敗扱いである必要があります");
        assert!(matches!(
            err,
            Error::ProxyConnectStatusNotSuccessful {
                status_code: 407,
                ..
            }
        ));
    }
}
