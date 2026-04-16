use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, build_metadata_with_access_token, generate_channel_id, load_env,
    secret_key, signaling_urls,
};
use sora_sdk::{Role, SoraConnectionContext};

#[tokio::test]
async fn test_recvonly_connect() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly);

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
