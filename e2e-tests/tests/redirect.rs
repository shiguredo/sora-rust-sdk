use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, build_metadata_with_access_token, generate_access_token,
    generate_channel_id, load_env, secret_key, signaling_urls,
};
use sora_sdk::{Role, SignalingDirection, SignalingType, SoraConnectionContext};

/// redirect メッセージを受信して再接続できることを確認する。
///
/// signaling URL が 2 つ以上必要。
#[tokio::test]
async fn test_redirect() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    if urls.len() < 2 {
        eprintln!("test_redirect をスキップします: signaling URL が 2 つ以上必要です");
        return;
    }
    let Some(secret) = secret_key() else {
        eprintln!("test_redirect をスキップします: TEST_SECRET_KEY が未設定です");
        return;
    };
    let channel_id = generate_channel_id();

    let first_context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let first_access_token = generate_access_token(&channel_id, &secret, |f| {
        f.member("cluster_affinity", false)
    });
    let first_builder = SoraTestConnection::builder(
        first_context,
        vec![urls[0].clone()],
        channel_id.clone(),
        Role::RecvOnly,
    )
    .metadata(build_metadata_with_access_token(&first_access_token));
    let mut first_connection = first_builder
        .connect()
        .expect("1 つ目の SoraTestConnection の作成に失敗しました");
    first_connection
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("1 つ目の recvonly の接続がタイムアウトしました");

    let second_context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let second_access_token =
        generate_access_token(&channel_id, &secret, |f| f.member("cluster_affinity", true));
    let second_builder = SoraTestConnection::builder(
        second_context,
        vec![urls[1].clone()],
        channel_id.clone(),
        Role::RecvOnly,
    )
    .metadata(build_metadata_with_access_token(&second_access_token));
    let mut second_connection = second_builder
        .connect()
        .expect("2 つ目の SoraTestConnection の作成に失敗しました");

    second_connection
        .wait_for_signaling_message(
            |signaling_type, direction, text| {
                signaling_type == SignalingType::WebSocket
                    && direction == SignalingDirection::Received
                    && (text.contains("\"type\":\"redirect\"")
                        || text.contains("\"type\": \"redirect\""))
            },
            Duration::from_secs(10),
        )
        .await
        .expect("redirect メッセージを受信できませんでした");

    second_connection
        .wait_for_signaling_message(
            |signaling_type, direction, text| {
                signaling_type == SignalingType::WebSocket
                    && direction == SignalingDirection::Sent
                    && (text.contains("\"type\":\"connect\"")
                        || text.contains("\"type\": \"connect\""))
                    && (text.contains("\"redirect\":true") || text.contains("\"redirect\": true"))
            },
            Duration::from_secs(10),
        )
        .await
        .expect("redirect: true の connect 送信を確認できませんでした");

    second_connection
        .wait_for_signaling_message(
            |signaling_type, direction, text| {
                signaling_type == SignalingType::WebSocket
                    && direction == SignalingDirection::Received
                    && (text.contains("\"type\":\"offer\"") || text.contains("\"type\": \"offer\""))
            },
            Duration::from_secs(10),
        )
        .await
        .expect("redirect 後の offer を受信できませんでした");

    second_connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("2 つ目の disconnect に失敗しました");
    first_connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("1 つ目の disconnect に失敗しました");
}
