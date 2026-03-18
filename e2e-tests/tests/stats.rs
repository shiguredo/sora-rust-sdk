use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_stats_field_positive,
};
use sora_sdk::{JsonString, Role, SoraClient, SoraClientContext};

#[tokio::test]
async fn test_get_stats() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraClientContext::new().expect("コンテキスト作成失敗");

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let connected = Arc::new(AtomicBool::new(false));
    let connected_clone = connected.clone();

    let mut builder = SoraClient::builder(context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .on_notify(move |_| {
            connected_clone.store(true, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let (client, handle) = builder.build().expect("SoraClient の作成に失敗しました");

    // run() と統計取得を並行して実行
    let stats_result = Arc::new(std::sync::Mutex::new(Option::<JsonString>::None));
    let stats_result_clone = stats_result.clone();

    tokio::select! {
        _ = client.run() => {
            // run が終了した（通常はここに来ない）
        }
        _ = async {
            // 接続成功を待機
            tokio::time::sleep(Duration::from_secs(5)).await;

            // 接続成功を確認
            assert!(connected.load(Ordering::SeqCst), "接続に失敗しました");

            // 統計情報を取得
            let stats = handle
                .get_stats()
                .await
                .expect("get_stats の取得に失敗しました");
            *stats_result_clone.lock().unwrap() = Some(stats);

            // 切断
            handle
                .disconnect()
                .await
                .expect("disconnect の実行に失敗しました");
        } => {}
    }

    // 統計情報の検証
    let stats = stats_result.lock().unwrap().clone().unwrap();

    // outbound-rtp の packetsSent が 0 より大きいことを確認
    assert!(
        verify_stats_field_positive(&stats, "outbound-rtp", "packetsSent"),
        "outbound-rtp の packetsSent が 0 より大きくありません"
    );
}
