use std::io;
use std::sync::Arc;
use std::time::Duration;

use shiguredo_webrtc::{AudioTrack, FrameTransformerHandler, IceServer, VideoTrack};
use sora_sdk::{
    Audio, ConnectDataChannel, ForwardingFilter, JsonString, ProxyInfo, Result, Role,
    RpcRequestOptions, RpcResponse, SignalingDirection, SignalingType, SoraConnection,
    SoraConnectionBuilder, SoraConnectionContext, SoraConnectionEventHandler, SoraConnectionHandle,
    Video,
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::Instant;

const STATS_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// `SoraTestConnection` が保持するイベントログの要素。
///
/// callback から受け取った引数はこの enum に正規化して保存し、
/// 後段の predicate 判定で再利用する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoraTestEvent {
    SignalingMessage {
        signaling_type: SignalingType,
        direction: SignalingDirection,
        text: String,
    },
    Notify {
        message: String,
    },
    Push {
        message: String,
    },
    Track {
        kind: Option<String>,
    },
    RemoveTrack {
        kind: Option<String>,
    },
    Switched,
    WebsocketClose {
        code: Option<u16>,
        reason: String,
    },
    Message {
        label: String,
        data: Vec<u8>,
    },
    DataChannel {
        label: String,
    },
    DataChannelOpen {
        label: String,
    },
    DataChannelMessage {
        label: String,
        data: Vec<u8>,
    },
    DataChannelClose {
        label: String,
    },
}

/// `SoraConnection` のイベントを `SoraTestEvent` に変換する内部ハンドラ。
struct SoraTestEventHandler {
    event_tx: UnboundedSender<SoraTestEvent>,
}

impl SoraConnectionEventHandler for SoraTestEventHandler {
    fn on_signaling_message(
        &mut self,
        signaling_type: SignalingType,
        direction: SignalingDirection,
        text: &str,
    ) {
        let _ = self.event_tx.send(SoraTestEvent::SignalingMessage {
            signaling_type,
            direction,
            text: text.to_string(),
        });
    }
    fn on_notify(&mut self, message: &str) {
        let _ = self.event_tx.send(SoraTestEvent::Notify {
            message: message.to_string(),
        });
    }
    fn on_push(&mut self, message: &str) {
        let _ = self.event_tx.send(SoraTestEvent::Push {
            message: message.to_string(),
        });
    }
    fn on_track(&mut self, transceiver: shiguredo_webrtc::RtpTransceiver) {
        let kind = transceiver.receiver().track().kind().ok();
        let _ = self.event_tx.send(SoraTestEvent::Track { kind });
    }
    fn on_remove_track(&mut self, receiver: shiguredo_webrtc::RtpReceiver) {
        let kind = receiver.track().kind().ok();
        let _ = self.event_tx.send(SoraTestEvent::RemoveTrack { kind });
    }
    fn on_switched(&mut self) {
        let _ = self.event_tx.send(SoraTestEvent::Switched);
    }
    fn on_websocket_close(&mut self, code: Option<u16>, reason: &str) {
        let _ = self.event_tx.send(SoraTestEvent::WebsocketClose {
            code,
            reason: reason.to_string(),
        });
    }
    fn on_message(&mut self, label: &str, data: &[u8]) {
        let _ = self.event_tx.send(SoraTestEvent::Message {
            label: label.to_string(),
            data: data.to_vec(),
        });
    }
    fn on_data_channel(&mut self, label: &str) {
        let _ = self.event_tx.send(SoraTestEvent::DataChannel {
            label: label.to_string(),
        });
    }
    fn on_data_channel_open(&mut self, label: &str) {
        let _ = self.event_tx.send(SoraTestEvent::DataChannelOpen {
            label: label.to_string(),
        });
    }
    fn on_data_channel_message(&mut self, label: &str, data: &[u8]) {
        let _ = self.event_tx.send(SoraTestEvent::DataChannelMessage {
            label: label.to_string(),
            data: data.to_vec(),
        });
    }
    fn on_data_channel_close(&mut self, label: &str) {
        let _ = self.event_tx.send(SoraTestEvent::DataChannelClose {
            label: label.to_string(),
        });
    }
}

/// `SoraConnectionBuilder` をテスト向けに包むビルダー。
///
/// production API とほぼ同じ設定項目を公開しつつ、`connect()` 時に
/// イベントを `SoraTestEvent` へ集約する。
pub struct SoraTestConnectionBuilder {
    inner: SoraConnectionBuilder,
    event_rx: UnboundedReceiver<SoraTestEvent>,
}

impl SoraTestConnectionBuilder {
    pub fn sender_video_track(mut self, track: VideoTrack) -> Self {
        self.inner = self.inner.sender_video_track(track);
        self
    }

    pub fn sender_audio_track(mut self, track: AudioTrack) -> Self {
        self.inner = self.inner.sender_audio_track(track);
        self
    }

    pub fn sender_video_transform(
        mut self,
        transform: Box<dyn FrameTransformerHandler + Send>,
    ) -> Self {
        self.inner = self.inner.sender_video_transform(transform);
        self
    }

    pub fn receiver_video_transform(
        mut self,
        transform: Box<dyn FrameTransformerHandler + Send>,
    ) -> Self {
        self.inner = self.inner.receiver_video_transform(transform);
        self
    }

    pub fn client_id(mut self, client_id: String) -> Self {
        self.inner = self.inner.client_id(client_id);
        self
    }

    pub fn bundle_id(mut self, bundle_id: String) -> Self {
        self.inner = self.inner.bundle_id(bundle_id);
        self
    }

    pub fn metadata(mut self, metadata: JsonString) -> Self {
        self.inner = self.inner.metadata(metadata);
        self
    }

    pub fn audio(mut self, audio: Audio) -> Self {
        self.inner = self.inner.audio(audio);
        self
    }

    pub fn video(mut self, video: Video) -> Self {
        self.inner = self.inner.video(video);
        self
    }

    pub fn data_channel_signaling(mut self, value: bool) -> Self {
        self.inner = self.inner.data_channel_signaling(value);
        self
    }

    pub fn ignore_disconnect_websocket(mut self, value: bool) -> Self {
        self.inner = self.inner.ignore_disconnect_websocket(value);
        self
    }

    pub fn simulcast(mut self, value: bool) -> Self {
        self.inner = self.inner.simulcast(value);
        self
    }

    pub fn simulcast_request_rid(mut self, value: String) -> Self {
        self.inner = self.inner.simulcast_request_rid(value);
        self
    }

    pub fn spotlight(mut self, value: bool) -> Self {
        self.inner = self.inner.spotlight(value);
        self
    }

    pub fn spotlight_focus_rid(mut self, value: String) -> Self {
        self.inner = self.inner.spotlight_focus_rid(value);
        self
    }

    pub fn spotlight_unfocus_rid(mut self, value: String) -> Self {
        self.inner = self.inner.spotlight_unfocus_rid(value);
        self
    }

    pub fn signaling_notify_metadata(mut self, value: JsonString) -> Self {
        self.inner = self.inner.signaling_notify_metadata(value);
        self
    }

    pub fn data_channels(mut self, value: Vec<ConnectDataChannel>) -> Self {
        self.inner = self.inner.data_channels(value);
        self
    }

    pub fn forwarding_filters(mut self, value: Vec<ForwardingFilter>) -> Self {
        self.inner = self.inner.forwarding_filters(value);
        self
    }

    pub fn turn_tls_insecure(mut self, value: bool) -> Self {
        self.inner = self.inner.turn_tls_insecure(value);
        self
    }

    pub fn turn_tls_ca_cert(mut self, der: Vec<u8>) -> Self {
        self.inner = self.inner.turn_tls_ca_cert(der);
        self
    }

    pub fn proxy(mut self, proxy: ProxyInfo) -> Self {
        self.inner = self.inner.proxy(proxy);
        self
    }

    pub fn ice_server_url_configurer<F>(mut self, configurer: F) -> Self
    where
        F: Fn(&mut IceServer, &[String]) + Send + 'static,
    {
        self.inner = self.inner.ice_server_url_configurer(configurer);
        self
    }

    pub fn websocket_connection_timeout(mut self, value: Duration) -> Self {
        self.inner = self.inner.websocket_connection_timeout(value);
        self
    }

    pub fn websocket_close_timeout(mut self, value: Duration) -> Self {
        self.inner = self.inner.websocket_close_timeout(value);
        self
    }

    pub fn disconnect_wait_timeout(mut self, value: Duration) -> Self {
        self.inner = self.inner.disconnect_wait_timeout(value);
        self
    }

    pub fn insecure(mut self, value: bool) -> Self {
        self.inner = self.inner.insecure(value);
        self
    }

    pub fn client_cert(mut self, cert: String, key: String) -> Self {
        self.inner = self.inner.client_cert(cert, key);
        self
    }

    pub fn ca_cert(mut self, cert: String) -> Self {
        self.inner = self.inner.ca_cert(cert);
        self
    }

    pub fn user_agent(mut self, value: String) -> Self {
        self.inner = self.inner.user_agent(value);
        self
    }

    /// 接続を開始し、run ループをバックグラウンド task で起動する。
    pub fn connect(self) -> Result<SoraTestConnection> {
        let (connection, handle) = self.inner.build()?;
        let run_task = tokio::spawn(async move { connection.run().await });

        Ok(SoraTestConnection {
            handle,
            event_rx: self.event_rx,
            event_log: Vec::new(),
            run_task,
            run_task_joined: false,
            run_task_error_message: None,
        })
    }
}

/// e2e テスト用の接続ラッパー。
///
/// - `event_rx` / `event_log` でイベント履歴を保持
/// - `wait_for_*` 系 API で過去ログ + 新規受信の両方を判定
/// - `run_task` の終了待機まで含めて接続ライフサイクルを管理
///
/// `run_task` の結果は 1 回しか読み出せない。初回読み出し時に結果を保持し、
/// 2 回目以降の読み出しは保持済みの結果を使う。
pub struct SoraTestConnection {
    handle: SoraConnectionHandle,
    event_rx: UnboundedReceiver<SoraTestEvent>,
    event_log: Vec<SoraTestEvent>,
    run_task: JoinHandle<Result<()>>,
    /// `run_task` の `JoinHandle` を await 済みかどうか。
    run_task_joined: bool,
    /// `run_task` が `Err` または panic で終了した場合のエラーメッセージ。
    run_task_error_message: Option<String>,
}

impl SoraTestConnection {
    /// `SoraTestConnectionBuilder` を生成する。
    ///
    /// 内部で `SoraTestEventHandler` を生成し、
    /// `SoraConnection::builder()` の第 5 引数として渡す。
    pub fn builder(
        context: Arc<SoraConnectionContext>,
        signaling_urls: Vec<String>,
        channel_id: String,
        role: Role,
    ) -> SoraTestConnectionBuilder {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let event_handler = SoraTestEventHandler { event_tx };
        let inner =
            SoraConnection::builder(context, signaling_urls, channel_id, role, event_handler);
        SoraTestConnectionBuilder { inner, event_rx }
    }

    pub async fn selected_signaling_url(&self) -> Result<Option<String>> {
        self.handle.selected_signaling_url().await
    }

    pub async fn connected_signaling_url(&self) -> Result<Option<String>> {
        self.handle.connected_signaling_url().await
    }

    pub async fn send_rpc_request(
        &self,
        method: &str,
        params: Option<JsonString>,
        options: RpcRequestOptions,
    ) -> Result<Option<RpcResponse>> {
        self.handle.send_rpc_request(method, params, options).await
    }

    pub async fn send_message(&self, label: &str, data: &[u8]) -> Result<()> {
        self.handle.send_message(label, data).await
    }

    pub async fn get_stats(&self) -> Result<JsonString> {
        self.handle.get_stats().await
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.handle.disconnect().await
    }

    /// 切断要求を送信し、`run_task` の終了まで待機する。
    pub async fn disconnect_and_wait(&mut self, timeout: Duration) -> Result<()> {
        match self.disconnect().await {
            Ok(()) => self.wait_for_run_finished(timeout).await,
            Err(error) => {
                // disconnect() の失敗は run task の終了のみを意味するため、
                // 直接結果を読み出して真のエラーを優先表示する。
                self.store_run_task_error_message().await;
                match self.run_task_error_message.as_deref() {
                    Some(message) => Err(io::Error::other(message).into()),
                    None => Err(error),
                }
            }
        }
    }

    /// `run_task` の終了を待機する。
    ///
    /// `disconnect()` 後の後始末をテスト側で明示的に完了させるために使う。
    pub async fn wait_for_run_finished(&mut self, timeout: Duration) -> Result<()> {
        // タイムアウト内に `run_task` の終了を待機して結果を保持する。
        tokio::time::timeout(timeout, self.store_run_task_error_message())
            .await
            .map_err(|_| {
                io::Error::other("connection task がタイムアウト内に終了しませんでした")
            })?;
        // 保持済みの結果を返す。
        match self.run_task_error_message.as_deref() {
            Some(message) => Err(io::Error::other(message).into()),
            None => Ok(()),
        }
    }

    pub async fn events(&mut self) -> Vec<SoraTestEvent> {
        self.collect_pending_events();
        self.event_log.clone()
    }

    pub async fn has_event<P>(&mut self, predicate: P) -> bool
    where
        P: Fn(&SoraTestEvent) -> bool,
    {
        self.count_events(predicate).await > 0
    }

    pub async fn count_events<P>(&mut self, predicate: P) -> usize
    where
        P: Fn(&SoraTestEvent) -> bool,
    {
        // 待機前に受信済みイベントを必ずログへ反映する。
        self.collect_pending_events();
        self.event_log
            .iter()
            .filter(|event| predicate(event))
            .count()
    }

    pub async fn wait_for_event<P>(&mut self, predicate: P, timeout: Duration) -> Result<()>
    where
        P: Fn(&SoraTestEvent) -> bool,
    {
        // まず過去ログを確認し、既に条件を満たしていれば即座に成功させる。
        self.collect_pending_events();
        if self.event_log.iter().any(&predicate) {
            return Ok(());
        }

        // 過去ログで見つからない場合のみ、受信待機しながらログを増やして判定する。
        let deadline = Instant::now() + timeout;
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return self.timeout_or_run_task_error().await;
            };

            let maybe_event = tokio::time::timeout(remaining, self.event_rx.recv())
                .await
                .ok()
                .flatten();

            let Some(event) = maybe_event else {
                // チャネルクローズは run task の終了を意味するため、結果を直接読み出す。
                self.store_run_task_error_message().await;
                return self.timeout_or_run_task_error().await;
            };

            let matched = predicate(&event);
            self.event_log.push(event);
            if matched {
                return Ok(());
            }
        }
    }

    pub async fn wait_for_notify<P>(&mut self, predicate: P, timeout: Duration) -> Result<()>
    where
        P: Fn(&str) -> bool,
    {
        self.wait_for_event(
            |event| match event {
                SoraTestEvent::Notify { message } => predicate(message),
                _ => false,
            },
            timeout,
        )
        .await
    }

    pub async fn wait_for_switched(&mut self, timeout: Duration) -> Result<()> {
        self.wait_for_event(|event| matches!(event, SoraTestEvent::Switched), timeout)
            .await
    }

    pub async fn wait_for_connect(&mut self, timeout: Duration) -> Result<()> {
        self.wait_for_notify(|_| true, timeout).await
    }

    pub async fn wait_for_signaling_message<P>(
        &mut self,
        predicate: P,
        timeout: Duration,
    ) -> Result<()>
    where
        P: Fn(SignalingType, SignalingDirection, &str) -> bool,
    {
        self.wait_for_event(
            |event| match event {
                SoraTestEvent::SignalingMessage {
                    signaling_type,
                    direction,
                    text,
                } => predicate(*signaling_type, *direction, text),
                _ => false,
            },
            timeout,
        )
        .await
    }

    pub async fn count_signaling_message<P>(&mut self, predicate: P) -> usize
    where
        P: Fn(SignalingType, SignalingDirection, &str) -> bool,
    {
        self.count_events(|event| match event {
            SoraTestEvent::SignalingMessage {
                signaling_type,
                direction,
                text,
            } => predicate(*signaling_type, *direction, text),
            _ => false,
        })
        .await
    }

    pub async fn wait_for_track_kind(
        &mut self,
        expected_kind: &str,
        timeout: Duration,
    ) -> Result<()> {
        self.wait_for_event(
            |event| match event {
                SoraTestEvent::Track { kind } => kind.as_deref() == Some(expected_kind),
                _ => false,
            },
            timeout,
        )
        .await
    }

    pub async fn wait_for_video_track(&mut self, timeout: Duration) -> Result<()> {
        self.wait_for_track_kind("video", timeout).await
    }

    pub async fn wait_for_message<P>(&mut self, predicate: P, timeout: Duration) -> Result<()>
    where
        P: Fn(&str, &[u8]) -> bool,
    {
        self.wait_for_event(
            |event| match event {
                SoraTestEvent::Message { label, data } => predicate(label, data),
                _ => false,
            },
            timeout,
        )
        .await
    }

    pub async fn count_message<P>(&mut self, predicate: P) -> usize
    where
        P: Fn(&str, &[u8]) -> bool,
    {
        self.count_events(|event| match event {
            SoraTestEvent::Message { label, data } => predicate(label, data),
            _ => false,
        })
        .await
    }

    pub async fn wait_for_data_channel<P>(&mut self, predicate: P, timeout: Duration) -> Result<()>
    where
        P: Fn(&str) -> bool,
    {
        self.wait_for_event(
            |event| match event {
                SoraTestEvent::DataChannel { label } => predicate(label),
                _ => false,
            },
            timeout,
        )
        .await
    }

    pub async fn wait_for_data_channel_open<P>(
        &mut self,
        predicate: P,
        timeout: Duration,
    ) -> Result<()>
    where
        P: Fn(&str) -> bool,
    {
        self.wait_for_event(
            |event| match event {
                SoraTestEvent::DataChannelOpen { label } => predicate(label),
                _ => false,
            },
            timeout,
        )
        .await
    }

    pub async fn wait_for_data_channel_close<P>(
        &mut self,
        predicate: P,
        timeout: Duration,
    ) -> Result<()>
    where
        P: Fn(&str) -> bool,
    {
        self.wait_for_event(
            |event| match event {
                SoraTestEvent::DataChannelClose { label } => predicate(label),
                _ => false,
            },
            timeout,
        )
        .await
    }

    pub async fn count_data_channel_open<P>(&mut self, predicate: P) -> usize
    where
        P: Fn(&str) -> bool,
    {
        self.count_events(|event| match event {
            SoraTestEvent::DataChannelOpen { label } => predicate(label),
            _ => false,
        })
        .await
    }

    pub async fn count_data_channel_close<P>(&mut self, predicate: P) -> usize
    where
        P: Fn(&str) -> bool,
    {
        self.count_events(|event| match event {
            SoraTestEvent::DataChannelClose { label } => predicate(label),
            _ => false,
        })
        .await
    }

    pub async fn wait_stats<P>(&self, predicate: P, timeout: Duration) -> Result<()>
    where
        P: Fn(&JsonString) -> bool,
    {
        // stats は受動 callback ではないため、期限まで能動ポーリングする。
        let deadline = Instant::now() + timeout;
        loop {
            let stats = self.get_stats().await?;
            if predicate(&stats) {
                return Ok(());
            }

            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Self::timeout_error();
            };
            if remaining.is_zero() {
                return Self::timeout_error();
            }

            tokio::time::sleep(STATS_POLL_INTERVAL.min(remaining)).await;
        }
    }

    pub async fn wait_video_outbound_packets_sent(&self, timeout: Duration) -> Result<()> {
        self.wait_stats(
            |stats| crate::verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent"),
            timeout,
        )
        .await
    }

    pub async fn wait_video_inbound_packets_received(&self, timeout: Duration) -> Result<()> {
        self.wait_stats(
            |stats| {
                crate::verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && crate::verify_video_stats_field_positive(
                        stats,
                        "inbound-rtp",
                        "framesDecoded",
                    )
            },
            timeout,
        )
        .await
    }

    /// channel に溜まったイベントをログへ取り込む。
    fn collect_pending_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.event_log.push(event);
        }
    }

    /// 待機 API 共通のタイムアウトエラー生成。
    fn timeout_error() -> Result<()> {
        Err(io::Error::new(io::ErrorKind::TimedOut, "タイムアウトしました").into())
    }

    /// `run_task` の結果を読み出して、エラーメッセージを `self` に保持する。
    ///
    /// 初回読み出し時だけ実際に `run_task` の終了を待って結果を保持し、2 回目以降は
    /// 何もしない。`Err` または panic で終了した場合は、そのエラーメッセージを
    /// `run_task_error_message` に保持する。
    async fn store_run_task_error_message(&mut self) {
        if self.run_task_joined {
            return;
        }
        match (&mut self.run_task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.run_task_error_message = Some(error.to_string());
            }
            Err(join_error) => {
                self.run_task_error_message =
                    Some(format!("connection task が panic しました: {join_error}"));
            }
        }
        self.run_task_joined = true;
    }

    /// `run_task` が終了済みの場合に、エラーメッセージを読み出して返す。
    ///
    /// `run_task` が終了済みなら結果を読み出してからメッセージを返し、
    /// 未終了なら `None` を返す。終了済みでも `Ok(())` の場合は `None` を返す。
    async fn read_run_task_error_message(&mut self) -> Option<String> {
        if self.run_task.is_finished() {
            self.store_run_task_error_message().await;
        }
        self.run_task_error_message.clone()
    }

    /// タイムアウトエラーを返す。
    ///
    /// `run_task` が真のエラーを保持している場合は、タイムアウトであることを示しつつ
    /// そのエラー内容を併記して返す。保持していない場合は従来どおりの
    /// タイムアウトエラーを返す。
    async fn timeout_or_run_task_error(&mut self) -> Result<()> {
        if let Some(message) = self.read_run_task_error_message().await {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("タイムアウトしました (run_task のエラー: {message})"),
            )
            .into());
        }
        Self::timeout_error()
    }
}

/// recvonly で DataChannel シグナリングと `ignore_disconnect_websocket` を有効にした接続を構築する。
///
/// 切断時に signaling DataChannel 経由の後始末を検証するテストで共通利用する。
/// クローズ待機 (`disconnect_wait_timeout`) と WebSocket close handshake
/// (`websocket_close_timeout`) は `Some` で指定する。`None` なら SDK の
/// デフォルト値が使われる。
pub fn build_recvonly_data_channel_signaling_connection(
    urls: Vec<String>,
    channel_id: String,
    disconnect_wait_timeout: Option<Duration>,
    websocket_close_timeout: Option<Duration>,
) -> SoraTestConnection {
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(true)
        .ignore_disconnect_websocket(true);
    if let Some(timeout) = disconnect_wait_timeout {
        builder = builder.disconnect_wait_timeout(timeout);
    }
    if let Some(timeout) = websocket_close_timeout {
        builder = builder.websocket_close_timeout(timeout);
    }
    if let Some(token) = crate::secret_key() {
        builder = builder.metadata(crate::build_metadata_with_access_token(&token));
    }
    builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました")
}
