#[cfg(any(target_os = "macos", target_os = "ios"))]
use std::time::Duration;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, verify_video_codec_mime_type, verify_video_stats_field_positive,
};
#[cfg(any(target_os = "macos", target_os = "ios"))]
use shiguredo_webrtc::VideoCodecType;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use sora_sdk::{Role, SoraConnectionContext, Video};

/// テスト用のチャンネル ID を生成する (suffix 付き)
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

/// 指定 codec の Encoder / Decoder が両方対応しているか確認する。
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn is_codec_fully_supported(codec_type: VideoCodecType) -> bool {
    let config = sora_sdk::SoraConnectionContextConfig::default();
    let has_encoder = config
        .video_codec_capabilities
        .iter()
        .any(|capability| capability.is_supported(sora_sdk::CodecDirection::Encoder, codec_type));
    let has_decoder = config
        .video_codec_capabilities
        .iter()
        .any(|capability| capability.is_supported(sora_sdk::CodecDirection::Decoder, codec_type));
    has_encoder && has_decoder
}

/// 指定 codec が未対応ならスキップし、スキップしたかを返す。
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn skip_if_codec_not_fully_supported(codec_type: VideoCodecType, codec_name: &str) -> bool {
    if is_codec_fully_supported(codec_type) {
        return false;
    }
    println!("SKIP: {codec_name} の encoder/decoder が完全にサポートされていません");
    true
}

/// 指定したコーデックで SendOnly → RecvOnly の接続テストを実行する
#[cfg(any(target_os = "macos", target_os = "ios"))]
async fn run_sendonly_recvonly_with_codec(
    video: Video,
    codec_name: &str,
    expected_mime_type: &str,
) {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id(&format!("{}-sendonly-recvonly", codec_name));

    // SendOnly クライアントを作成・起動
    let sendonly_context = SoraConnectionContext::new().expect("SendOnly コンテキスト作成失敗");
    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&sendonly_context, &mut capturer).expect("送信用トラック作成失敗");

    let mut sendonly_builder = SoraTestConnection::builder(
        sendonly_context,
        urls.clone(),
        channel_id.clone(),
        Role::SendOnly,
    )
    .sender_video_track(video_track)
    .sender_audio_track(audio_track)
    .video(video)
    .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        sendonly_builder = sendonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut sendonly = sendonly_builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");

    sendonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("SendOnly クライアントの接続がタイムアウトしました");

    println!("{}: SendOnly 接続完了、RecvOnly を起動します", codec_name);

    // RecvOnly クライアントを作成・起動
    let recvonly_context = SoraConnectionContext::new().expect("RecvOnly コンテキスト作成失敗");

    let mut recvonly_builder =
        SoraTestConnection::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut recvonly = recvonly_builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");

    recvonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("RecvOnly クライアントの接続がタイムアウトしました");

    println!(
        "{}: RecvOnly 接続完了、トラック受信を待機します",
        codec_name
    );

    recvonly
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("RecvOnly クライアントがトラックを受信できませんでした");

    sendonly
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_codec_mime_type(stats, "outbound-rtp", expected_mime_type)
            },
            Duration::from_secs(15),
        )
        .await
        .unwrap_or_else(|_| panic!("{codec_name}: SendOnly stats が期待値に到達しませんでした"));
    recvonly
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
                    && verify_video_codec_mime_type(stats, "inbound-rtp", expected_mime_type)
            },
            Duration::from_secs(15),
        )
        .await
        .unwrap_or_else(|_| panic!("{codec_name}: RecvOnly stats が期待値に到達しませんでした"));

    // 切断
    sendonly
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("SendOnly の disconnect に失敗しました");
    recvonly
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("RecvOnly の disconnect に失敗しました");

    println!("{}: テスト成功: 受信と統計情報検証完了", codec_name);
}

/// 指定したコーデックで SendRecv の双方向接続テストを実行する
#[cfg(any(target_os = "macos", target_os = "ios"))]
async fn run_sendrecv_with_codec(video: Video, codec_name: &str, expected_mime_type: &str) {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id(&format!("{}-sendrecv", codec_name));

    // クライアント 1 を作成・起動
    let context1 = SoraConnectionContext::new().expect("クライアント 1 コンテキスト作成失敗");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 作成失敗");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("送信用トラック作成失敗");

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .video(video.clone())
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let mut client1 = builder1
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");

    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .unwrap_or_else(|_| panic!("{codec_name}: クライアント 1 の接続がタイムアウトしました"));

    println!(
        "{}: クライアント 1 接続完了、クライアント 2 を起動します",
        codec_name
    );

    // クライアント 2 を作成・起動
    let context2 = SoraConnectionContext::new().expect("クライアント 2 コンテキスト作成失敗");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 作成失敗");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("送信用トラック作成失敗");

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .video(video)
        .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let mut client2 = builder2
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");

    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .unwrap_or_else(|_| panic!("{codec_name}: クライアント 2 の接続がタイムアウトしました"));

    println!(
        "{}: クライアント 2 接続完了、トラック受信を待機します",
        codec_name
    );

    client1
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .unwrap_or_else(|_| {
            panic!("{codec_name}: クライアント 1 がトラックを受信できませんでした")
        });
    client2
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .unwrap_or_else(|_| {
            panic!("{codec_name}: クライアント 2 がトラックを受信できませんでした")
        });

    client1
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
                    && verify_video_codec_mime_type(stats, "outbound-rtp", expected_mime_type)
                    && verify_video_codec_mime_type(stats, "inbound-rtp", expected_mime_type)
            },
            Duration::from_secs(15),
        )
        .await
        .unwrap_or_else(|_| {
            panic!("{codec_name}: クライアント 1 stats が期待値に到達しませんでした")
        });
    client2
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
                    && verify_video_codec_mime_type(stats, "outbound-rtp", expected_mime_type)
                    && verify_video_codec_mime_type(stats, "inbound-rtp", expected_mime_type)
            },
            Duration::from_secs(15),
        )
        .await
        .unwrap_or_else(|_| {
            panic!("{codec_name}: クライアント 2 stats が期待値に到達しませんでした")
        });

    // 切断
    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の disconnect に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の disconnect に失敗しました");

    println!("{}: テスト成功: 双方向通信、統計情報検証完了", codec_name);
}

/// H.264 で SendOnly → RecvOnly の接続テスト
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[tokio::test]
async fn test_h264_sendonly_recvonly() {
    if skip_if_codec_not_fully_supported(VideoCodecType::H264, "H264") {
        return;
    }
    run_sendonly_recvonly_with_codec(Video::new_h264(None, None), "H264", "video/H264").await;
}

/// H.265 で SendOnly → RecvOnly の接続テスト
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[tokio::test]
async fn test_h265_sendonly_recvonly() {
    if skip_if_codec_not_fully_supported(VideoCodecType::H265, "H265") {
        return;
    }
    run_sendonly_recvonly_with_codec(Video::new_h265(None, None), "H265", "video/H265").await;
}

/// H.264 で SendRecv の双方向接続テスト
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[tokio::test]
async fn test_h264_sendrecv() {
    if skip_if_codec_not_fully_supported(VideoCodecType::H264, "H264") {
        return;
    }
    run_sendrecv_with_codec(Video::new_h264(None, None), "H264", "video/H264").await;
}

/// H.265 で SendRecv の双方向接続テスト
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[tokio::test]
async fn test_h265_sendrecv() {
    if skip_if_codec_not_fully_supported(VideoCodecType::H265, "H265") {
        return;
    }
    run_sendrecv_with_codec(Video::new_h265(None, None), "H265", "video/H265").await;
}
