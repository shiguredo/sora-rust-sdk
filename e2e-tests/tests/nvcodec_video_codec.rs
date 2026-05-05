#![cfg(feature = "nvcodec")]

use std::sync::Arc;
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, verify_video_codec_mime_type, verify_video_stats_field_positive,
};
use serial_test::serial;
use shiguredo_webrtc::VideoCodecType;
use sora_sdk::{
    CodecDirection, NvCodecVideoCodecCapability, Role, SoraConnectionContext,
    SoraConnectionContextConfig, Video, VideoCodecCapability, VideoCodecPreference,
};

fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

fn codec_label(codec_type: VideoCodecType) -> &'static str {
    match codec_type {
        VideoCodecType::H264 => "h264",
        VideoCodecType::H265 => "h265",
        VideoCodecType::Av1 => "av1",
        VideoCodecType::Vp8 => "vp8",
        VideoCodecType::Vp9 => "vp9",
        _ => "unknown",
    }
}

fn codec_mime_type(codec_type: VideoCodecType) -> &'static str {
    match codec_type {
        VideoCodecType::H264 => "video/H264",
        VideoCodecType::H265 => "video/H265",
        VideoCodecType::Av1 => "video/AV1",
        VideoCodecType::Vp8 => "video/VP8",
        VideoCodecType::Vp9 => "video/VP9",
        _ => "",
    }
}

fn video_setting(codec_type: VideoCodecType) -> Video {
    match codec_type {
        VideoCodecType::H264 => Video::new_h264(None, None),
        VideoCodecType::H265 => Video::new_h265(None, None),
        VideoCodecType::Av1 => Video::new_av1(None, None),
        VideoCodecType::Vp8 => Video::new_vp8(None),
        VideoCodecType::Vp9 => Video::new_vp9(None, None),
        _ => panic!("unsupported codec type"),
    }
}

fn create_nvcodec_context() -> sora_sdk::Result<Arc<SoraConnectionContext>> {
    let capability: Box<dyn VideoCodecCapability> = Box::new(NvCodecVideoCodecCapability::new()?);
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());

    let mut config = SoraConnectionContextConfig {
        video_codec_preference: preference,
        ..Default::default()
    };
    config.video_codec_capabilities.push(capability);

    SoraConnectionContext::new_with_config(config)
}

fn nvcodec_fully_supported_codecs() -> Option<Vec<VideoCodecType>> {
    let capability =
        NvCodecVideoCodecCapability::new().expect("Failed to create NvCodecVideoCodecCapability");
    let mut codecs = Vec::new();

    for codec_type in [
        VideoCodecType::H264,
        VideoCodecType::H265,
        VideoCodecType::Av1,
    ] {
        if capability.is_supported(CodecDirection::Encoder, codec_type)
            && capability.is_supported(CodecDirection::Decoder, codec_type)
        {
            codecs.push(codec_type);
        }
    }

    if codecs.is_empty() {
        eprintln!("NVCodec has no codec with both encoder and decoder support, skipping test");
        return None;
    }

    Some(codecs)
}

fn nvcodec_decoder_supported_only_codecs() -> Option<Vec<VideoCodecType>> {
    let capability =
        NvCodecVideoCodecCapability::new().expect("Failed to create NvCodecVideoCodecCapability");
    let mut codecs = Vec::new();

    for codec_type in [VideoCodecType::Vp8, VideoCodecType::Vp9] {
        if capability.is_supported(CodecDirection::Decoder, codec_type)
            && !capability.is_supported(CodecDirection::Encoder, codec_type)
        {
            codecs.push(codec_type);
        }
    }

    if codecs.is_empty() {
        eprintln!("NVCodec has no decoder-only codec in VP8/VP9, skipping test");
        return None;
    }

    Some(codecs)
}

fn default_context_supports_encoder(codec_type: VideoCodecType) -> bool {
    let config = SoraConnectionContextConfig::default();
    config
        .video_codec_capabilities
        .iter()
        .any(|capability| capability.is_supported(CodecDirection::Encoder, codec_type))
}

async fn run_sendonly_recvonly_with_contexts(
    sendonly_context: Arc<SoraConnectionContext>,
    recvonly_context: Arc<SoraConnectionContext>,
    codec_type: VideoCodecType,
    suffix_prefix: &str,
) -> std::result::Result<(), String> {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS is required");
    let codec_label = codec_label(codec_type);
    let expected_mime_type = codec_mime_type(codec_type);
    let channel_id = test_channel_id(&format!("{suffix_prefix}-{codec_label}-sendonly-recvonly"));

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .map_err(|e| format!("failed to create FakeVideoCapturer: {e}"))?;
    let (video_track, audio_track) = build_sender_tracks(&sendonly_context, &mut capturer)
        .map_err(|e| format!("failed to build tracks: {e}"))?;

    let mut sendonly_builder = SoraTestConnection::builder(
        sendonly_context,
        urls.clone(),
        channel_id.clone(),
        Role::SendOnly,
    )
    .sender_video_track(video_track)
    .sender_audio_track(audio_track)
    .video(video_setting(codec_type))
    .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        sendonly_builder = sendonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut sendonly = sendonly_builder
        .connect()
        .map_err(|e| format!("failed to build sendonly client: {e}"))?;

    if sendonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .is_err()
    {
        let _ = sendonly.disconnect_and_wait(Duration::from_secs(10)).await;
        return Err("sendonly connection timed out".to_string());
    }

    let mut recvonly_builder =
        SoraTestConnection::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut recvonly = recvonly_builder
        .connect()
        .map_err(|e| format!("failed to build recvonly client: {e}"))?;

    let result = async {
        if recvonly
            .wait_for_connect(Duration::from_secs(10))
            .await
            .is_err()
        {
            return Err("recvonly connection timed out".to_string());
        }

        if recvonly
            .wait_for_video_track(Duration::from_secs(10))
            .await
            .is_err()
        {
            return Err("recvonly did not receive video tracks".to_string());
        }

        sendonly
            .wait_stats(
                |stats| {
                    verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                        && verify_video_codec_mime_type(stats, "outbound-rtp", expected_mime_type)
                },
                Duration::from_secs(15),
            )
            .await
            .map_err(|_| "sendonly stats did not reach expected values".to_string())?;
        recvonly
            .wait_stats(
                |stats| {
                    verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
                        && verify_video_codec_mime_type(stats, "inbound-rtp", expected_mime_type)
                },
                Duration::from_secs(15),
            )
            .await
            .map_err(|_| "recvonly stats did not reach expected values".to_string())?;

        Ok(())
    }
    .await;

    let _ = sendonly.disconnect_and_wait(Duration::from_secs(10)).await;
    let _ = recvonly.disconnect_and_wait(Duration::from_secs(10)).await;
    result
}

async fn run_sendrecv_with_codec(codec_type: VideoCodecType) {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS is required");
    let codec_label = codec_label(codec_type);
    let expected_mime_type = codec_mime_type(codec_type);
    let channel_id = test_channel_id(&format!("nvcodec-{codec_label}-sendrecv"));

    let context1 = create_nvcodec_context().expect("failed to create client1 context");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("failed to create FakeVideoCapturer for client1");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("failed to build client1 tracks");

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .video(video_setting(codec_type))
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let mut client1 = builder1.connect().expect("failed to build client1");

    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("client1 connection timed out");

    let context2 = create_nvcodec_context().expect("failed to create client2 context");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("failed to create FakeVideoCapturer for client2");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("failed to build client2 tracks");

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .video(video_setting(codec_type))
        .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let mut client2 = builder2.connect().expect("failed to build client2");

    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("client2 connection timed out");
    client1
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("client1 did not receive tracks");
    client2
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("client2 did not receive tracks");

    client1
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
                    && verify_video_codec_mime_type(stats, "outbound-rtp", expected_mime_type)
                    && verify_video_codec_mime_type(stats, "inbound-rtp", expected_mime_type)
            },
            Duration::from_secs(15),
        )
        .await
        .expect("client1 stats did not reach expected values");
    client2
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
                    && verify_video_codec_mime_type(stats, "outbound-rtp", expected_mime_type)
                    && verify_video_codec_mime_type(stats, "inbound-rtp", expected_mime_type)
            },
            Duration::from_secs(15),
        )
        .await
        .expect("client2 stats did not reach expected values");

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("failed to disconnect client1");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("failed to disconnect client2");
}

#[tokio::test]
#[serial]
async fn test_nvcodec_sendonly_recvonly() {
    load_env();
    let Some(codec_types) = nvcodec_fully_supported_codecs() else {
        return;
    };

    let mut failures = Vec::new();
    for codec_type in codec_types {
        let sendonly_context = create_nvcodec_context().expect("failed to create sendonly context");
        let recvonly_context = create_nvcodec_context().expect("failed to create recvonly context");
        match run_sendonly_recvonly_with_contexts(
            sendonly_context,
            recvonly_context,
            codec_type,
            "nvcodec",
        )
        .await
        {
            Ok(()) => {
                eprintln!(
                    "nvcodec sendonly/recvonly passed for {}",
                    codec_label(codec_type)
                );
            }
            Err(error) => failures.push(format!("{}: {error}", codec_label(codec_type))),
        }
    }

    assert!(
        failures.is_empty(),
        "nvcodec sendonly/recvonly failures:\n{}",
        failures.join("\n")
    );
}

#[tokio::test]
#[serial]
async fn test_nvcodec_sendrecv() {
    load_env();
    let Some(codec_types) = nvcodec_fully_supported_codecs() else {
        return;
    };

    for codec_type in codec_types {
        run_sendrecv_with_codec(codec_type).await;
    }
}

#[tokio::test]
#[serial]
async fn test_nvcodec_decoder_only_recvonly() {
    load_env();
    let Some(codec_types) = nvcodec_decoder_supported_only_codecs() else {
        return;
    };

    for codec_type in codec_types {
        if !default_context_supports_encoder(codec_type) {
            eprintln!(
                "default context does not support encoder for {}, skipping",
                codec_label(codec_type)
            );
            continue;
        }

        let sendonly_context =
            SoraConnectionContext::new().expect("failed to create default sendonly context");
        let recvonly_context = create_nvcodec_context().expect("failed to create recvonly context");
        run_sendonly_recvonly_with_contexts(
            sendonly_context,
            recvonly_context,
            codec_type,
            "nvcodec-decoder-only",
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "nvcodec decoder-only recvonly failed for {}: {error}",
                codec_label(codec_type)
            )
        });
    }
}
