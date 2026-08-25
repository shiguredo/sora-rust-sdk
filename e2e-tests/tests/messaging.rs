use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls,
};
use sora_sdk::{
    ConnectDataChannel, Role, SignalingDirection, SignalingType, SoraConnectionContext,
};

fn build_sendrecv_connection(
    urls: Vec<String>,
    channel_id: String,
    data_channels: Vec<ConnectDataChannel>,
) -> (SoraTestConnection, FakeVideoCapturer) {
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .data_channel_signaling(true)
        .data_channels(data_channels);

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let connection = builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");
    (connection, capturer)
}

/// data_channels に #messaging を指定して、2 クライアント間でメッセージを送受信するテスト。
///
/// DataChannel シグナリングへの切替は `switched` 受信と全設定チャンネルの Open を待つため、
/// 両クライアントで `#messaging` の Open を明示的に待ってからメッセージを送信する。
/// 2 クライアント目の参加に伴う re-offer / re-answer が DataChannel 経由で
/// やり取りされることも確認する。
#[tokio::test]
async fn test_messaging_sendrecv() {
    load_env();
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();

    let messaging_label = "#messaging";
    let data_channels = vec![ConnectDataChannel {
        label: messaging_label.to_string(),
        direction: "sendrecv".to_string(),
        ordered: Some(true),
        max_packet_life_time: None,
        max_retransmits: None,
        protocol: None,
        compress: None,
        header: None,
    }];

    let (mut client1, _capturer1) =
        build_sendrecv_connection(urls.clone(), channel_id.clone(), data_channels.clone());
    client1
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("クライアント 1 の switched 通知がタイムアウトしました");
    client1
        .wait_for_data_channel_open(|label| label == messaging_label, Duration::from_secs(15))
        .await
        .expect("クライアント 1 で #messaging の Open がタイムアウトしました");

    let (mut client2, _capturer2) = build_sendrecv_connection(urls, channel_id, data_channels);
    client2
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("クライアント 2 の switched 通知がタイムアウトしました");
    client2
        .wait_for_data_channel_open(|label| label == messaging_label, Duration::from_secs(15))
        .await
        .expect("クライアント 2 で #messaging の Open がタイムアウトしました");

    // 2 クライアント目の参加に伴い、1 クライアント目が re-offer を受信し
    // re-answer を返信する。切替成立後はどちらも DataChannel 経由で観測される。
    client1
        .wait_for_signaling_message(
            |signaling_type, direction, text| {
                signaling_type == SignalingType::DataChannel
                    && direction == SignalingDirection::Received
                    && text.contains("re-offer")
            },
            Duration::from_secs(15),
        )
        .await
        .expect("クライアント 1 が DataChannel 経由で re-offer を受信していません");
    client1
        .wait_for_signaling_message(
            |signaling_type, direction, text| {
                signaling_type == SignalingType::DataChannel
                    && direction == SignalingDirection::Sent
                    && text.contains("re-answer")
            },
            Duration::from_secs(15),
        )
        .await
        .expect("クライアント 1 が DataChannel 経由で re-answer を送信していません");

    let message_from_1 = b"hello from client 1";
    let message_from_2 = b"hello from client 2";

    client1
        .send_message(messaging_label, message_from_1)
        .await
        .expect("クライアント 1 のメッセージ送信に失敗しました");
    client2
        .send_message(messaging_label, message_from_2)
        .await
        .expect("クライアント 2 のメッセージ送信に失敗しました");

    client1
        .wait_for_message(
            |label, data| label == messaging_label && data == message_from_2,
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 1 がクライアント 2 からのメッセージを受信していません");
    client2
        .wait_for_message(
            |label, data| label == messaging_label && data == message_from_1,
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 2 がクライアント 1 からのメッセージを受信していません");

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の disconnect に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の disconnect に失敗しました");
}
