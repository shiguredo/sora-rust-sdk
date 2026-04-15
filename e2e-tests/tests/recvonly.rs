use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use e2e_tests::{
    build_metadata_with_access_token, generate_channel_id, load_env, secret_key, signaling_urls,
};
use sora_sdk::{Role, SoraConnection, SoraConnectionContext};

#[tokio::test]
async fn test_recvonly_connect() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let connected = Arc::new(AtomicBool::new(false));
    let connected_clone = connected.clone();

    let mut builder =
        SoraConnection::builder(context, urls, channel_id, Role::RecvOnly).on_notify(move |_| {
            connected_clone.store(true, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (connection, handle) = builder
        .build()
        .expect("SoraConnection の作成に失敗しました");

    let run_task = tokio::spawn(async move {
        connection.run().await.expect("connection run failed");
    });

    let connected_wait = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if connected.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(connected_wait.is_ok(), "接続に失敗しました");

    handle.disconnect().await.expect("disconnect failed");
    e2e_tests::wait_task_finished(run_task, "run_task").await;
}
