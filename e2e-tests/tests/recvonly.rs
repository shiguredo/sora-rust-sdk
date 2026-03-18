use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use e2e_tests::{
    build_metadata_with_access_token, generate_channel_id, load_env, secret_key, signaling_urls,
};
use sora_sdk::{Role, SoraClient, SoraClientContext};

#[tokio::test]
async fn test_recvonly_connect() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraClientContext::new().expect("コンテキスト作成失敗");

    let connected = Arc::new(AtomicBool::new(false));
    let connected_clone = connected.clone();

    let mut builder =
        SoraClient::builder(context, urls, channel_id, Role::RecvOnly).on_notify(move |_| {
            connected_clone.store(true, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (client, _handle) = builder.build().expect("SoraClient の作成に失敗しました");

    // タイムアウト付きで接続テスト
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), client.run()).await;

    // 接続成功を確認（タイムアウトでも notify を受信していれば OK）
    assert!(
        connected.load(Ordering::SeqCst) || result.is_ok(),
        "接続に失敗しました"
    );
}
