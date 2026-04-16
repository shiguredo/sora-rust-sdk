use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_video_stats_field_positive,
};
use sora_sdk::{Role, SoraConnection, SoraConnectionContext};

/// テスト用のチャンネル ID を生成する (suffix 付き)
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

/// 2 つの SendRecv クライアントが相互に接続して通信するテスト
#[tokio::test]
async fn test_sendrecv_bidirectional() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("sendrecv-bidirectional");

    // クライアント 1 の状態
    let client1_connected = Arc::new(AtomicBool::new(false));
    let client1_connected_clone = client1_connected.clone();
    let client1_track_received = Arc::new(AtomicUsize::new(0));
    let client1_track_received_clone = client1_track_received.clone();

    // クライアント 2 の状態
    let client2_connected = Arc::new(AtomicBool::new(false));
    let client2_connected_clone = client2_connected.clone();
    let client2_track_received = Arc::new(AtomicUsize::new(0));
    let client2_track_received_clone = client2_track_received.clone();

    // クライアント 1 を作成・起動
    let context1 = SoraConnectionContext::new().expect("クライアント 1 コンテキスト作成失敗");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 作成失敗");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("送信用トラック作成失敗");

    let mut builder1 =
        SoraConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .data_channel_signaling(true)
            .on_notify(move |_| {
                client1_connected_clone.store(true, Ordering::SeqCst);
            })
            .on_track(move |transceiver| {
                let receiver = transceiver.receiver();
                let track = receiver.track();
                let kind = match track.kind() {
                    Ok(kind) => kind,
                    Err(_) => return,
                };
                if kind != "video" {
                    return;
                }
                println!("クライアント 1: トラックを受信しました");
                client1_track_received_clone.fetch_add(1, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let (client1, handle1) = builder1
        .build()
        .expect("SoraConnection の作成に失敗しました");

    // クライアント 1 を起動
    let client1_task = tokio::spawn(async move {
        client1.run().await.expect("client1 run failed");
    });

    // クライアント 1 が接続するまで待機 (最大 10 秒)
    let client1_connected_for_wait = client1_connected.clone();
    let client1_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if client1_connected_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        client1_wait.is_ok(),
        "クライアント 1 の接続がタイムアウトしました"
    );

    println!("クライアント 1 接続完了、クライアント 2 を起動します");

    // クライアント 1 接続後、少し待機してからクライアント 2 を起動
    tokio::time::sleep(Duration::from_secs(1)).await;

    // クライアント 2 を作成・起動
    let context2 = SoraConnectionContext::new().expect("クライアント 2 コンテキスト作成失敗");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 作成失敗");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("送信用トラック作成失敗");

    let mut builder2 = SoraConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .data_channel_signaling(true)
        .on_notify(move |_| {
            client2_connected_clone.store(true, Ordering::SeqCst);
        })
        .on_track(move |transceiver| {
            let receiver = transceiver.receiver();
            let track = receiver.track();
            let kind = match track.kind() {
                Ok(kind) => kind,
                Err(_) => return,
            };
            if kind != "video" {
                return;
            }
            println!("クライアント 2: トラックを受信しました");
            client2_track_received_clone.fetch_add(1, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let (client2, handle2) = builder2
        .build()
        .expect("SoraConnection の作成に失敗しました");

    let client2_task = tokio::spawn(async move {
        client2.run().await.expect("client2 run failed");
    });

    // クライアント 2 が接続するまで待機 (最大 10 秒)
    let client2_connected_for_wait = client2_connected.clone();
    let client2_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if client2_connected_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        client2_wait.is_ok(),
        "クライアント 2 の接続がタイムアウトしました"
    );

    println!("クライアント 2 接続完了、トラック受信を待機します");

    // 両方のクライアントがトラックを受信するまで待機 (最大 15 秒)
    let client1_track_for_wait = client1_track_received.clone();
    let client2_track_for_wait = client2_track_received.clone();
    let track_wait = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let tracks1 = client1_track_for_wait.load(Ordering::SeqCst);
            let tracks2 = client2_track_for_wait.load(Ordering::SeqCst);
            if tracks1 > 0 && tracks2 > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    // 結果を取得
    let tracks1 = client1_track_received.load(Ordering::SeqCst);
    let tracks2 = client2_track_received.load(Ordering::SeqCst);

    // トラック受信の検証
    assert!(
        track_wait.is_ok() && tracks1 > 0 && tracks2 > 0,
        "相互のトラック受信に失敗しました (クライアント 1: {} トラック, クライアント 2: {} トラック, タイムアウト: {})",
        tracks1,
        tracks2,
        track_wait.is_err()
    );

    println!(
        "トラック受信成功: クライアント 1: {} トラック, クライアント 2: {} トラック",
        tracks1, tracks2
    );

    // 統計情報を取得・検証
    // パケットが実際に送受信されるまで少し待機
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("統計情報を取得します");

    let stats1 = handle1
        .get_stats()
        .await
        .expect("クライアント 1 の get_stats に失敗しました");
    let stats2 = handle2
        .get_stats()
        .await
        .expect("クライアント 2 の get_stats に失敗しました");

    // 統計情報に outbound-rtp が含まれ、packetsSent が 0 より大きいことを確認
    assert!(
        verify_video_stats_field_positive(&stats1, "outbound-rtp", "packetsSent"),
        "クライアント 1 の outbound-rtp の packetsSent が 0 より大きくありません"
    );
    assert!(
        verify_video_stats_field_positive(&stats2, "outbound-rtp", "packetsSent"),
        "クライアント 2 の outbound-rtp の packetsSent が 0 より大きくありません"
    );

    // 統計情報に inbound-rtp が含まれ、packetsReceived が 0 より大きいことを確認
    assert!(
        verify_video_stats_field_positive(&stats1, "inbound-rtp", "packetsReceived"),
        "クライアント 1 の inbound-rtp の packetsReceived が 0 より大きくありません"
    );
    assert!(
        verify_video_stats_field_positive(&stats2, "inbound-rtp", "packetsReceived"),
        "クライアント 2 の inbound-rtp の packetsReceived が 0 より大きくありません"
    );

    println!("統計情報の検証成功 (送受信パケット数 > 0)");

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

    println!("テスト成功: 2 つの SendRecv クライアントが相互に通信しました");
}
