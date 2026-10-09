use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    recording_video_codec_capability, secret_key, signaling_urls,
};
use shiguredo_webrtc::{EnvironmentFactory, FieldTrials};
use sora_sdk::{
    InternalVideoCodecCapability, Role, SoraConnectionContext, SoraConnectionContextConfig,
    VideoCodecPreference,
};

/// フィールドトライアルのキー。エンコーダー生成時に EnvironmentRef で有効になっていることを検証する。
const FIELD_TRIAL_KEY: &str = "WebRTC-Video-PerSsrcKeyframes";

#[tokio::test]
async fn test_environment_with_field_trials_reaches_video_encoder() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();

    // 利用側がフィールドトライアル付きの Environment を生成する
    let mut environment_factory = EnvironmentFactory::new();
    environment_factory.set_field_trials(
        FieldTrials::new("WebRTC-Video-PerSsrcKeyframes/Enabled/")
            .expect("フィールドトライアル文字列のパースに失敗しました"),
    );
    let environment = environment_factory.create();

    // 内部 capability に委譲しつつ、エンコーダー生成時の Environment を記録する
    let (capability, recorded_environments) =
        recording_video_codec_capability(Box::new(InternalVideoCodecCapability::new()));
    let config = SoraConnectionContextConfig {
        environment: Some(environment),
        video_codec_preference: VideoCodecPreference::new_from_capability(capability.as_ref()),
        video_codec_capabilities: vec![capability],
        ..SoraConnectionContextConfig::default()
    };
    let context =
        SoraConnectionContext::new_with_config(config).expect("コンテキストの生成に失敗しました");

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 作成失敗");
    let (video_track, audio_track) =
        build_sender_tracks(&context, &mut capturer).expect("送信用トラック作成失敗");

    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::SendOnly)
        .sender_video_track(video_track)
        .sender_audio_track(audio_track)
        .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut connection = builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");
    connection
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("接続がタイムアウトしました");

    // エンコーダー生成は libwebrtc のスレッドで行われ、記録は接続確立までに完了する
    let mut recorded = false;
    for environment in recorded_environments.try_iter() {
        recorded = true;
        assert!(
            environment.field_trials().is_enabled(FIELD_TRIAL_KEY),
            "エンコーダー生成時の EnvironmentRef でフィールドトライアルが有効でなければなりません",
        );
    }
    assert!(
        recorded,
        "エンコーダー生成時に EnvironmentRef が渡されなければなりません",
    );

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}
