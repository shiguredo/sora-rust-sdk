use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls,
};
use sora_sdk::{Role, SignalingDirection, SignalingType, SoraConnectionContext};

fn apply_optional_metadata(
    mut builder: e2e_tests::SoraTestConnectionBuilder,
) -> e2e_tests::SoraTestConnectionBuilder {
    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }
    builder
}

/// WebSocket 経由で on_signaling_message コールバックが Sent / Received 両方呼ばれることを確認する。
#[tokio::test]
async fn test_on_signaling_message_websocket() {
    load_env();
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(false);
    let mut connection = apply_optional_metadata(builder)
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");

    connection
        .wait_for_signaling_message(
            |signaling_type, direction, _| {
                signaling_type == SignalingType::WebSocket && direction == SignalingDirection::Sent
            },
            Duration::from_secs(10),
        )
        .await
        .expect("WebSocket Sent の on_signaling_message がタイムアウトしました");
    connection
        .wait_for_signaling_message(
            |signaling_type, direction, _| {
                signaling_type == SignalingType::WebSocket
                    && direction == SignalingDirection::Received
            },
            Duration::from_secs(10),
        )
        .await
        .expect("WebSocket Received の on_signaling_message がタイムアウトしました");

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("websocket の disconnect に失敗しました");

    let sent_count = connection
        .count_signaling_message(|signaling_type, direction, _| {
            signaling_type == SignalingType::WebSocket && direction == SignalingDirection::Sent
        })
        .await;
    let received_count = connection
        .count_signaling_message(|signaling_type, direction, _| {
            signaling_type == SignalingType::WebSocket && direction == SignalingDirection::Received
        })
        .await;

    assert!(
        sent_count > 0,
        "Sent 方向の on_signaling_message が呼ばれませんでした"
    );
    assert!(
        received_count > 0,
        "Received 方向の on_signaling_message が呼ばれませんでした"
    );
}

/// DataChannel シグナリング有効時に re-offer / re-answer で
/// DataChannel 経由の on_signaling_message が呼ばれることを確認する。
#[tokio::test]
async fn test_on_signaling_message_datachannel() {
    load_env();
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();

    let recv_context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let recv_builder = SoraTestConnection::builder(
        recv_context,
        urls.clone(),
        channel_id.clone(),
        Role::RecvOnly,
    )
    .data_channel_signaling(true);
    let mut recv_connection = apply_optional_metadata(recv_builder)
        .connect()
        .expect("recvonly の SoraTestConnection 作成に失敗しました");

    recv_connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("recvonly の switched 通知がタイムアウトしました");

    let send_context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&send_context, &mut capturer).expect("送信用トラック作成失敗");
    let send_builder = SoraTestConnection::builder(send_context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track);
    let mut send_connection = apply_optional_metadata(send_builder)
        .connect()
        .expect("sendonly の SoraTestConnection 作成に失敗しました");

    recv_connection
        .wait_for_signaling_message(
            |signaling_type, direction, _| {
                signaling_type == SignalingType::DataChannel
                    && direction == SignalingDirection::Received
            },
            Duration::from_secs(15),
        )
        .await
        .expect("DataChannel Received の on_signaling_message がタイムアウトしました");
    recv_connection
        .wait_for_signaling_message(
            |signaling_type, direction, _| {
                signaling_type == SignalingType::DataChannel
                    && direction == SignalingDirection::Sent
            },
            Duration::from_secs(15),
        )
        .await
        .expect("DataChannel Sent の on_signaling_message がタイムアウトしました");

    recv_connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("recvonly の disconnect に失敗しました");
    send_connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("sendonly の disconnect に失敗しました");
}
