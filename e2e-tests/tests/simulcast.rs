use std::sync::Arc;
use std::time::Duration;

use e2e_tests::stats::RtcSentRtpStreamStatsTrait;
use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, collect_video_outbound_rid_stats,
    count_active_simulcast_layers, generate_channel_id, has_simulcast_rids, load_env,
    openh264_path, secret_key, signaling_urls,
};
use serial_test::serial;
#[cfg(any(feature = "amf", feature = "vpl"))]
use shiguredo_webrtc::VideoCodecType;
#[cfg(feature = "amf")]
use sora_sdk::AmfVideoCodecCapability;
#[cfg(any(feature = "amf", feature = "vpl"))]
use sora_sdk::CodecDirection;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use sora_sdk::InternalAppleVideoCodecCapability;
#[cfg(feature = "nvcodec")]
use sora_sdk::NvCodecVideoCodecCapability;
#[cfg(feature = "v4l2")]
use sora_sdk::V4l2VideoCodecCapability;
#[cfg(feature = "vpl")]
use sora_sdk::VplVideoCodecCapability;
use sora_sdk::{
    AdmConfig, Openh264VideoCodecCapability, Role, SoraConnectionContext,
    SoraConnectionContextConfig, Video, VideoCodecCapability, VideoCodecPreference,
};

/// simulcast 3 レイヤー (720p) を安定して送るための映像ビットレート (kbps)。
///
/// ラボ既定の 500 kbps では最高レイヤー r2 が枯渇し、outbound stats 待ちがタイムアウトしやすい。
const SIMULCAST_VIDEO_BIT_RATE_KBPS: u32 = 2500;

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

fn has_basic_simulcast_outbound(stats: &sora_sdk::JsonString) -> bool {
    has_simulcast_rids(stats, &["r0", "r1", "r2"])
        && count_active_simulcast_layers(stats, 500, 5) >= 2
}

fn has_expected_simulcast_outbound(
    stats: &sora_sdk::JsonString,
    expected_encoder_impl_substrings: &[&str],
) -> bool {
    if expected_encoder_impl_substrings.is_empty() || !has_basic_simulcast_outbound(stats) {
        return false;
    }
    let rid_stats = collect_video_outbound_rid_stats(stats);
    for rid in ["r0", "r1", "r2"] {
        let Some(stat) = rid_stats
            .iter()
            .find(|stat| stat.rid.as_deref() == Some(rid))
        else {
            return false;
        };
        let bytes_sent = stat.bytes_sent().unwrap_or(0);
        let packets_sent = stat.packets_sent().unwrap_or(0);
        if bytes_sent <= 500 || packets_sent <= 5 {
            return false;
        }
        let Some(encoder_implementation) = &stat.encoder_implementation else {
            return false;
        };
        if expected_encoder_impl_substrings
            .iter()
            .any(|substring| !encoder_implementation.contains(substring))
        {
            return false;
        }
    }
    true
}

fn create_non_builtin_context(
    capability: Box<dyn VideoCodecCapability>,
) -> sora_sdk::Result<Arc<SoraConnectionContext>> {
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());
    let config = SoraConnectionContextConfig {
        adm_config: AdmConfig::default(),
        video_codec_preference: preference,
        video_codec_capabilities: vec![capability],
    };
    SoraConnectionContext::new_with_config(config)
}

async fn run_sendonly_simulcast_outbound_layers(
    context: Arc<SoraConnectionContext>,
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

    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .simulcast(true)
        .disconnect_wait_timeout(Duration::from_secs(1));
    if let Some(video) = video {
        builder = builder.video(video);
    }

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut connection = builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");
    connection
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("接続待機がタイムアウトしました");
    connection
        .wait_video_outbound_packets_sent(Duration::from_secs(10))
        .await
        .expect("outbound-rtp packetsSent が 0 より大きくなりませんでした");

    connection
        .wait_stats(
            |stats| has_expected_simulcast_outbound(stats, expected_encoder_impl_substrings),
            Duration::from_secs(15),
        )
        .await
        .expect("simulcast outbound stats が期待値に到達しませんでした");

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect の実行に失敗しました");
}

#[tokio::test]
#[serial]
async fn test_sendonly_simulcast_outbound_layers() {
    load_env();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    run_sendonly_simulcast_outbound_layers(
        context,
        Some(Video::new_vp8(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS))),
        "simulcast-sendonly",
        // libvpx は EncoderInfo::supports_simulcast == true であるため、
        // SimulcastEncoderAdapter を使用していてもバイパスモードになって単一のエンコーダーとして扱われる
        &["libvpx"],
    )
    .await;
}

#[tokio::test]
#[serial]
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
        Some(Video::new_h264(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None)),
        "simulcast-sendonly-openh264",
        &["SimulcastEncoderAdapter", "OpenH264"],
    )
    .await;
}

#[cfg(feature = "nvcodec")]
#[tokio::test]
#[serial]
async fn test_sendonly_simulcast_outbound_layers_nvcodec() {
    load_env();
    let capability =
        NvCodecVideoCodecCapability::new().expect("Failed to create NvCodecVideoCodecCapability");
    let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
    let context = create_non_builtin_context(capability).expect("コンテキスト作成失敗");
    run_sendonly_simulcast_outbound_layers(
        context,
        Some(Video::new_h264(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None)),
        "simulcast-sendonly-nvcodec",
        &["SimulcastEncoderAdapter", "NvCodec"],
    )
    .await;
}

#[cfg(feature = "v4l2")]
#[tokio::test]
#[serial]
async fn test_sendonly_simulcast_outbound_layers_v4l2() {
    load_env();
    let capability = match V4l2VideoCodecCapability::new() {
        Ok(capability) => capability,
        Err(err) => {
            eprintln!("V4L2 capability is not available, skipping simulcast test: {err}");
            return;
        }
    };

    let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
    let context = create_non_builtin_context(capability).expect("コンテキスト作成失敗");
    run_sendonly_simulcast_outbound_layers(
        context,
        Some(Video::new_h264(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None)),
        "simulcast-sendonly-v4l2",
        &["SimulcastEncoderAdapter", "V4L2"],
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
            VideoCodecType::H264 => (
                Video::new_h264(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None),
                "h264",
            ),
            VideoCodecType::H265 => (
                Video::new_h265(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None),
                "h265",
            ),
            VideoCodecType::Av1 => (
                Video::new_av1(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None),
                "av1",
            ),
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

#[cfg(feature = "vpl")]
fn vpl_simulcast_supported_codecs() -> Option<Vec<VideoCodecType>> {
    let capability = match VplVideoCodecCapability::new() {
        Ok(capability) => capability,
        Err(err) => {
            eprintln!("VPL capability is not available, skipping simulcast test: {err}");
            return None;
        }
    };

    let mut codec_types = Vec::new();
    for codec_type in [
        VideoCodecType::H264,
        VideoCodecType::H265,
        VideoCodecType::Vp9,
        VideoCodecType::Av1,
    ] {
        if capability.is_supported(CodecDirection::Encoder, codec_type) {
            codec_types.push(codec_type);
        }
    }
    if codec_types.is_empty() {
        eprintln!("VPL has no encoder support, skipping simulcast test");
        return None;
    }

    Some(codec_types)
}

#[cfg(feature = "vpl")]
#[tokio::test]
#[serial]
async fn test_sendonly_simulcast_outbound_layers_vpl() {
    load_env();
    let Some(codec_types) = vpl_simulcast_supported_codecs() else {
        return;
    };

    for codec_type in codec_types {
        let (video, codec_label) = match codec_type {
            VideoCodecType::H264 => (
                Video::new_h264(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None),
                "h264",
            ),
            VideoCodecType::H265 => (
                Video::new_h265(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None),
                "h265",
            ),
            VideoCodecType::Vp9 => (
                Video::new_vp9(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None),
                "vp9",
            ),
            VideoCodecType::Av1 => (
                Video::new_av1(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None),
                "av1",
            ),
            _ => continue,
        };

        let capability: Box<dyn VideoCodecCapability> = Box::new(
            VplVideoCodecCapability::new().expect("VplVideoCodecCapability の作成に失敗しました"),
        );
        let context = create_non_builtin_context(capability).expect("コンテキスト作成失敗");
        run_sendonly_simulcast_outbound_layers(
            context,
            Some(video),
            &format!("simulcast-sendonly-vpl-{codec_label}"),
            &["SimulcastEncoderAdapter", "VPL"],
        )
        .await;
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[tokio::test]
#[serial]
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
        Some(Video::new_h264(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS), None)),
        "simulcast-sendonly-internal-apple",
        &["SimulcastEncoderAdapter", "VideoToolbox"],
    )
    .await;
}

#[tokio::test]
#[serial]
async fn test_sendrecv_simulcast_persists_after_reoffer() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("simulcast-reoffer");

    // クライアント A: SendOnly + simulcast true
    let context_a = SoraConnectionContext::new().expect("クライアント A のコンテキスト作成失敗");
    let mut capturer_a = FakeVideoCapturer::new(simulcast_video_capturer_config())
        .expect("FakeVideoCapturer A 作成失敗");
    let (video_track_a, audio_track_a) =
        build_sender_tracks(&context_a, &mut capturer_a).expect("送信用トラック A 作成失敗");

    let mut builder_a =
        SoraTestConnection::builder(context_a, urls.clone(), channel_id.clone(), Role::SendOnly)
            .sender_video_track(video_track_a)
            .sender_audio_track(audio_track_a)
            .video(Video::new_vp8(Some(SIMULCAST_VIDEO_BIT_RATE_KBPS)))
            .simulcast(true)
            .disconnect_wait_timeout(Duration::from_secs(1));

    if let Some(token) = secret_key() {
        builder_a = builder_a.metadata(build_metadata_with_access_token(&token));
    }

    let mut client_a = builder_a
        .connect()
        .expect("SoraTestConnection A の作成に失敗しました");
    client_a
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント A の接続待機がタイムアウトしました");
    client_a
        .wait_video_outbound_packets_sent(Duration::from_secs(10))
        .await
        .expect("クライアント A の outbound-rtp packetsSent が 0 より大きくなりませんでした");

    client_a
        .wait_stats(has_basic_simulcast_outbound, Duration::from_secs(15))
        .await
        .expect("re-offer 前の simulcast stats が期待値に到達しませんでした");

    // クライアント B: RecvOnly (後から参加)
    let context_b = SoraConnectionContext::new().expect("クライアント B のコンテキスト作成失敗");
    let mut builder_b = SoraTestConnection::builder(context_b, urls, channel_id, Role::RecvOnly)
        .disconnect_wait_timeout(Duration::from_secs(1));

    if let Some(token) = secret_key() {
        builder_b = builder_b.metadata(build_metadata_with_access_token(&token));
    }

    let mut client_b = builder_b
        .connect()
        .expect("SoraTestConnection B の作成に失敗しました");
    client_b
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント B の接続待機がタイムアウトしました");
    client_b
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .expect("クライアント B の on_track 受信待機がタイムアウトしました");

    client_a
        .wait_stats(has_basic_simulcast_outbound, Duration::from_secs(15))
        .await
        .expect("re-offer 後の simulcast stats が期待値に到達しませんでした");

    client_b
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント B の disconnect に失敗しました");
    client_a
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント A の disconnect に失敗しました");
}
