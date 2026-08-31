use std::time::Duration;

use e2e_tests::{
    FakeAudioDeviceModule, FakeAudioDeviceModuleConfig, SoraTestConnection,
    build_metadata_with_access_token, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_audio_codec_mime_type, verify_audio_stats_field_positive,
};
use sora_sdk::{AdmConfig, Audio, Role, SoraConnectionContext, SoraConnectionContextConfig};

/// テスト用のチャンネル ID を生成する (suffix 付き)
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

/// 音声のみの SendRecv クライアントを作成・起動し、SoraTestConnection を返す。
fn connect_audio_sendrecv(urls: &[String], channel_id: &str) -> SoraTestConnection {
    // マイクが存在しない環境でも動くよう、正弦波を流すダミー AudioDeviceModule を使う。
    let fake = FakeAudioDeviceModule::new(FakeAudioDeviceModuleConfig::default());
    let config = SoraConnectionContextConfig {
        adm_config: AdmConfig::UseExternal(fake.audio_device_module()),
        ..SoraConnectionContextConfig::default()
    };
    let context = SoraConnectionContext::new_with_config(config)
        .expect("audio codec context の作成に失敗しました");
    let audio_source = context
        .create_audio_source()
        .expect("audio source の作成に失敗しました");
    let audio_track = context
        .create_audio_track(&audio_source)
        .expect("audio track の作成に失敗しました");

    let mut builder = SoraTestConnection::builder(
        context,
        urls.to_vec(),
        channel_id.to_string(),
        Role::SendRecv,
    )
    .sender_audio_track(audio_track)
    .audio(Audio::new_opus(None, None))
    .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました")
}

/// Opus (audio/opus) が実接続で送受信できることを検証する。
#[tokio::test]
async fn test_opus_sendrecv() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = test_channel_id("opus-sendrecv");

    // クライアント 1 を作成・起動
    let mut client1 = connect_audio_sendrecv(&urls, &channel_id);
    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の接続がタイムアウトしました");

    println!("クライアント 1 接続完了、クライアント 2 を起動します");

    // クライアント 2 を作成・起動
    let mut client2 = connect_audio_sendrecv(&urls, &channel_id);
    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の接続がタイムアウトしました");

    println!("クライアント 2 接続完了、音声トラック受信を待機します");

    // 両クライアントが音声トラックを受信するのを待つ
    client1
        .wait_for_track_kind("audio", Duration::from_secs(10))
        .await
        .expect("クライアント 1 が音声トラックを受信できませんでした");
    client2
        .wait_for_track_kind("audio", Duration::from_secs(10))
        .await
        .expect("クライアント 2 が音声トラックを受信できませんでした");

    // 実 Opus が送受信されていることを統計で確認する。
    client1
        .wait_stats(
            |stats| {
                verify_audio_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_audio_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_audio_codec_mime_type(stats, "outbound-rtp", "audio/opus")
                    && verify_audio_codec_mime_type(stats, "inbound-rtp", "audio/opus")
            },
            Duration::from_secs(15),
        )
        .await
        .unwrap_or_else(|_| panic!("クライアント 1 stats が期待値に到達しませんでした"));
    client2
        .wait_stats(
            |stats| {
                verify_audio_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_audio_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_audio_codec_mime_type(stats, "outbound-rtp", "audio/opus")
                    && verify_audio_codec_mime_type(stats, "inbound-rtp", "audio/opus")
            },
            Duration::from_secs(15),
        )
        .await
        .unwrap_or_else(|_| panic!("クライアント 2 stats が期待値に到達しませんでした"));

    // 切断
    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の disconnect に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の disconnect に失敗しました");

    println!("テスト成功: Opus (audio/opus) で双方向通信、統計情報検証完了");
}
