use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls,
};
use sora_sdk::{Role, SoraConnectionContext};

/// テスト用のチャンネル ID を生成する (suffix 付き)
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

fn apply_optional_metadata(
    mut builder: e2e_tests::SoraTestConnectionBuilder,
) -> e2e_tests::SoraTestConnectionBuilder {
    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }
    builder
}

fn build_sendonly(
    urls: Vec<String>,
    channel_id: String,
) -> (SoraTestConnection, FakeVideoCapturer) {
    let context = SoraConnectionContext::new().expect("SendOnly コンテキスト作成失敗");
    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let builder = SoraTestConnection::builder(context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .data_channel_signaling(true);
    let connection = apply_optional_metadata(builder)
        .connect()
        .expect("SendOnly の SoraTestConnection 作成に失敗しました");
    (connection, capturer)
}

fn build_recvonly(urls: Vec<String>, channel_id: String) -> SoraTestConnection {
    let context = SoraConnectionContext::new().expect("RecvOnly コンテキスト作成失敗");
    let builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(true);
    apply_optional_metadata(builder)
        .connect()
        .expect("RecvOnly の SoraTestConnection 作成に失敗しました")
}

/// SendOnly を先に接続してから RecvOnly を接続するテスト
#[tokio::test]
async fn test_sendonly_then_recvonly() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("sendonly-first");

    let (mut sendonly, _capturer) = build_sendonly(urls.clone(), channel_id.clone());
    sendonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("SendOnly クライアントの接続がタイムアウトしました");

    let mut recvonly = build_recvonly(urls, channel_id);
    recvonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("RecvOnly クライアントの接続がタイムアウトしました");
    recvonly
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("RecvOnly クライアントがトラックを受信できませんでした");

    sendonly
        .wait_video_outbound_packets_sent(Duration::from_secs(10))
        .await
        .expect("SendOnly の outbound-rtp の packetsSent が 0 より大きくなりませんでした");
    recvonly
        .wait_video_inbound_packets_received(Duration::from_secs(10))
        .await
        .expect("RecvOnly の inbound-rtp の packetsReceived と framesDecoded が 0 より大きくなりませんでした");

    sendonly
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("SendOnly の disconnect に失敗しました");
    recvonly
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("RecvOnly の disconnect に失敗しました");
}

/// RecvOnly を先に接続してから SendOnly を接続するテスト (re-offer のテスト)
#[tokio::test]
async fn test_recvonly_then_sendonly() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("recvonly-first");

    let mut recvonly = build_recvonly(urls.clone(), channel_id.clone());
    recvonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("RecvOnly クライアントの接続がタイムアウトしました");

    let (mut sendonly, _capturer) = build_sendonly(urls, channel_id);
    sendonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("SendOnly クライアントの接続がタイムアウトしました");

    recvonly
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("RecvOnly クライアントが re-offer 経由でトラックを受信できませんでした");

    sendonly
        .wait_video_outbound_packets_sent(Duration::from_secs(10))
        .await
        .expect("SendOnly の outbound-rtp の packetsSent が 0 より大きくなりませんでした");
    recvonly
        .wait_video_inbound_packets_received(Duration::from_secs(10))
        .await
        .expect("RecvOnly の inbound-rtp の packetsReceived と framesDecoded が 0 より大きくなりませんでした");

    sendonly
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("SendOnly の disconnect に失敗しました");
    recvonly
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("RecvOnly の disconnect に失敗しました");
}
