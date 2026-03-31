use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_stats_field_positive,
};
use sora_sdk::{Role, SoraClient, SoraClientContext, Video};

/// テスト用のチャンネル ID を生成する (suffix 付き)
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

/// 指定したコーデックで SendOnly → RecvOnly の接続テストを実行する
async fn run_sendonly_recvonly_with_codec(video: Video, codec_name: &str) {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id(&format!("{}-sendonly-recvonly", codec_name));

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
    .video(video)
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
        let _ = tokio::time::timeout(Duration::from_secs(30), sendonly_client.run()).await;
    });

    // SendOnly が接続するまで待機
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
        "{}: SendOnly クライアントの接続がタイムアウトしました",
        codec_name
    );

    println!("{}: SendOnly 接続完了、RecvOnly を起動します", codec_name);

    tokio::time::sleep(Duration::from_secs(1)).await;

    // RecvOnly クライアントを作成・起動
    let recvonly_context = SoraClientContext::new().expect("RecvOnly コンテキスト作成失敗");

    let mut recvonly_builder =
        SoraClient::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
            .data_channel_signaling(true)
            .on_notify(move |_| {
                recvonly_connected_clone.store(true, Ordering::SeqCst);
            })
            .on_track(move |_track| {
                track_received_clone.fetch_add(1, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (recvonly_client, recvonly_handle) = recvonly_builder
        .build()
        .expect("SoraClient の作成に失敗しました");

    let recvonly_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), recvonly_client.run()).await;
    });

    // RecvOnly が接続するまで待機
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
        "{}: RecvOnly クライアントの接続がタイムアウトしました",
        codec_name
    );

    println!(
        "{}: RecvOnly 接続完了、トラック受信を待機します",
        codec_name
    );

    // トラック受信を待機
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

    let tracks = track_received.load(Ordering::SeqCst);
    assert!(
        track_wait.is_ok() && tracks > 0,
        "{}: RecvOnly クライアントがトラックを受信できませんでした (受信数: {}, タイムアウト: {})",
        codec_name,
        tracks,
        track_wait.is_err()
    );

    // 統計情報を検証 (H.264/H.265 はエンコード開始まで時間がかかる場合がある)
    tokio::time::sleep(Duration::from_secs(5)).await;

    let sendonly_stats = sendonly_handle
        .get_stats()
        .await
        .expect("SendOnly の get_stats に失敗しました");
    let recvonly_stats = recvonly_handle
        .get_stats()
        .await
        .expect("RecvOnly の get_stats に失敗しました");

    println!("{}: SendOnly stats: {}", codec_name, sendonly_stats);
    println!("{}: RecvOnly stats: {}", codec_name, recvonly_stats);

    assert!(
        verify_stats_field_positive(&sendonly_stats, "outbound-rtp", "packetsSent"),
        "{}: SendOnly の outbound-rtp の packetsSent が 0 より大きくありません",
        codec_name
    );
    assert!(
        verify_stats_field_positive(&recvonly_stats, "inbound-rtp", "packetsReceived"),
        "{}: RecvOnly の inbound-rtp の packetsReceived が 0 より大きくありません",
        codec_name
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

    sendonly_task.abort();
    recvonly_task.abort();

    println!(
        "{}: テスト成功: {} トラックを受信、統計情報検証完了",
        codec_name, tracks
    );
}

/// 指定したコーデックで SendRecv の双方向接続テストを実行する
async fn run_sendrecv_with_codec(video: Video, codec_name: &str) {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id(&format!("{}-sendrecv", codec_name));

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
    let context1 = SoraClientContext::new().expect("クライアント 1 コンテキスト作成失敗");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 作成失敗");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("送信用トラック作成失敗");

    let mut builder1 =
        SoraClient::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .video(video.clone())
            .data_channel_signaling(true)
            .on_notify(move |_| {
                client1_connected_clone.store(true, Ordering::SeqCst);
            })
            .on_track(move |_track| {
                client1_track_received_clone.fetch_add(1, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let (client1, handle1) = builder1.build().expect("SoraClient の作成に失敗しました");

    let client1_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), client1.run()).await;
    });

    // クライアント 1 が接続するまで待機
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
        "{}: クライアント 1 の接続がタイムアウトしました",
        codec_name
    );

    println!(
        "{}: クライアント 1 接続完了、クライアント 2 を起動します",
        codec_name
    );

    tokio::time::sleep(Duration::from_secs(1)).await;

    // クライアント 2 を作成・起動
    let context2 = SoraClientContext::new().expect("クライアント 2 コンテキスト作成失敗");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 作成失敗");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("送信用トラック作成失敗");

    let mut builder2 = SoraClient::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .video(video)
        .data_channel_signaling(true)
        .on_notify(move |_| {
            client2_connected_clone.store(true, Ordering::SeqCst);
        })
        .on_track(move |_track| {
            client2_track_received_clone.fetch_add(1, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let (client2, handle2) = builder2.build().expect("SoraClient の作成に失敗しました");

    let client2_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), client2.run()).await;
    });

    // クライアント 2 が接続するまで待機
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
        "{}: クライアント 2 の接続がタイムアウトしました",
        codec_name
    );

    println!(
        "{}: クライアント 2 接続完了、トラック受信を待機します",
        codec_name
    );

    // 両方のクライアントがトラックを受信するまで待機
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

    let tracks1 = client1_track_received.load(Ordering::SeqCst);
    let tracks2 = client2_track_received.load(Ordering::SeqCst);

    assert!(
        track_wait.is_ok() && tracks1 > 0 && tracks2 > 0,
        "{}: 相互のトラック受信に失敗しました (クライアント 1: {} トラック, クライアント 2: {} トラック)",
        codec_name,
        tracks1,
        tracks2
    );

    // 統計情報を検証 (H.264/H.265 はエンコード開始まで時間がかかる場合がある)
    tokio::time::sleep(Duration::from_secs(5)).await;

    let stats1 = handle1
        .get_stats()
        .await
        .expect("クライアント 1 の get_stats に失敗しました");
    let stats2 = handle2
        .get_stats()
        .await
        .expect("クライアント 2 の get_stats に失敗しました");

    assert!(
        verify_stats_field_positive(&stats1, "outbound-rtp", "packetsSent"),
        "{}: クライアント 1 の outbound-rtp の packetsSent が 0 より大きくありません",
        codec_name
    );
    assert!(
        verify_stats_field_positive(&stats2, "outbound-rtp", "packetsSent"),
        "{}: クライアント 2 の outbound-rtp の packetsSent が 0 より大きくありません",
        codec_name
    );
    assert!(
        verify_stats_field_positive(&stats1, "inbound-rtp", "packetsReceived"),
        "{}: クライアント 1 の inbound-rtp の packetsReceived が 0 より大きくありません",
        codec_name
    );
    assert!(
        verify_stats_field_positive(&stats2, "inbound-rtp", "packetsReceived"),
        "{}: クライアント 2 の inbound-rtp の packetsReceived が 0 より大きくありません",
        codec_name
    );

    // 切断
    handle1
        .disconnect()
        .await
        .expect("クライアント 1 の disconnect に失敗しました");
    handle2
        .disconnect()
        .await
        .expect("クライアント 2 の disconnect に失敗しました");

    client1_task.abort();
    client2_task.abort();

    println!("{}: テスト成功: 双方向通信、統計情報検証完了", codec_name);
}

/// H.264 で SendOnly → RecvOnly の接続テスト
/// webrtc-rs が macOS H.264/H.265 に未対応のため一時的に ignore
#[tokio::test]
#[ignore]
async fn test_h264_sendonly_recvonly() {
    run_sendonly_recvonly_with_codec(Video::new_h264(None, None), "H264").await;
}

/// H.265 で SendOnly → RecvOnly の接続テスト
/// webrtc-rs が macOS H.264/H.265 に未対応のため一時的に ignore
#[tokio::test]
#[ignore]
async fn test_h265_sendonly_recvonly() {
    run_sendonly_recvonly_with_codec(Video::new_h265(None, None), "H265").await;
}

/// H.264 で SendRecv の双方向接続テスト
/// webrtc-rs が macOS H.264/H.265 に未対応のため一時的に ignore
#[tokio::test]
#[ignore]
async fn test_h264_sendrecv() {
    run_sendrecv_with_codec(Video::new_h264(None, None), "H264").await;
}

/// H.265 で SendRecv の双方向接続テスト
/// webrtc-rs が macOS H.264/H.265 に未対応のため一時的に ignore
#[tokio::test]
#[ignore]
async fn test_h265_sendrecv() {
    run_sendrecv_with_codec(Video::new_h265(None, None), "H265").await;
}
