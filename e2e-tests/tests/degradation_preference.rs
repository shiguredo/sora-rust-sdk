use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, verify_video_stats_field_positive,
};
use shiguredo_webrtc::DegradationPreference;
use sora_sdk::{Role, SoraConnectionContext};

/// 映像を送信する role で degradation preference を指定しても、映像の送信が継続することを確認する。
///
/// degradation preference は SDP に現れないため、Sora とのネゴシエーションが成立して
/// 送信が継続することまでを確認する。video sender の RTP パラメータへの反映は
/// SDK の単体テストで確認する。
#[tokio::test]
async fn test_sendrecv_degradation_preference() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .degradation_preference(DegradationPreference::MaintainResolution);

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut connection = builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");
    connection
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("接続に失敗しました");

    // 品質制御を解像度優先にしても映像の送信そのものは継続する。
    connection
        .wait_stats(
            |stats| verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent"),
            Duration::from_secs(10),
        )
        .await
        .expect("映像の送信が継続しませんでした");

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}

/// 映像を送信しない role では degradation preference がスキップされ、接続できることを確認する。
///
/// recvonly は video sender を持たないため、値を設定しても何も起きずに接続が成立する。
#[tokio::test]
async fn test_recvonly_degradation_preference() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .degradation_preference(DegradationPreference::MaintainFramerateAndResolution);

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut connection = builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");
    connection
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("接続に失敗しました");
    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}
