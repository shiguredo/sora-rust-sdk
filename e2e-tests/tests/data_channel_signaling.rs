use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use e2e_tests::{
    build_metadata_with_access_token, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_data_channel_label,
};
use sora_sdk::{Role, SoraConnection, SoraConnectionContext};

#[tokio::test]
async fn test_recvonly_data_channel_signaling() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let switched_received = Arc::new(AtomicBool::new(false));
    let switched_received_clone = switched_received.clone();

    let mut builder = SoraConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(true)
        .on_switched(move || {
            println!("switched 通知を受信しました");
            switched_received_clone.store(true, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (connection, handle) = builder
        .build()
        .expect("SoraConnection の作成に失敗しました");

    // クライアントを起動
    let connection_task = tokio::spawn(async move {
        connection.run().await.expect("connection run failed");
    });

    // switched 通知を待つ (最大 15 秒)
    let switched_for_wait = switched_received.clone();
    let switched_wait = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if switched_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        switched_wait.is_ok(),
        "switched 通知の受信がタイムアウトしました"
    );

    // switched 通知が受信されたことを確認
    assert!(
        switched_received.load(Ordering::SeqCst),
        "switched 通知が受信されませんでした"
    );

    println!("switched 通知受信完了、統計情報取得まで待機します");

    // 統計情報が安定するまで待機
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 統計情報を取得
    println!("統計情報を取得します");
    let stats = handle.get_stats().await.expect("get_stats に失敗しました");

    // data-channel タイプのエントリで label が "signaling" であることを確認
    assert!(
        verify_data_channel_label(&stats, "signaling"),
        "data-channel タイプのエントリで label が \"signaling\" のものが見つかりませんでした"
    );

    println!("統計情報の検証成功 (data-channel label = \"signaling\")");

    // 切断
    handle
        .disconnect()
        .await
        .expect("disconnect に失敗しました");

    e2e_tests::wait_task_finished(connection_task, "connection_task").await;

    println!("テスト成功: DataChannel シグナリングの統計情報を確認しました");
}

#[tokio::test]
async fn test_data_channel_callbacks() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let expected_labels: HashSet<String> = ["signaling", "stats", "notify", "push", "rpc"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let registered_labels = Arc::new(Mutex::new(HashSet::<String>::new()));
    let opened_labels = Arc::new(Mutex::new(HashSet::<String>::new()));
    let closed_labels = Arc::new(Mutex::new(HashSet::<String>::new()));
    let switched_received = Arc::new(AtomicBool::new(false));

    let registered_labels_clone = registered_labels.clone();
    let opened_labels_clone = opened_labels.clone();
    let closed_labels_clone = closed_labels.clone();
    let switched_received_clone = switched_received.clone();

    let mut builder = SoraConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(true)
        .on_data_channel(move |label| {
            println!("on_data_channel: {}", label);
            registered_labels_clone
                .lock()
                .unwrap()
                .insert(label.to_string());
        })
        .on_data_channel_open(move |label| {
            println!("on_data_channel_open: {}", label);
            opened_labels_clone
                .lock()
                .unwrap()
                .insert(label.to_string());
        })
        .on_data_channel_close(move |label| {
            println!("on_data_channel_close: {}", label);
            closed_labels_clone
                .lock()
                .unwrap()
                .insert(label.to_string());
        })
        .on_switched(move || {
            switched_received_clone.store(true, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (connection, handle) = builder
        .build()
        .expect("SoraConnection の作成に失敗しました");

    let connection_task = tokio::spawn(async move {
        connection.run().await.expect("connection run failed");
    });

    // switched 通知を待つ (最大 15 秒)
    let switched_for_wait = switched_received.clone();
    let switched_wait = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if switched_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        switched_wait.is_ok(),
        "switched 通知の受信がタイムアウトしました"
    );

    // on_data_channel で全ラベルが登録されたことを確認
    let registered = registered_labels.lock().unwrap().clone();
    for label in &expected_labels {
        assert!(
            registered.contains(label),
            "on_data_channel に '{}' が来ませんでした (受信: {:?})",
            label,
            registered
        );
    }
    println!("on_data_channel 受信ラベル: {:?}", registered);

    // on_data_channel_open で全ラベルが開いたことを確認
    let opened = opened_labels.lock().unwrap().clone();
    for label in &expected_labels {
        assert!(
            opened.contains(label),
            "on_data_channel_open に '{}' が来ませんでした (受信: {:?})",
            label,
            opened
        );
    }
    println!("on_data_channel_open 受信ラベル: {:?}", opened);

    // 切断 (disconnect 内で DataChannel の close イベントが処理される)
    handle
        .disconnect()
        .await
        .expect("disconnect に失敗しました");

    e2e_tests::wait_task_finished(connection_task, "connection_task").await;

    // on_data_channel_close で全ラベルが閉じたことを確認
    let closed = closed_labels.lock().unwrap().clone();
    for label in &expected_labels {
        assert!(
            closed.contains(label),
            "on_data_channel_close に '{}' が来ませんでした (受信: {:?})",
            label,
            closed
        );
    }
    println!("on_data_channel_close 受信ラベル: {:?}", closed);

    println!("テスト成功: DataChannel コールバックが正しく呼ばれました");
}
