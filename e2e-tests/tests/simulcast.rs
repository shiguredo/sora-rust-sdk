use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, collect_video_outbound_rid_stats, count_active_simulcast_layers,
    generate_channel_id, has_simulcast_rids, load_env, openh264_path, secret_key, signaling_urls,
};
#[cfg(feature = "amf")]
use serial_test::serial;
#[cfg(feature = "amf")]
use shiguredo_webrtc::VideoCodecType;
#[cfg(feature = "amf")]
use sora_sdk::AmfVideoCodecCapability;
#[cfg(feature = "amf")]
use sora_sdk::CodecDirection;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use sora_sdk::InternalAppleVideoCodecCapability;
#[cfg(feature = "nvcodec")]
use sora_sdk::NvCodecVideoCodecCapability;
use sora_sdk::{
    AdmConfig, Openh264VideoCodecCapability, Role, SoraClient, SoraClientContext,
    SoraClientContextConfig, Video, VideoCodecCapability, VideoCodecPreference,
};

fn test_channel_id(suffix: &str) -> String {
    format!("{}-{}", generate_channel_id(), suffix)
}

fn simulcast_video_capturer_config() -> FakeVideoCapturerConfig {
    FakeVideoCapturerConfig {
        width: 1280,
        height: 720,
        fps: 30,
    }
}

async fn wait_for_connected(flag: &Arc<AtomicBool>, timeout_secs: u64) {
    let flag_for_wait = flag.clone();
    let wait = tokio::time::timeout(Duration::from_secs(timeout_secs), async move {
        loop {
            if flag_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(wait.is_ok(), "接続待機がタイムアウトしました");
}

fn create_non_builtin_context(
    capability: Box<dyn VideoCodecCapability>,
) -> sora_sdk::Result<Arc<SoraClientContext>> {
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());
    let config = SoraClientContextConfig {
        adm_config: AdmConfig::default(),
        video_codec_preference: preference,
        video_codec_capabilities: vec![capability],
    };
    SoraClientContext::new_with_config(config)
}

async fn run_sendonly_simulcast_outbound_layers(
    context: Arc<SoraClientContext>,
    video: Option<Video>,
    channel_suffix: &str,
    expected_encoder_impl_substrings: &[&str],
) {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id(channel_suffix);

    let mut capturer = FakeVideoCapturer::new(simulcast_video_capturer_config())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let connected = Arc::new(AtomicBool::new(false));
    let connected_clone = connected.clone();

    let mut builder = SoraClient::builder(context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .simulcast(true)
        .on_notify(move |_| {
            connected_clone.store(true, Ordering::SeqCst);
        });
    if let Some(video) = video {
        builder = builder.video(video);
    }

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (client, handle) = builder.build().expect("SoraClient の作成に失敗しました");

    let run_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(40), client.run()).await;
    });

    wait_for_connected(&connected, 10).await;
    tokio::time::sleep(Duration::from_secs(8)).await;

    let stats = handle.get_stats().await.expect("get_stats に失敗しました");
    let rid_stats = collect_video_outbound_rid_stats(&stats);

    assert!(
        has_simulcast_rids(&stats, &["r0", "r1", "r2"]),
        "simulcast の rid (r0/r1/r2) が揃っていません"
    );
    assert!(
        count_active_simulcast_layers(&stats, 500, 5) >= 2,
        "有効な simulcast layer 数が不足しています"
    );
    assert!(
        !expected_encoder_impl_substrings.is_empty(),
        "expected_encoder_impl_substrings must contain at least one entry"
    );
    let mut sorted_rid_stats = rid_stats;
    sorted_rid_stats.sort_by(|a, b| a.rid.cmp(&b.rid));
    for (index, stat) in sorted_rid_stats.iter().enumerate() {
        assert_eq!(stat.rid, format!("r{index}"));
        assert!(
            stat.bytes_sent > 500 && stat.packets_sent > 5,
            "rid={} の送信量が不足しています: bytesSent={}, packetsSent={}",
            stat.rid,
            stat.bytes_sent,
            stat.packets_sent
        );
        let Some(encoder_implementation) = &stat.encoder_implementation else {
            panic!("rid={} に encoderImplementation がありません", stat.rid);
        };
        for expected_substring in expected_encoder_impl_substrings {
            assert!(
                encoder_implementation.contains(expected_substring),
                "rid={} の encoderImplementation が期待値を含みません: actual={}, expected_substring={}",
                stat.rid,
                encoder_implementation,
                expected_substring
            );
        }
    }

    handle
        .disconnect()
        .await
        .expect("disconnect の実行に失敗しました");
    run_task.abort();
}

#[tokio::test]
async fn test_sendonly_simulcast_outbound_layers() {
    load_env();
    let context = SoraClientContext::new().expect("コンテキスト作成失敗");
    run_sendonly_simulcast_outbound_layers(
        context,
        Some(Video::new_vp8(None)),
        "simulcast-sendonly",
        // libvpx は EncoderInfo::supports_simulcast == true であるため、
        // SimulcastEncoderAdapter を使用していてもバイパスモードになって単一のエンコーダーとして扱われる
        &["libvpx"],
    )
    .await;
}

#[tokio::test]
async fn test_sendonly_simulcast_outbound_layers_openh264() {
    load_env();
    let Some(path) = openh264_path() else {
        eprintln!("OPENH264_PATH is not set, skipping OpenH264 non-builtin simulcast failure test");
        return;
    };
    let capability: Box<dyn VideoCodecCapability> = Box::new(
        Openh264VideoCodecCapability::new(path)
            .expect("Openh264VideoCodecCapability の作成に失敗しました"),
    );
    let context = create_non_builtin_context(capability).expect("コンテキスト作成失敗");
    run_sendonly_simulcast_outbound_layers(
        context,
        Some(Video::new_h264(None, None)),
        "simulcast-sendonly-openh264",
        &["SimulcastEncoderAdapter", "OpenH264"],
    )
    .await;
}

#[cfg(feature = "nvcodec")]
#[tokio::test]
async fn test_sendonly_simulcast_outbound_layers_nvcodec() {
    load_env();
    let capability: Box<dyn VideoCodecCapability> = Box::new(NvCodecVideoCodecCapability::new());
    let context = create_non_builtin_context(capability).expect("コンテキスト作成失敗");
    run_sendonly_simulcast_outbound_layers(
        context,
        Some(Video::new_h264(None, None)),
        "simulcast-sendonly-nvcodec",
        &["SimulcastEncoderAdapter", "NvCodec"],
    )
    .await;
}

#[cfg(feature = "amf")]
fn amf_simulcast_supported_codecs() -> Option<Vec<VideoCodecType>> {
    let capability = match AmfVideoCodecCapability::new() {
        Ok(capability) => capability,
        Err(err) => {
            eprintln!("AMF capability is not available, skipping test: {err}");
            return None;
        }
    };

    let mut codec_types = Vec::new();
    for codec_type in [
        VideoCodecType::H264,
        VideoCodecType::H265,
        VideoCodecType::Av1,
    ] {
        if capability.is_supported(CodecDirection::Encoder, codec_type) {
            codec_types.push(codec_type);
        }
    }
    if codec_types.is_empty() {
        eprintln!("AMF has no encoder support, skipping simulcast test");
        return None;
    }

    Some(codec_types)
}

#[cfg(feature = "amf")]
#[tokio::test]
#[serial]
async fn test_sendonly_simulcast_outbound_layers_amf() {
    load_env();
    let Some(codec_types) = amf_simulcast_supported_codecs() else {
        return;
    };

    for codec_type in codec_types {
        let (video, codec_label) = match codec_type {
            VideoCodecType::H264 => (Video::new_h264(None, None), "h264"),
            VideoCodecType::H265 => (Video::new_h265(None, None), "h265"),
            VideoCodecType::Av1 => (Video::new_av1(None, None), "av1"),
            _ => continue,
        };

        let capability: Box<dyn VideoCodecCapability> = Box::new(
            AmfVideoCodecCapability::new().expect("AmfVideoCodecCapability の作成に失敗しました"),
        );
        let context = create_non_builtin_context(capability).expect("コンテキスト作成失敗");
        run_sendonly_simulcast_outbound_layers(
            context,
            Some(video),
            &format!("simulcast-sendonly-amf-{codec_label}"),
            &["SimulcastEncoderAdapter", "AMF"],
        )
        .await;
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[tokio::test]
async fn test_sendonly_simulcast_outbound_layers_internal_apple() {
    load_env();
    let Some(capability) = InternalAppleVideoCodecCapability::new() else {
        eprintln!("InternalAppleVideoCodecCapability is not available, skipping test");
        return;
    };
    let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
    let context = create_non_builtin_context(capability).expect("コンテキスト作成失敗");
    run_sendonly_simulcast_outbound_layers(
        context,
        Some(Video::new_h264(None, None)),
        "simulcast-sendonly-internal-apple",
        &["SimulcastEncoderAdapter", "VideoToolbox"],
    )
    .await;
}

#[tokio::test]
async fn test_sendrecv_simulcast_persists_after_reoffer() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("simulcast-reoffer");

    // クライアント A: SendOnly + simulcast true
    let context_a = SoraClientContext::new().expect("クライアント A のコンテキスト作成失敗");
    let mut capturer_a = FakeVideoCapturer::new(simulcast_video_capturer_config())
        .expect("FakeVideoCapturer A 作成失敗");
    let (video_track_a, audio_track_a) =
        build_sender_tracks(&context_a, &mut capturer_a).expect("送信用トラック A 作成失敗");

    let connected_a = Arc::new(AtomicBool::new(false));
    let connected_a_clone = connected_a.clone();

    let mut builder_a =
        SoraClient::builder(context_a, urls.clone(), channel_id.clone(), Role::SendOnly)
            .sender_video_track(video_track_a)
            .sender_audio_track(audio_track_a)
            .simulcast(true)
            .on_notify(move |_| {
                connected_a_clone.store(true, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        builder_a = builder_a.metadata(build_metadata_with_access_token(&token));
    }

    let (client_a, handle_a) = builder_a
        .build()
        .expect("SoraClient A の作成に失敗しました");
    let run_task_a = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(50), client_a.run()).await;
    });

    wait_for_connected(&connected_a, 10).await;
    tokio::time::sleep(Duration::from_secs(5)).await;

    let baseline_stats = handle_a
        .get_stats()
        .await
        .expect("クライアント A の baseline get_stats に失敗しました");
    assert!(
        has_simulcast_rids(&baseline_stats, &["r0", "r1", "r2"]),
        "re-offer 前に simulcast の rid (r0/r1/r2) が揃っていません"
    );
    assert!(
        count_active_simulcast_layers(&baseline_stats, 500, 5) >= 2,
        "re-offer 前の有効な simulcast layer 数が不足しています"
    );

    // クライアント B: RecvOnly (後から参加)
    let context_b = SoraClientContext::new().expect("クライアント B のコンテキスト作成失敗");
    let connected_b = Arc::new(AtomicBool::new(false));
    let connected_b_clone = connected_b.clone();
    let track_received_b = Arc::new(AtomicUsize::new(0));
    let track_received_b_clone = track_received_b.clone();

    let mut builder_b = SoraClient::builder(context_b, urls, channel_id, Role::RecvOnly)
        .on_notify(move |_| {
            connected_b_clone.store(true, Ordering::SeqCst);
        })
        .on_track(move |_track| {
            track_received_b_clone.fetch_add(1, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder_b = builder_b.metadata(build_metadata_with_access_token(&token));
    }

    let (client_b, handle_b) = builder_b
        .build()
        .expect("SoraClient B の作成に失敗しました");
    let run_task_b = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(50), client_b.run()).await;
    });

    wait_for_connected(&connected_b, 10).await;

    let track_received_for_wait = track_received_b.clone();
    let track_wait = tokio::time::timeout(Duration::from_secs(15), async move {
        loop {
            if track_received_for_wait.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(
        track_wait.is_ok(),
        "クライアント B の on_track 受信待機がタイムアウトしました"
    );

    tokio::time::sleep(Duration::from_secs(8)).await;

    let post_stats = handle_a
        .get_stats()
        .await
        .expect("クライアント A の re-offer 後 get_stats に失敗しました");
    assert!(
        has_simulcast_rids(&post_stats, &["r0", "r1", "r2"]),
        "re-offer 後に simulcast の rid (r0/r1/r2) が揃っていません"
    );
    assert!(
        count_active_simulcast_layers(&post_stats, 500, 5) >= 2,
        "re-offer 後の有効な simulcast layer 数が不足しています"
    );

    handle_b
        .disconnect()
        .await
        .expect("クライアント B の disconnect に失敗しました");
    handle_a
        .disconnect()
        .await
        .expect("クライアント A の disconnect に失敗しました");

    run_task_b.abort();
    run_task_a.abort();
}
