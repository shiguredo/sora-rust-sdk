mod args;
mod error;
mod fake_video_capturer;
mod stats;
mod virtual_client;

use std::time::Duration;

use shiguredo_webrtc::{log, rtc_log_info, rtc_log_warning};
use sora_sdk::{AdmConfig, SoraClientContext, SoraClientContextConfig};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::error::Result;
use crate::fake_video_capturer::{FakeVideoCapturer, FakeVideoCapturerConfig};
use crate::stats::StatsCollector;
use crate::virtual_client::VirtualClientConfig;

fn build_video(args: &args::Args) -> Option<sora_sdk::Video> {
    if args.no_video_device {
        return Some(sora_sdk::Video::new_bool(false));
    }
    match args.video_codec_type.as_deref() {
        Some("vp8") => Some(sora_sdk::Video::new_vp8(args.video_bit_rate)),
        Some("vp9") => Some(sora_sdk::Video::new_vp9(args.video_bit_rate, None)),
        Some("av1") => Some(sora_sdk::Video::new_av1(args.video_bit_rate, None)),
        Some("h264") => Some(sora_sdk::Video::new_h264(args.video_bit_rate, None)),
        Some("h265") => Some(sora_sdk::Video::new_h265(args.video_bit_rate, None)),
        _ => {
            if args.video_bit_rate.is_some() {
                Some(sora_sdk::Video::new_vp8(args.video_bit_rate))
            } else {
                None
            }
        }
    }
}

fn build_audio(args: &args::Args) -> Option<sora_sdk::Audio> {
    if args.no_audio_device || !args.audio {
        return Some(sora_sdk::Audio::new_bool(false));
    }
    match args.audio_codec_type.as_deref() {
        Some("opus") => Some(sora_sdk::Audio::new_opus(args.audio_bit_rate, None)),
        _ => {
            if args.audio_bit_rate.is_some() {
                Some(sora_sdk::Audio::new_opus(args.audio_bit_rate, None))
            } else {
                None
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    log::log_to_debug(log::Severity::Info);
    log::enable_timestamps();
    log::enable_threads();

    let args = args::parse_args()?;

    rtc_log_info!(
        "zakuro: vcs={} hatch_rate={} duration={:?} repeat_interval={:?}",
        args.vcs,
        args.vcs_hatch_rate,
        args.duration,
        args.repeat_interval,
    );

    let context = SoraClientContext::new_with_config(SoraClientContextConfig {
        adm_config: AdmConfig::NoAudioDevice,
        ..Default::default()
    })?;

    let token = CancellationToken::new();

    // FakeVideoCapturer（映像有効時のみ）
    let mut _capturer = None;
    let video_source = if !args.no_video_device && args.role.wants_send() {
        let config = FakeVideoCapturerConfig {
            width: args.resolution.0,
            height: args.resolution.1,
            fps: args.framerate as i32,
            sandstorm: args.sandstorm,
        };
        let mut capturer = FakeVideoCapturer::new(config)?;
        capturer.start()?;
        let source = capturer.video_source();
        _capturer = Some(capturer);
        Some(source)
    } else {
        None
    };

    let stats = StatsCollector::new(args.vcs, token.clone());
    let stats_tx = stats.event_tx();

    let vc_config = VirtualClientConfig {
        signaling_urls: args.signaling_urls.clone(),
        channel_id: args.channel_id.clone(),
        role: args.role,
        duration: args.duration,
        repeat_interval: args.repeat_interval,
        max_retry: args.max_retry,
        retry_interval: args.retry_interval,
        video: build_video(&args),
        audio: build_audio(&args),
        data_channel_signaling: args.data_channel_signaling,
        ignore_disconnect_websocket: args.ignore_disconnect_websocket,
    };

    let mut clients = JoinSet::new();

    // hatch rate 制御
    let hatch_start = tokio::time::Instant::now();
    let interval_per_client = Duration::from_secs_f64(1.0 / args.vcs_hatch_rate);

    for i in 0..args.vcs {
        if token.is_cancelled() {
            break;
        }

        // hatch タイミングまで待機（初回は即座に起動）
        if i > 0 {
            let target = hatch_start + interval_per_client * i;
            tokio::select! {
                biased;
                _ = token.cancelled() => break,
                _ = tokio::time::sleep_until(target) => {}
            }
        }

        rtc_log_info!("仮想クライアント {} を起動します", i);

        let child_token = token.child_token();
        clients.spawn(virtual_client::run(
            i,
            context.clone(),
            video_source.clone(),
            vc_config.clone(),
            child_token,
            stats_tx.clone(),
        ));
    }

    // main 側の stats_tx を drop して、全クライアント終了時に channel が閉じるようにする
    drop(stats_tx);

    // Ctrl+C で CancellationToken を発火
    let shutdown_token = token.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        rtc_log_info!("Ctrl+C を受信しました。シャットダウンします...");
        shutdown_token.cancel();
    });

    // 全仮想クライアントの完了を待機
    while let Some(result) = clients.join_next().await {
        if let Err(e) = result {
            rtc_log_warning!("仮想クライアントタスクがパニックしました: {}", e);
        }
    }

    // 統計タスク等を停止
    token.cancel();

    rtc_log_info!("zakuro: 全ての仮想クライアントが終了しました");

    Ok(())
}
