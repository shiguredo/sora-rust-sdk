use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use e2e_tests::{
    build_metadata_with_access_token, generate_access_token, generate_channel_id, load_env,
    secret_key, signaling_urls,
};
use sora_sdk::{Role, SignalingDirection, SignalingType, SoraClient, SoraClientContext};

/// redirect メッセージを受信して再接続できることを確認する。
///
/// 接続先ノードを URL で確定的に分離し、redirect を必ず発生させる。
///
/// 1. 1 つ目の recvonly を urls[0] に cluster_affinity: true で接続する
/// 2. 2 つ目の recvonly を urls[1] に cluster_affinity: true で接続する
/// 3. 2 つ目のクライアントが redirect メッセージを受信し、redirect: true で再接続する
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

    // --- 1 つ目の recvonly クライアント ---
    let first_context = SoraClientContext::new().expect("コンテキスト作成失敗");
    let first_connected = Arc::new(AtomicBool::new(false));
    let first_connected_clone = first_connected.clone();

    let first_access_token = generate_access_token(&channel_id, &secret, |f| {
        f.member("cluster_affinity", false)
    });

    let first_builder = SoraClient::builder(
        first_context,
        vec![urls[0].clone()],
        channel_id.clone(),
        Role::RecvOnly,
    )
    .metadata(build_metadata_with_access_token(&first_access_token))
    .on_notify(move |_| {
        first_connected_clone.store(true, Ordering::SeqCst);
    });

    let (first_client, first_handle) = first_builder
        .build()
        .expect("1 つ目の SoraClient の作成に失敗しました");

    let first_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), first_client.run()).await;
    });

    // 1 つ目の接続完了を待つ
    let first_connected_wait = first_connected.clone();
    let first_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if first_connected_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        first_wait.is_ok(),
        "1 つ目の recvonly の接続がタイムアウトしました"
    );

    // --- 2 つ目の recvonly クライアント (redirect を受ける側) ---
    let second_context = SoraClientContext::new().expect("コンテキスト作成失敗");

    let redirect_received = Arc::new(AtomicBool::new(false));
    let redirect_connect_sent = Arc::new(AtomicBool::new(false));
    let offer_received = Arc::new(AtomicBool::new(false));

    let redirect_received_clone = redirect_received.clone();
    let redirect_connect_sent_clone = redirect_connect_sent.clone();
    let offer_received_clone = offer_received.clone();

    let second_access_token =
        generate_access_token(&channel_id, &secret, |f| f.member("cluster_affinity", true));

    let second_builder = SoraClient::builder(
        second_context,
        vec![urls[1].clone()],
        channel_id.clone(),
        Role::RecvOnly,
    )
    .metadata(build_metadata_with_access_token(&second_access_token))
    .on_signaling_message(move |signaling_type, direction, text| {
        println!(
            "[2nd] on_signaling_message: {:?} {:?} {}",
            signaling_type,
            direction,
            &text[..text.len().min(200)]
        );

        if signaling_type == SignalingType::WebSocket {
            match direction {
                SignalingDirection::Received => {
                    if text.contains("\"type\":\"redirect\"")
                        || text.contains("\"type\": \"redirect\"")
                    {
                        redirect_received_clone.store(true, Ordering::SeqCst);
                    }
                    if text.contains("\"type\":\"offer\"") || text.contains("\"type\": \"offer\"") {
                        offer_received_clone.store(true, Ordering::SeqCst);
                    }
                }
                SignalingDirection::Sent => {
                    if (text.contains("\"type\":\"connect\"")
                        || text.contains("\"type\": \"connect\""))
                        && (text.contains("\"redirect\":true")
                            || text.contains("\"redirect\": true"))
                    {
                        redirect_connect_sent_clone.store(true, Ordering::SeqCst);
                    }
                }
            }
        }
    });

    let (second_client, second_handle) = second_builder
        .build()
        .expect("2 つ目の SoraClient の作成に失敗しました");

    let second_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(15), second_client.run()).await;
    });

    // 2 つ目のクライアントで redirect → 再接続 → offer の受信を待つ
    let redirect_for_wait = redirect_received.clone();
    let offer_for_wait = offer_received.clone();
    let redirect_connect_for_wait = redirect_connect_sent.clone();
    let wait_result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if redirect_for_wait.load(Ordering::SeqCst)
                && redirect_connect_for_wait.load(Ordering::SeqCst)
                && offer_for_wait.load(Ordering::SeqCst)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    // 切断
    let _ = second_handle.disconnect().await;
    second_task.abort();

    first_handle
        .disconnect()
        .await
        .expect("1 つ目の disconnect に失敗しました");
    first_task.abort();

    assert!(
        wait_result.is_ok(),
        "redirect が発生しませんでした (redirect_received={}, redirect_connect_sent={}, offer_received={})",
        redirect_received.load(Ordering::SeqCst),
        redirect_connect_sent.load(Ordering::SeqCst),
        offer_received.load(Ordering::SeqCst),
    );

    println!("テスト成功: redirect メッセージを受信し、redirect: true で再接続しました");
}
