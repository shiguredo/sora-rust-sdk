#![cfg(feature = "libcamera")]

use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, build_metadata_with_access_token, generate_channel_id, load_env,
    secret_key, signaling_urls, verify_video_stats_field_positive,
};
use serial_test::serial;
use sora_sdk::{LibcameraVideoCapturer, Role, SoraConnectionContext};

fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

#[serial]
#[tokio::test]
async fn test_sendonly_recvonly_with_libcamera() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("libcamera-sendonly-recvonly");

    let sendonly_context = SoraConnectionContext::new().expect("SendOnly コンテキスト作成失敗");
    let mut capturer = LibcameraVideoCapturer::builder()
        .width(640)
        .height(480)
        .build()
        .expect("libcamera capturer 作成失敗");
    capturer.start().expect("libcamera capturer 開始失敗");

    let video_track = sendonly_context
        .create_video_track(&capturer.video_source())
        .expect("映像トラック作成失敗");
    let audio_source = sendonly_context
        .create_audio_source()
        .expect("音声ソース作成失敗");
    let audio_track = sendonly_context
        .create_audio_track(&audio_source)
        .expect("音声トラック作成失敗");

    let mut sendonly_builder = SoraTestConnection::builder(
        sendonly_context,
        urls.clone(),
        channel_id.clone(),
        Role::SendOnly,
    )
    .sender_video_track(video_track)
    .sender_audio_track(audio_track)
    .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        sendonly_builder = sendonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut sendonly = sendonly_builder
        .connect()
        .expect("SendOnly クライアント作成失敗");

    let recvonly_context = SoraConnectionContext::new().expect("RecvOnly コンテキスト作成失敗");

    let mut recvonly_builder =
        SoraTestConnection::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut recvonly = recvonly_builder
        .connect()
        .expect("RecvOnly クライアント作成失敗");

    sendonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("sendonly connection timed out");
    recvonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("recvonly connection timed out");
    recvonly
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("recvonly did not receive video track");
    sendonly
        .wait_stats(
            |stats| verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent"),
            Duration::from_secs(15),
        )
        .await
        .expect("sendonly stats did not reach expected values within timeout");
    recvonly
        .wait_stats(
            |stats| verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived"),
            Duration::from_secs(15),
        )
        .await
        .expect("recvonly stats did not reach expected values within timeout");

    let _ = sendonly.disconnect_and_wait(Duration::from_secs(10)).await;
    let _ = recvonly.disconnect_and_wait(Duration::from_secs(10)).await;
    capturer.stop();
}
