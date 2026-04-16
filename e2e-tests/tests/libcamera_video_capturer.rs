#![cfg(feature = "libcamera")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use e2e_tests::{
    build_metadata_with_access_token, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_video_stats_field_positive,
};
use serial_test::serial;
use sora_sdk::{LibcameraVideoCapturer, Role, SoraConnection, SoraConnectionContext};

fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

#[serial]
#[tokio::test]
async fn test_sendonly_recvonly_with_libcamera() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("libcamera-sendonly-recvonly");

    let sendonly_connected = Arc::new(AtomicBool::new(false));
    let sendonly_connected_clone = sendonly_connected.clone();

    let recvonly_connected = Arc::new(AtomicBool::new(false));
    let recvonly_connected_clone = recvonly_connected.clone();
    let track_received = Arc::new(AtomicUsize::new(0));
    let track_received_clone = track_received.clone();

    let sendonly_context = SoraConnectionContext::new().expect("SendOnly コンテキスト作成失敗");
    let mut capturer = LibcameraVideoCapturer::builder()
        .width(640)
        .height(480)
        .build()
        .expect("libcamera capturer 作成失敗");
    capturer.start().expect("libcamera capturer 開始失敗");

    let video_track = sendonly_context
        .create_video_track(&capturer.video_source())
        .expect("映像トラック作成失敗");
    let audio_source = sendonly_context
        .create_audio_source()
        .expect("音声ソース作成失敗");
    let audio_track = sendonly_context
        .create_audio_track(&audio_source)
        .expect("音声トラック作成失敗");

    let mut sendonly_builder = SoraConnection::builder(
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
        .expect("SendOnly クライアント作成失敗");

    let sendonly_task = tokio::spawn(async move {
        sendonly_client
            .run()
            .await
            .expect("sendonly_client run failed");
    });

    let recvonly_context = SoraConnectionContext::new().expect("RecvOnly コンテキスト作成失敗");

    let mut recvonly_builder =
        SoraConnection::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
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
                track_received_clone.fetch_add(1, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (recvonly_client, recvonly_handle) = recvonly_builder
        .build()
        .expect("RecvOnly クライアント作成失敗");

    let recvonly_task = tokio::spawn(async move {
        recvonly_client
            .run()
            .await
            .expect("recvonly_client run failed");
    });

    let mut test_error = None;

    let sendonly_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if sendonly_connected.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    if sendonly_wait.is_err() {
        test_error = Some("sendonly connection timed out".to_string());
    }

    if test_error.is_none() {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    if test_error.is_none() {
        let recvonly_wait = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if recvonly_connected.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        if recvonly_wait.is_err() {
            test_error = Some("recvonly connection timed out".to_string());
        }
    }

    if test_error.is_none() {
        let track_wait = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if track_received.load(Ordering::SeqCst) > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        if track_wait.is_err() || track_received.load(Ordering::SeqCst) == 0 {
            test_error = Some("recvonly did not receive video track".to_string());
        }
    }

    if test_error.is_none() {
        let stats_wait = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let sendonly_stats = sendonly_handle
                    .get_stats()
                    .await
                    .map_err(|err| format!("failed to get sendonly stats: {err}"))?;
                let recvonly_stats = recvonly_handle
                    .get_stats()
                    .await
                    .map_err(|err| format!("failed to get recvonly stats: {err}"))?;
                if verify_video_stats_field_positive(&sendonly_stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(
                        &recvonly_stats,
                        "inbound-rtp",
                        "packetsReceived",
                    )
                {
                    break Ok(());
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await;

        match stats_wait {
            Ok(Ok(())) => {}
            Ok(Err(err)) => test_error = Some(err),
            Err(_) => {
                test_error = Some(
                    "video statistics did not reach expected values within timeout".to_string(),
                )
            }
        }
    }

    let _ = sendonly_handle.disconnect().await;
    let _ = recvonly_handle.disconnect().await;

    e2e_tests::wait_task_finished(sendonly_task, "sendonly_task").await;
    e2e_tests::wait_task_finished(recvonly_task, "recvonly_task").await;
    capturer.stop();

    if let Some(err) = test_error {
        panic!("{}", err);
    }
}
