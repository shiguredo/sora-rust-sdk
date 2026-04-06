use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_video_stats_field_positive,
};
use sora_sdk::{Role, SoraClient, SoraClientContext};

/// テスト用のチャンネル ID を生成する (suffix 付き)
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

/// SendOnly を先に接続してから RecvOnly を接続するテスト
#[tokio::test]
async fn test_sendonly_then_recvonly() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("sendonly-first");

    // SendOnly クライアントの状態
    let sendonly_connected = Arc::new(AtomicBool::new(false));
    let sendonly_connected_clone = sendonly_connected.clone();

    // RecvOnly クライアントの状態
    let recvonly_connected = Arc::new(AtomicBool::new(false));
    let recvonly_connected_clone = recvonly_connected.clone();
    let track_received = Arc::new(AtomicUsize::new(0));
    let track_received_clone = track_received.clone();

    // SendOnly クライアントを作成・起動
    let sendonly_context = SoraClientContext::new().expect("SendOnly コンテキスト作成失敗");
    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&sendonly_context, &mut capturer).expect("送信用トラック作成失敗");

    let mut sendonly_builder = SoraClient::builder(
        sendonly_context,
        urls.clone(),
        channel_id.clone(),
        Role::SendOnly,
    )
    .sender_video_track(video_track)
    .sender_audio_track(audio_track)
    .data_channel_signaling(true)
    .on_notify(move |_| {
        sendonly_connected_clone.store(true, Ordering::SeqCst);
    });

    if let Some(token) = secret_key() {
        sendonly_builder = sendonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (sendonly_client, sendonly_handle) = sendonly_builder
        .build()
        .expect("SoraClient の作成に失敗しました");

    // SendOnly を起動
    let sendonly_task = tokio::spawn(async move {
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(30), sendonly_client.run()).await;
    });

    // SendOnly が接続するまで待機 (最大 10 秒)
    let sendonly_connected_for_wait = sendonly_connected.clone();
    let sendonly_wait = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if sendonly_connected_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        sendonly_wait.is_ok(),
        "SendOnly クライアントの接続がタイムアウトしました"
    );

    println!("SendOnly 接続完了、RecvOnly を起動します");

    // SendOnly 接続後、少し待機してから RecvOnly を起動
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // RecvOnly クライアントを作成・起動
    let recvonly_context = SoraClientContext::new().expect("RecvOnly コンテキスト作成失敗");

    let mut recvonly_builder =
        SoraClient::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
            .data_channel_signaling(true)
            .on_notify(move |_| {
                recvonly_connected_clone.store(true, Ordering::SeqCst);
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
                println!("トラックを受信しました");
                track_received_clone.fetch_add(1, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (recvonly_client, recvonly_handle) = recvonly_builder
        .build()
        .expect("SoraClient の作成に失敗しました");

    let recvonly_task = tokio::spawn(async move {
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(30), recvonly_client.run()).await;
    });

    // RecvOnly が接続するまで待機 (最大 10 秒)
    let recvonly_connected_for_wait = recvonly_connected.clone();
    let recvonly_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if recvonly_connected_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        recvonly_wait.is_ok(),
        "RecvOnly クライアントの接続がタイムアウトしました"
    );

    println!("RecvOnly 接続完了、トラック受信を待機します");

    // トラック受信を待機 (最大 10 秒)
    let track_received_for_wait = track_received.clone();
    let track_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if track_received_for_wait.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    // 結果を取得
    let tracks = track_received.load(Ordering::SeqCst);

    // RecvOnly がトラックを受信したことを確認
    assert!(
        track_wait.is_ok() && tracks > 0,
        "RecvOnly クライアントがトラックを受信できませんでした (受信数: {}, タイムアウト: {})",
        tracks,
        track_wait.is_err()
    );

    // 統計情報を検証
    tokio::time::sleep(Duration::from_secs(2)).await;

    let sendonly_stats = sendonly_handle
        .get_stats()
        .await
        .expect("SendOnly の get_stats に失敗しました");
    let recvonly_stats = recvonly_handle
        .get_stats()
        .await
        .expect("RecvOnly の get_stats に失敗しました");

    assert!(
        verify_video_stats_field_positive(&sendonly_stats, "outbound-rtp", "packetsSent"),
        "SendOnly の outbound-rtp の packetsSent が 0 より大きくありません"
    );
    assert!(
        verify_video_stats_field_positive(&recvonly_stats, "inbound-rtp", "packetsReceived"),
        "RecvOnly の inbound-rtp の packetsReceived が 0 より大きくありません"
    );

    // 切断
    sendonly_handle
        .disconnect()
        .await
        .expect("SendOnly の disconnect に失敗しました");
    recvonly_handle
        .disconnect()
        .await
        .expect("RecvOnly の disconnect に失敗しました");

    // タスクをキャンセル
    sendonly_task.abort();
    recvonly_task.abort();

    println!("テスト成功: {} トラックを受信、統計情報検証完了", tracks);
}

/// RecvOnly を先に接続してから SendOnly を接続するテスト (re-offer のテスト)
#[tokio::test]
async fn test_recvonly_then_sendonly() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("recvonly-first");

    // SendOnly クライアントの状態
    let sendonly_connected = Arc::new(AtomicBool::new(false));
    let sendonly_connected_clone = sendonly_connected.clone();

    // RecvOnly クライアントの状態
    let recvonly_connected = Arc::new(AtomicBool::new(false));
    let recvonly_connected_clone = recvonly_connected.clone();
    let track_received = Arc::new(AtomicUsize::new(0));
    let track_received_clone = track_received.clone();

    // RecvOnly クライアントを作成・起動 (先に起動)
    let recvonly_context = SoraClientContext::new().expect("RecvOnly コンテキスト作成失敗");

    let mut recvonly_builder = SoraClient::builder(
        recvonly_context,
        urls.clone(),
        channel_id.clone(),
        Role::RecvOnly,
    )
    .data_channel_signaling(true)
    .on_notify(move |_| {
        recvonly_connected_clone.store(true, Ordering::SeqCst);
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
        println!("トラックを受信しました");
        track_received_clone.fetch_add(1, Ordering::SeqCst);
    });

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (recvonly_client, recvonly_handle) = recvonly_builder
        .build()
        .expect("SoraClient の作成に失敗しました");

    let recvonly_task = tokio::spawn(async move {
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(30), recvonly_client.run()).await;
    });

    // RecvOnly が接続するまで待機 (最大 10 秒)
    let recvonly_connected_for_wait = recvonly_connected.clone();
    let recvonly_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if recvonly_connected_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        recvonly_wait.is_ok(),
        "RecvOnly クライアントの接続がタイムアウトしました"
    );

    println!("RecvOnly 接続完了、SendOnly を起動します");

    // RecvOnly 接続後、少し待機してから SendOnly を起動
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // SendOnly クライアントを作成・起動
    let sendonly_context = SoraClientContext::new().expect("SendOnly コンテキスト作成失敗");
    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&sendonly_context, &mut capturer).expect("送信用トラック作成失敗");

    let mut sendonly_builder =
        SoraClient::builder(sendonly_context, urls, channel_id, Role::SendOnly)
            .sender_video_track(video_track)
            .sender_audio_track(audio_track)
            .data_channel_signaling(true)
            .on_notify(move |_| {
                sendonly_connected_clone.store(true, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        sendonly_builder = sendonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (sendonly_client, sendonly_handle) = sendonly_builder
        .build()
        .expect("SoraClient の作成に失敗しました");

    let sendonly_task = tokio::spawn(async move {
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(30), sendonly_client.run()).await;
    });

    // SendOnly が接続するまで待機 (最大 10 秒)
    let sendonly_connected_for_wait = sendonly_connected.clone();
    let sendonly_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if sendonly_connected_for_wait.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        sendonly_wait.is_ok(),
        "SendOnly クライアントの接続がタイムアウトしました"
    );

    println!("SendOnly 接続完了、トラック受信を待機します (re-offer 経由)");

    // トラック受信を待機 (最大 10 秒) - re-offer 経由でトラックが届くはず
    let track_received_for_wait = track_received.clone();
    let track_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if track_received_for_wait.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    // 結果を取得
    let tracks = track_received.load(Ordering::SeqCst);

    // RecvOnly がトラックを受信したことを確認
    assert!(
        track_wait.is_ok() && tracks > 0,
        "RecvOnly クライアントが re-offer 経由でトラックを受信できませんでした (受信数: {}, タイムアウト: {})",
        tracks,
        track_wait.is_err()
    );

    // 統計情報を検証
    tokio::time::sleep(Duration::from_secs(2)).await;

    let sendonly_stats = sendonly_handle
        .get_stats()
        .await
        .expect("SendOnly の get_stats に失敗しました");
    let recvonly_stats = recvonly_handle
        .get_stats()
        .await
        .expect("RecvOnly の get_stats に失敗しました");

    assert!(
        verify_video_stats_field_positive(&sendonly_stats, "outbound-rtp", "packetsSent"),
        "SendOnly の outbound-rtp の packetsSent が 0 より大きくありません"
    );
    assert!(
        verify_video_stats_field_positive(&recvonly_stats, "inbound-rtp", "packetsReceived"),
        "RecvOnly の inbound-rtp の packetsReceived が 0 より大きくありません"
    );

    // 切断
    sendonly_handle
        .disconnect()
        .await
        .expect("SendOnly の disconnect に失敗しました");
    recvonly_handle
        .disconnect()
        .await
        .expect("RecvOnly の disconnect に失敗しました");

    // タスクをキャンセル
    sendonly_task.abort();
    recvonly_task.abort();

    println!(
        "テスト成功: re-offer 経由で {} トラックを受信、統計情報検証完了",
        tracks
    );
}
