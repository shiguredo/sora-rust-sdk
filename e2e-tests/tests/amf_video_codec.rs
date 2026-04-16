#![cfg(feature = "amf")]

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
    AmfVideoCodecCapability, CodecDirection, Role, SoraConnectionContext,
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
        _ => "unknown",
    }
}

fn codec_mime_type(codec_type: VideoCodecType) -> &'static str {
    match codec_type {
        VideoCodecType::H264 => "video/H264",
        VideoCodecType::H265 => "video/H265",
        VideoCodecType::Av1 => "video/AV1",
        _ => "",
    }
}

fn video_setting(codec_type: VideoCodecType) -> Video {
    match codec_type {
        VideoCodecType::H264 => Video::new_h264(None, None),
        VideoCodecType::H265 => Video::new_h265(None, None),
        VideoCodecType::Av1 => Video::new_av1(None, None),
        _ => panic!("unsupported codec type"),
    }
}

fn create_amf_context() -> sora_sdk::Result<Arc<SoraConnectionContext>> {
    let capability: Box<dyn VideoCodecCapability> = Box::new(AmfVideoCodecCapability::new()?);
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());

    let mut config = SoraConnectionContextConfig {
        video_codec_preference: preference,
        ..Default::default()
    };
    config.video_codec_capabilities.push(capability);

    SoraConnectionContext::new_with_config(config)
}

fn amf_fully_supported_codecs() -> Option<Vec<VideoCodecType>> {
    let capability = match AmfVideoCodecCapability::new() {
        Ok(capability) => capability,
        Err(err) => {
            eprintln!("AMF capability is not available, skipping test: {err}");
            return None;
        }
    };

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
        eprintln!("AMF has no codec with both encoder and decoder support, skipping test");
        return None;
    }

    Some(codecs)
}

async fn run_sendonly_recvonly_with_codec(codec_type: VideoCodecType) {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS is required");
    let channel_id = test_channel_id(&format!(
        "amf-{}-sendonly-recvonly",
        codec_label(codec_type)
    ));
    let expected_mime_type = codec_mime_type(codec_type);

    let sendonly_context = create_amf_context().expect("failed to create sendonly context");
    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("failed to create FakeVideoCapturer");
    let (video_track, audio_track) =
        build_sender_tracks(&sendonly_context, &mut capturer).expect("failed to build tracks");

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
        .expect("failed to build sendonly client");

    sendonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("sendonly connection timed out");

    let recvonly_context = create_amf_context().expect("failed to create recvonly context");
    let mut recvonly_builder =
        SoraTestConnection::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut recvonly = recvonly_builder
        .connect()
        .expect("failed to build recvonly client");

    recvonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("recvonly connection timed out");
    recvonly
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("recvonly did not receive tracks");

    sendonly
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_codec_mime_type(stats, "outbound-rtp", expected_mime_type)
            },
            Duration::from_secs(15),
        )
        .await
        .expect("sendonly stats did not reach expected values");
    recvonly
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_codec_mime_type(stats, "inbound-rtp", expected_mime_type)
            },
            Duration::from_secs(15),
        )
        .await
        .expect("recvonly stats did not reach expected values");

    sendonly
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("failed to disconnect sendonly");
    recvonly
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("failed to disconnect recvonly");
}

async fn run_sendrecv_with_codec(codec_type: VideoCodecType) {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS is required");
    let channel_id = test_channel_id(&format!("amf-{}-sendrecv", codec_label(codec_type)));
    let expected_mime_type = codec_mime_type(codec_type);

    let context1 = create_amf_context().expect("failed to create client1 context");
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

    let context2 = create_amf_context().expect("failed to create client2 context");
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
async fn test_amf_sendonly_recvonly() {
    load_env();
    let Some(codec_types) = amf_fully_supported_codecs() else {
        return;
    };

    for codec_type in codec_types {
        run_sendonly_recvonly_with_codec(codec_type).await;
    }
}

#[tokio::test]
#[serial]
async fn test_amf_sendrecv() {
    load_env();
    let Some(codec_types) = amf_fully_supported_codecs() else {
        return;
    };

    for codec_type in codec_types {
        run_sendrecv_with_codec(codec_type).await;
    }
}
