use std::collections::HashSet;
use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, SoraTestEvent, build_metadata_with_access_token, generate_channel_id,
    load_env, secret_key, signaling_urls, verify_data_channel_label,
};
use sora_sdk::{Role, SoraConnectionContext};

fn build_recvonly_connection(
    urls: Vec<String>,
    channel_id: String,
    data_channel_signaling: bool,
) -> SoraTestConnection {
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(data_channel_signaling);
    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }
    builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました")
}

#[tokio::test]
async fn test_recvonly_data_channel_signaling() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection = build_recvonly_connection(urls, channel_id, true);

    connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("switched 通知の受信がタイムアウトしました");

    connection
        .wait_stats(
            |stats| verify_data_channel_label(stats, "signaling"),
            Duration::from_secs(10),
        )
        .await
        .expect(
            "data-channel タイプのエントリで label が \"signaling\" のものが見つかりませんでした",
        );

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}

#[tokio::test]
async fn test_data_channel_callbacks() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection = build_recvonly_connection(urls, channel_id, true);

    let expected_labels: HashSet<String> = ["signaling", "stats", "notify", "push", "rpc"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("switched 通知の受信がタイムアウトしました");

    for label in &expected_labels {
        let registered = connection
            .count_events(|event| match event {
                SoraTestEvent::DataChannel { label: actual } => actual == label,
                _ => false,
            })
            .await;
        assert!(
            registered > 0,
            "on_data_channel に '{}' が来ませんでした",
            label
        );

        let opened = connection
            .count_data_channel_open(|actual| actual == label)
            .await;
        assert!(
            opened > 0,
            "on_data_channel_open に '{}' が来ませんでした",
            label
        );
    }

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");

    for label in &expected_labels {
        let closed = connection
            .count_data_channel_close(|actual| actual == label)
            .await;
        assert!(
            closed > 0,
            "on_data_channel_close に '{}' が来ませんでした",
            label
        );
    }
}
