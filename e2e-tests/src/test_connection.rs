use std::io;
use std::sync::Arc;
use std::time::Duration;

use shiguredo_webrtc::{AudioTrack, VideoTrack};
use sora_sdk::{
    Audio, ConnectDataChannel, ForwardingFilter, JsonString, ProxyInfo, Result, Role,
    RpcRequestOptions, RpcResponse, SignalingDirection, SignalingType, SoraConnection,
    SoraConnectionBuilder, SoraConnectionContext, SoraConnectionHandle, Video,
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

/// `SoraConnectionBuilder` をテスト向けに包むビルダー。
///
/// production API とほぼ同じ設定項目を公開しつつ、`connect()` 時に
/// callback を `SoraTestEvent` へ集約する。
pub struct SoraTestConnectionBuilder {
    inner: SoraConnectionBuilder,
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
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let builder = self.with_callbacks(event_tx);
        let (connection, handle) = builder.build()?;
        let run_task = tokio::spawn(async move { connection.run().await });

        Ok(SoraTestConnection {
            handle,
            event_rx,
            event_log: Vec::new(),
            run_task,
        })
    }

    /// SDK callback を `SoraTestEvent` に変換して channel へ流す。
    ///
    /// callback 内では重い処理を行わず、イベント転送だけに限定する。
    fn with_callbacks(self, event_tx: UnboundedSender<SoraTestEvent>) -> SoraConnectionBuilder {
        self.inner
            .on_signaling_message({
                let event_tx = event_tx.clone();
                move |signaling_type, direction, text| {
                    let _ = event_tx.send(SoraTestEvent::SignalingMessage {
                        signaling_type,
                        direction,
                        text: text.to_string(),
                    });
                }
            })
            .on_notify({
                let event_tx = event_tx.clone();
                move |message| {
                    let _ = event_tx.send(SoraTestEvent::Notify {
                        message: message.to_string(),
                    });
                }
            })
            .on_push({
                let event_tx = event_tx.clone();
                move |message| {
                    let _ = event_tx.send(SoraTestEvent::Push {
                        message: message.to_string(),
                    });
                }
            })
            .on_track({
                let event_tx = event_tx.clone();
                move |transceiver| {
                    let kind = transceiver.receiver().track().kind().ok();
                    let _ = event_tx.send(SoraTestEvent::Track { kind });
                }
            })
            .on_remove_track({
                let event_tx = event_tx.clone();
                move |receiver| {
                    let kind = receiver.track().kind().ok();
                    let _ = event_tx.send(SoraTestEvent::RemoveTrack { kind });
                }
            })
            .on_switched({
                let event_tx = event_tx.clone();
                move || {
                    let _ = event_tx.send(SoraTestEvent::Switched);
                }
            })
            .on_websocket_close({
                let event_tx = event_tx.clone();
                move |code, reason| {
                    let _ = event_tx.send(SoraTestEvent::WebsocketClose {
                        code,
                        reason: reason.to_string(),
                    });
                }
            })
            .on_message({
                let event_tx = event_tx.clone();
                move |label, data| {
                    let _ = event_tx.send(SoraTestEvent::Message {
                        label: label.to_string(),
                        data: data.to_vec(),
                    });
                }
            })
            .on_data_channel({
                let event_tx = event_tx.clone();
                move |label| {
                    let _ = event_tx.send(SoraTestEvent::DataChannel {
                        label: label.to_string(),
                    });
                }
            })
            .on_data_channel_open({
                let event_tx = event_tx.clone();
                move |label| {
                    let _ = event_tx.send(SoraTestEvent::DataChannelOpen {
                        label: label.to_string(),
                    });
                }
            })
            .on_data_channel_message({
                let event_tx = event_tx.clone();
                move |label, data| {
                    let _ = event_tx.send(SoraTestEvent::DataChannelMessage {
                        label: label.to_string(),
                        data: data.to_vec(),
                    });
                }
            })
            .on_data_channel_close(move |label| {
                let _ = event_tx.send(SoraTestEvent::DataChannelClose {
                    label: label.to_string(),
                });
            })
    }
}

/// e2e テスト用の接続ラッパー。
///
/// - `event_rx` / `event_log` で callback 履歴を保持
/// - `wait_for_*` 系 API で過去ログ + 新規受信の両方を判定
/// - `run_task` の終了待機まで含めて接続ライフサイクルを管理
pub struct SoraTestConnection {
    handle: SoraConnectionHandle,
    event_rx: UnboundedReceiver<SoraTestEvent>,
    event_log: Vec<SoraTestEvent>,
    run_task: JoinHandle<Result<()>>,
}

impl SoraTestConnection {
    /// `SoraTestConnectionBuilder` を生成する。
    pub fn builder(
        context: Arc<SoraConnectionContext>,
        signaling_urls: Vec<String>,
        channel_id: String,
        role: Role,
    ) -> SoraTestConnectionBuilder {
        SoraTestConnectionBuilder {
            inner: SoraConnection::builder(context, signaling_urls, channel_id, role),
        }
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

    /// 切断要求を送信し、run task の終了まで待機する。
    pub async fn disconnect_and_wait(&mut self, timeout: Duration) -> Result<()> {
        self.disconnect().await?;
        self.wait_for_run_finished(timeout).await
    }

    /// run task の終了を待機する。
    ///
    /// `disconnect()` 後の後始末をテスト側で明示的に完了させるために使う。
    pub async fn wait_for_run_finished(&mut self, timeout: Duration) -> Result<()> {
        let joined = tokio::time::timeout(timeout, &mut self.run_task)
            .await
            .map_err(|_| {
                io::Error::other("connection task がタイムアウト内に終了しませんでした")
            })?;
        joined.map_err(|source| {
            io::Error::other(format!("connection task が panic しました: {source}"))
        })?
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
                return Self::timeout_error();
            };

            let maybe_event = tokio::time::timeout(remaining, self.event_rx.recv())
                .await
                .ok()
                .flatten();

            let Some(event) = maybe_event else {
                return Self::timeout_error();
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

    pub async fn wait_for_connect(&mut self, timeout: Duration) -> Result<()> {
        self.wait_for_notify(|_| true, timeout).await
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
}
