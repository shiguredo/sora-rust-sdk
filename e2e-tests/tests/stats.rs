use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, verify_stats_field_positive,
};
use sora_sdk::{Role, SoraConnectionContext};

#[tokio::test]
async fn test_get_stats() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track);

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
        .wait_stats(
            |stats| verify_stats_field_positive(stats, "outbound-rtp", "packetsSent"),
            Duration::from_secs(10),
        )
        .await
        .expect("outbound-rtp の packetsSent が 0 より大きくなりませんでした");

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}
