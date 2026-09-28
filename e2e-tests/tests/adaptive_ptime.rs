use std::time::Duration;

use e2e_tests::{
    FakeAudioDeviceModule, FakeAudioDeviceModuleConfig, SoraTestConnection,
    build_metadata_with_access_token, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_audio_stats_field_positive,
};
use sora_sdk::{AdmConfig, Role, SoraConnectionContext, SoraConnectionContextConfig};

/// adaptive ptime を指定した音声送信の接続を作成・起動する。
///
/// マイクが存在しない環境でも動くよう、正弦波を流すダミー AudioDeviceModule を使う。
/// 戻り値のダミー AudioDeviceModule は接続が使うため、呼び出し側で保持する。
fn connect_audio_sendrecv(
    urls: &[String],
    channel_id: &str,
    role: Role,
    adaptive_ptime: bool,
) -> (SoraTestConnection, FakeAudioDeviceModule) {
    let fake = FakeAudioDeviceModule::new(FakeAudioDeviceModuleConfig::default());
    let config = SoraConnectionContextConfig {
        adm_config: AdmConfig::UseExternal(fake.audio_device_module()),
        ..SoraConnectionContextConfig::default()
    };
    let context = SoraConnectionContext::new_with_config(config)
        .expect("adaptive ptime 用の context 作成に失敗しました");
    let audio_source = context
        .create_audio_source()
        .expect("audio source の作成に失敗しました");
    let audio_track = context
        .create_audio_track(&audio_source)
        .expect("audio track の作成に失敗しました");

    let mut builder =
        SoraTestConnection::builder(context, urls.to_vec(), channel_id.to_string(), role)
            .sender_audio_track(audio_track)
            .adaptive_ptime(adaptive_ptime);

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let connection = builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");
    (connection, fake)
}

/// 音声を送信する role で adaptive ptime を有効にしても、音声の送信が継続することを確認する。
///
/// adaptive ptime は SDP に現れないため、Sora とのネゴシエーションが成立して
/// 送信が継続することまでを確認する。audio sender の RTP パラメータへの反映は
/// SDK の単体テストで確認する。
#[tokio::test]
async fn test_sendrecv_adaptive_ptime() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();

    let (mut connection, _fake) = connect_audio_sendrecv(&urls, &channel_id, Role::SendRecv, true);
    connection
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("接続に失敗しました");

    // adaptive ptime を有効にしても音声の送信そのものは継続する。
    connection
        .wait_stats(
            |stats| verify_audio_stats_field_positive(stats, "outbound-rtp", "packetsSent"),
            Duration::from_secs(10),
        )
        .await
        .expect("音声の送信が継続しませんでした");

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}

/// 音声を送信しない role では adaptive ptime がスキップされ、接続できることを確認する。
///
/// recvonly は audio sender を持たないため、値を設定しても何も起きずに接続が成立する。
#[tokio::test]
async fn test_recvonly_adaptive_ptime() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();

    let (mut connection, _fake) = connect_audio_sendrecv(&urls, &channel_id, Role::RecvOnly, true);
    connection
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("接続に失敗しました");
    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}
