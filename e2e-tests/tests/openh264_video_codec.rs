use std::io;
use std::sync::Arc;
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    openh264_path, secret_key, signaling_urls, verify_video_codec_mime_type,
    verify_video_stats_field_positive,
};
use serial_test::serial;
use sora_sdk::{
    Openh264VideoCodecCapability, Role, SoraConnectionContext, SoraConnectionContextConfig, Video,
    VideoCodecCapability, VideoCodecPreference,
};

/// テスト用のチャンネル ID を生成する (suffix 付き)
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

/// OpenH264 capability を設定した SoraConnectionContext を作成する。
fn create_openh264_context() -> sora_sdk::Result<Arc<SoraConnectionContext>> {
    let path = openh264_path().ok_or_else(|| io::Error::other("OPENH264_PATH is not set"))?;

    let capability: Box<dyn VideoCodecCapability> =
        Box::new(Openh264VideoCodecCapability::new(path)?);
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());

    let mut config = SoraConnectionContextConfig {
        video_codec_preference: preference,
        ..Default::default()
    };
    config.video_codec_capabilities.push(capability);

    SoraConnectionContext::new_with_config(config)
}

/// OpenH264 で SendOnly → RecvOnly の接続テストを実行する。
#[tokio::test]
#[serial]
async fn test_openh264_sendonly_recvonly() {
    load_env();
    if openh264_path().is_none() {
        eprintln!("OPENH264_PATH is not set, skipping test");
        return;
    }

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS is required");
    let channel_id = test_channel_id("openh264-sendonly-recvonly");

    let sendonly_context = create_openh264_context().expect("failed to create sendonly context");
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
    .video(Video::new_h264(None, None))
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

    let recvonly_context = create_openh264_context().expect("failed to create recvonly context");
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
                    && verify_video_codec_mime_type(stats, "outbound-rtp", "video/H264")
            },
            Duration::from_secs(15),
        )
        .await
        .expect("sendonly stats did not reach expected values");
    recvonly
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_codec_mime_type(stats, "inbound-rtp", "video/H264")
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

/// OpenH264 で SendRecv の双方向接続テストを実行する。
#[tokio::test]
#[serial]
async fn test_openh264_sendrecv() {
    load_env();
    if openh264_path().is_none() {
        eprintln!("OPENH264_PATH is not set, skipping test");
        return;
    }

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS is required");
    let channel_id = test_channel_id("openh264-sendrecv");

    let context1 = create_openh264_context().expect("failed to create client1 context");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("failed to create FakeVideoCapturer for client1");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("failed to build client1 tracks");

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .video(Video::new_h264(None, None))
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let mut client1 = builder1.connect().expect("failed to build client1");
    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("client1 connection timed out");

    let context2 = create_openh264_context().expect("failed to create client2 context");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("failed to create FakeVideoCapturer for client2");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("failed to build client2 tracks");

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .video(Video::new_h264(None, None))
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
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .expect("client1 did not receive tracks");
    client2
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .expect("client2 did not receive tracks");

    client1
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_codec_mime_type(stats, "outbound-rtp", "video/H264")
                    && verify_video_codec_mime_type(stats, "inbound-rtp", "video/H264")
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
                    && verify_video_codec_mime_type(stats, "outbound-rtp", "video/H264")
                    && verify_video_codec_mime_type(stats, "inbound-rtp", "video/H264")
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
