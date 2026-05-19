use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, verify_video_stats_field_positive,
};
use sora_sdk::{Role, SoraConnectionContext};

/// テスト用のチャンネル ID を生成する (suffix 付き)
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

/// 2 つの SendRecv クライアントが相互に接続して通信するテスト
#[tokio::test]
async fn test_sendrecv_bidirectional() {
    // シグナリング設定とテスト用チャネル ID を準備する。
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要です");
    let channel_id = test_channel_id("sendrecv-bidirectional");

    // 1 台目を接続
    let context1 =
        SoraConnectionContext::new().expect("クライアント 1 の context 作成に失敗しました");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 の作成に失敗しました");
    let (video_track1, audio_track1) = build_sender_tracks(&context1, &mut capturer1)
        .expect("クライアント 1 の送信用トラック作成に失敗しました");

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }
    let mut client1 = builder1
        .connect()
        .expect("SoraTestConnection 1 の作成に失敗しました");

    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の接続待機がタイムアウトしました");

    // 2 台目を接続
    let context2 =
        SoraConnectionContext::new().expect("クライアント 2 の context 作成に失敗しました");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 の作成に失敗しました");
    let (video_track2, audio_track2) = build_sender_tracks(&context2, &mut capturer2)
        .expect("クライアント 2 の送信用トラック作成に失敗しました");

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }
    let mut client2 = builder2
        .connect()
        .expect("SoraTestConnection 2 の作成に失敗しました");

    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の接続待機がタイムアウトしました");

    // 双方向で映像トラックが届くことを確認する。
    client1
        .wait_for_track_kind("video", Duration::from_secs(15))
        .await
        .expect("クライアント 1 が video トラックを受信できませんでした");
    client2
        .wait_for_track_kind("video", Duration::from_secs(15))
        .await
        .expect("クライアント 2 が video トラックを受信できませんでした");

    // stats をポーリングして、video の送受信パケットが双方で増加していることを検証する。
    client1
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
            },
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 1 の送受信 stats が条件を満たしませんでした");

    client2
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
            },
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 2 の送受信 stats が条件を満たしませんでした");

    // 最後に明示的に切断し、バックグラウンドの run タスク終了まで待つ。
    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の切断に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の切断に失敗しました");
}
