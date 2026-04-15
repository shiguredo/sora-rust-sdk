use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, generate_channel_id, load_env, secret_key, signaling_urls,
};
use sora_sdk::{Role, SignalingDirection, SignalingType, SoraConnection, SoraConnectionContext};

/// WebSocket 経由で on_signaling_message コールバックが Sent / Received 両方呼ばれることを確認する。
#[tokio::test]
async fn test_on_signaling_message_websocket() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let sent_received = Arc::new(AtomicBool::new(false));
    let recv_received = Arc::new(AtomicBool::new(false));
    let sent_clone = sent_received.clone();
    let recv_clone = recv_received.clone();

    let mut builder = SoraConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(false)
        .on_signaling_message(move |signaling_type, direction, text| {
            println!(
                "on_signaling_message: {:?} {:?} {}",
                signaling_type,
                direction,
                &text[..text.len().min(100)]
            );
            assert_eq!(signaling_type, SignalingType::WebSocket);
            match direction {
                SignalingDirection::Sent => {
                    sent_clone.store(true, Ordering::SeqCst);
                }
                SignalingDirection::Received => {
                    recv_clone.store(true, Ordering::SeqCst);
                }
            }
        });

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (connection, _handle) = builder
        .build()
        .expect("SoraConnection の作成に失敗しました");

    let _ = tokio::time::timeout(Duration::from_secs(10), connection.run()).await;

    assert!(
        sent_received.load(Ordering::SeqCst),
        "Sent 方向の on_signaling_message が呼ばれませんでした"
    );
    assert!(
        recv_received.load(Ordering::SeqCst),
        "Received 方向の on_signaling_message が呼ばれませんでした"
    );

    println!("テスト成功: WebSocket 経由で Sent / Received の両方が呼ばれました");
}

/// DataChannel シグナリング有効時に re-offer / re-answer で
/// DataChannel 経由の on_signaling_message が呼ばれることを確認する。
///
/// recvonly を接続後に sendonly を同じチャンネルに接続することで re-offer を発火させる。
#[tokio::test]
async fn test_on_signaling_message_datachannel() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();

    // --- recvonly クライアント ---
    let recv_context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let dc_received = Arc::new(AtomicBool::new(false));
    let dc_sent = Arc::new(AtomicBool::new(false));
    let switched_received = Arc::new(AtomicBool::new(false));

    let dc_received_clone = dc_received.clone();
    let dc_sent_clone = dc_sent.clone();
    let switched_received_clone = switched_received.clone();

    let mut recv_builder = SoraConnection::builder(
        recv_context,
        urls.clone(),
        channel_id.clone(),
        Role::RecvOnly,
    )
    .data_channel_signaling(true)
    .on_signaling_message(move |signaling_type, direction, text| {
        println!(
            "[recvonly] on_signaling_message: {:?} {:?} {}",
            signaling_type,
            direction,
            &text[..text.len().min(100)]
        );
        if signaling_type == SignalingType::DataChannel {
            match direction {
                SignalingDirection::Received => {
                    dc_received_clone.store(true, Ordering::SeqCst);
                }
                SignalingDirection::Sent => {
                    dc_sent_clone.store(true, Ordering::SeqCst);
                }
            }
        }
    })
    .on_switched(move || {
        switched_received_clone.store(true, Ordering::SeqCst);
    });

    if let Some(token) = secret_key() {
        recv_builder = recv_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (recv_client, recv_handle) = recv_builder.build().expect("recvonly の作成に失敗しました");

    let recv_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), recv_client.run()).await;
    });

    // recvonly の switched を待つ
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
        "recvonly の switched 通知がタイムアウトしました"
    );

    // --- sendonly クライアントを接続して re-offer を発火させる ---
    let send_context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&send_context, &mut capturer).expect("送信用トラック作成失敗");

    let mut send_builder = SoraConnection::builder(
        send_context,
        urls.clone(),
        channel_id.clone(),
        Role::SendOnly,
    )
    .sender_video_track(video_track)
    .sender_audio_track(audio_track);

    if let Some(token) = secret_key() {
        send_builder = send_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (send_client, send_handle) = send_builder.build().expect("sendonly の作成に失敗しました");

    let send_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(15), send_client.run()).await;
    });

    // recvonly 側で DataChannel 経由の re-offer (Received) と re-answer (Sent) を待つ
    let dc_received_for_wait = dc_received.clone();
    let dc_sent_for_wait = dc_sent.clone();
    let dc_wait = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if dc_received_for_wait.load(Ordering::SeqCst)
                && dc_sent_for_wait.load(Ordering::SeqCst)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        dc_wait.is_ok(),
        "DataChannel 経由の on_signaling_message がタイムアウトしました (Received={}, Sent={})",
        dc_received.load(Ordering::SeqCst),
        dc_sent.load(Ordering::SeqCst),
    );

    // 切断
    recv_handle
        .disconnect()
        .await
        .expect("recvonly の disconnect に失敗しました");
    send_handle
        .disconnect()
        .await
        .expect("sendonly の disconnect に失敗しました");

    recv_task.abort();
    send_task.abort();

    println!(
        "テスト成功: DataChannel 経由で re-offer (Received) と re-answer (Sent) が呼ばれました"
    );
}
