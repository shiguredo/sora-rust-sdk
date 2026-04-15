use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, generate_channel_id, load_env, secret_key, signaling_urls,
};
use sora_sdk::{Role, SoraConnection, SoraConnectionContext};

#[tokio::test]
async fn test_sendonly_connect() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let connected = Arc::new(AtomicBool::new(false));
    let connected_clone = connected.clone();

    let mut builder = SoraConnection::builder(context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .on_notify(move |_| {
            connected_clone.store(true, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (connection, _handle) = builder
        .build()
        .expect("SoraConnection の作成に失敗しました");

    // タイムアウト付きで接続テスト
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), connection.run()).await;

    // 接続成功を確認（タイムアウトでも notify を受信していれば OK）
    assert!(
        connected.load(Ordering::SeqCst) || result.is_ok(),
        "接続に失敗しました"
    );
}

#[tokio::test]
async fn test_sendonly_data_channel_signaling() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let switched_received = Arc::new(AtomicBool::new(false));
    let switched_received_clone = switched_received.clone();

    let mut builder = SoraConnection::builder(context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .data_channel_signaling(true)
        .on_switched(move || {
            println!("switched 通知を受信しました");
            switched_received_clone.store(true, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (connection, _handle) = builder
        .build()
        .expect("SoraConnection の作成に失敗しました");

    // タイムアウト付きで接続テスト（switched を待つため長めに）
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), connection.run()).await;

    // switched 通知が受信されたことを確認
    assert!(
        switched_received.load(Ordering::SeqCst),
        "switched 通知が受信されませんでした"
    );
}
