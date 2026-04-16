use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, generate_channel_id, load_env, secret_key, signaling_urls,
};
use sora_sdk::{ConnectDataChannel, Role, SoraConnection, SoraConnectionContext};

/// data_channels に #messaging を指定して、2 クライアント間でメッセージを送受信するテスト
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

    // クライアント 1
    let client1_switched = Arc::new(AtomicBool::new(false));
    let client1_switched_clone = client1_switched.clone();
    let client1_received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let client1_received_clone = client1_received.clone();

    let context1 = SoraConnectionContext::new().expect("クライアント 1 コンテキスト作成失敗");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 作成失敗");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("送信用トラック 1 作成失敗");

    let mut builder1 =
        SoraConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .data_channel_signaling(true)
            .data_channels(data_channels.clone())
            .on_switched(move || {
                client1_switched_clone.store(true, Ordering::SeqCst);
            })
            .on_message(move |label, data| {
                if label == messaging_label {
                    println!("クライアント 1: メッセージ受信 ({} bytes)", data.len());
                    client1_received_clone.lock().unwrap().push(data.to_vec());
                }
            });

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let (client1, handle1) = builder1
        .build()
        .expect("SoraConnection 1 の作成に失敗しました");

    let client1_task = tokio::spawn(async move {
        client1.run().await.expect("client1 run failed");
    });

    // クライアント 1 の switched を待つ
    let client1_switched_wait = client1_switched.clone();
    let wait1 = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if client1_switched_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(
        wait1.is_ok(),
        "クライアント 1 の switched 通知がタイムアウトしました"
    );
    println!("クライアント 1 switched 受信完了");

    tokio::time::sleep(Duration::from_secs(1)).await;

    // クライアント 2
    let client2_switched = Arc::new(AtomicBool::new(false));
    let client2_switched_clone = client2_switched.clone();
    let client2_received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let client2_received_clone = client2_received.clone();

    let context2 = SoraConnectionContext::new().expect("クライアント 2 コンテキスト作成失敗");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 作成失敗");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("送信用トラック 2 作成失敗");

    let mut builder2 = SoraConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .data_channel_signaling(true)
        .data_channels(data_channels)
        .on_switched(move || {
            client2_switched_clone.store(true, Ordering::SeqCst);
        })
        .on_message(move |label, data| {
            if label == messaging_label {
                println!("クライアント 2: メッセージ受信 ({} bytes)", data.len());
                client2_received_clone.lock().unwrap().push(data.to_vec());
            }
        });

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let (client2, handle2) = builder2
        .build()
        .expect("SoraConnection 2 の作成に失敗しました");

    let client2_task = tokio::spawn(async move {
        client2.run().await.expect("client2 run failed");
    });

    // クライアント 2 の switched を待つ
    let client2_switched_wait = client2_switched.clone();
    let wait2 = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if client2_switched_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(
        wait2.is_ok(),
        "クライアント 2 の switched 通知がタイムアウトしました"
    );
    println!("クライアント 2 switched 受信完了");

    // メッセージ送信
    let message_from_1 = b"hello from client 1";
    let message_from_2 = b"hello from client 2";

    handle1
        .send_message(messaging_label, message_from_1)
        .await
        .expect("クライアント 1 のメッセージ送信に失敗しました");
    println!("クライアント 1 からメッセージ送信完了");

    handle2
        .send_message(messaging_label, message_from_2)
        .await
        .expect("クライアント 2 のメッセージ送信に失敗しました");
    println!("クライアント 2 からメッセージ送信完了");

    // メッセージ受信を待つ (最大 10 秒)
    let client1_received_wait = client1_received.clone();
    let client2_received_wait = client2_received.clone();
    let msg_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let c1 = !client1_received_wait.lock().unwrap().is_empty();
            let c2 = !client2_received_wait.lock().unwrap().is_empty();
            if c1 && c2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(msg_wait.is_ok(), "メッセージ受信がタイムアウトしました");

    // クライアント 1 がクライアント 2 からのメッセージを受信していることを確認
    {
        let received1 = client1_received.lock().unwrap();
        assert!(
            received1.iter().any(|m| m == message_from_2),
            "クライアント 1 がクライアント 2 からのメッセージを受信していません (受信: {:?})",
            received1
        );
    }

    // クライアント 2 がクライアント 1 からのメッセージを受信していることを確認
    {
        let received2 = client2_received.lock().unwrap();
        assert!(
            received2.iter().any(|m| m == message_from_1),
            "クライアント 2 がクライアント 1 からのメッセージを受信していません (受信: {:?})",
            received2
        );
    }

    println!("メッセージ送受信の検証成功");

    // 切断
    handle1
        .disconnect()
        .await
        .expect("クライアント 1 の disconnect に失敗しました");
    handle2
        .disconnect()
        .await
        .expect("クライアント 2 の disconnect に失敗しました");

    e2e_tests::wait_task_finished(client1_task, "client1_task").await;
    e2e_tests::wait_task_finished(client2_task, "client2_task").await;

    println!("テスト成功: data_channels を指定したメッセージングの送受信を確認しました");
}
