use std::sync::Arc;
use std::time::Duration;

use shiguredo_webrtc::{VideoTrackSource, rtc_log_info, rtc_log_warning};
use sora_sdk::{Role, SoraClient, SoraClientContext};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::stats::StatsEvent;

#[derive(Clone)]
pub(crate) struct VirtualClientConfig {
    pub(crate) signaling_urls: Vec<String>,
    pub(crate) channel_id: String,
    pub(crate) role: Role,
    pub(crate) duration: Option<f64>,
    pub(crate) repeat_interval: Option<f64>,
    pub(crate) max_retry: u32,
    pub(crate) retry_interval: f64,
    pub(crate) video: Option<sora_sdk::Video>,
    pub(crate) audio: Option<sora_sdk::Audio>,
    pub(crate) data_channel_signaling: Option<bool>,
    pub(crate) ignore_disconnect_websocket: Option<bool>,
}

enum DisconnectReason {
    Shutdown,
    DurationExpired,
    Unexpected(sora_sdk::Result<()>),
}

pub(crate) async fn run(
    id: u32,
    context: Arc<SoraClientContext>,
    video_source: Option<VideoTrackSource>,
    config: VirtualClientConfig,
    token: CancellationToken,
    stats_tx: mpsc::Sender<StatsEvent>,
) {
    let mut retry_count: u32 = 0;

    loop {
        let connection_token = token.child_token();

        let (client, handle) = match build_client(&context, &video_source, &config) {
            Ok(pair) => pair,
            Err(e) => {
                rtc_log_warning!("[vc-{}] クライアント構築に失敗: {}", id, e);
                retry_count += 1;
                if retry_count > config.max_retry {
                    rtc_log_info!(
                        "[vc-{}] 最大リトライ回数 ({}) に達しました",
                        id,
                        config.max_retry,
                    );
                    break;
                }
                let _ = stats_tx
                    .send(StatsEvent::Retrying { id, retry_count })
                    .await;
                tokio::select! {
                    biased;
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs_f64(config.retry_interval)) => continue,
                }
            }
        };
        let _ = stats_tx.send(StatsEvent::Connected { id }).await;
        rtc_log_info!("[vc-{}] 接続しました", id);

        let mut run_future = Box::pin(client.run());

        let reason = tokio::select! {
            biased;
            _ = token.cancelled() => DisconnectReason::Shutdown,
            _ = duration_timer(config.duration) => DisconnectReason::DurationExpired,
            result = &mut run_future => DisconnectReason::Unexpected(result),
        };

        match reason {
            DisconnectReason::Shutdown => {
                rtc_log_info!("[vc-{}] シャットダウンします", id);
                // run_future をポーリングしながら disconnect を送信する
                // (client.run() が disconnect コマンドを処理するため)
                tokio::select! {
                    _ = handle.disconnect() => {}
                    _ = &mut run_future => {}
                }
                break;
            }
            DisconnectReason::DurationExpired => {
                rtc_log_info!("[vc-{}] duration が経過しました", id);
                tokio::select! {
                    _ = handle.disconnect() => {}
                    _ = &mut run_future => {}
                }
                connection_token.cancel();
                let _ = stats_tx.send(StatsEvent::Disconnected { id }).await;
                retry_count = 0;

                match config.repeat_interval {
                    Some(interval) if interval > 0.0 => {
                        rtc_log_info!("[vc-{}] {:.1} 秒後に再接続します", id, interval);
                        tokio::select! {
                            biased;
                            _ = token.cancelled() => break,
                            _ = tokio::time::sleep(Duration::from_secs_f64(interval)) => continue,
                        }
                    }
                    _ => break,
                }
            }
            DisconnectReason::Unexpected(result) => {
                connection_token.cancel();
                let _ = stats_tx.send(StatsEvent::Disconnected { id }).await;
                if let Err(e) = result {
                    rtc_log_warning!("[vc-{}] 予期しない切断: {}", id, e);
                }
                retry_count += 1;
                if retry_count > config.max_retry {
                    rtc_log_info!(
                        "[vc-{}] 最大リトライ回数 ({}) に達しました",
                        id,
                        config.max_retry,
                    );
                    break;
                }
                let _ = stats_tx
                    .send(StatsEvent::Retrying { id, retry_count })
                    .await;
                rtc_log_info!(
                    "[vc-{}] {:.1} 秒後にリトライします ({}/{})",
                    id,
                    config.retry_interval,
                    retry_count,
                    config.max_retry,
                );
                tokio::select! {
                    biased;
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs_f64(config.retry_interval)) => continue,
                }
            }
        }
    }

    let _ = stats_tx.send(StatsEvent::Stopped { id }).await;
}

async fn duration_timer(duration: Option<f64>) {
    match duration {
        Some(d) if d > 0.0 => tokio::time::sleep(Duration::from_secs_f64(d)).await,
        _ => std::future::pending().await,
    }
}

fn build_client(
    context: &Arc<SoraClientContext>,
    video_source: &Option<VideoTrackSource>,
    config: &VirtualClientConfig,
) -> sora_sdk::Result<(sora_sdk::SoraClient, sora_sdk::SoraClientHandle)> {
    let mut builder = SoraClient::builder(
        context.clone(),
        config.signaling_urls.clone(),
        config.channel_id.clone(),
        config.role,
    )
    .on_notify(|_text| {})
    .on_push(|_text| {})
    .on_track(|_transceiver| {})
    .on_remove_track(|_receiver| {});

    if let Some(video) = &config.video {
        builder = builder.video(video.clone());
    }

    if let Some(audio) = &config.audio {
        builder = builder.audio(audio.clone());
    }

    if config.role.wants_send() {
        if let Some(source) = video_source {
            let video_track = context.create_video_track(source)?;
            builder = builder.sender_video_track(video_track);
        }
        let audio_source = context.create_audio_source()?;
        let audio_track = context.create_audio_track(&audio_source)?;
        builder = builder.sender_audio_track(audio_track);
    }

    if let Some(data_channel_signaling) = config.data_channel_signaling {
        builder = builder.data_channel_signaling(data_channel_signaling);
    }
    if let Some(ignore_disconnect_websocket) = config.ignore_disconnect_websocket {
        builder = builder.ignore_disconnect_websocket(ignore_disconnect_websocket);
    }

    builder.build()
}
