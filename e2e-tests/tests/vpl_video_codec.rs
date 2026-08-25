#![cfg(feature = "vpl")]

use std::sync::Arc;
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, verify_video_codec_mime_type, verify_video_stats_field_positive,
};
use serial_test::serial;
use shiguredo_webrtc::VideoCodecType;
use sora_sdk::{
    CodecDirection, Role, SoraConnectionContext, SoraConnectionContextConfig, Video,
    VideoCodecCapability, VideoCodecPreference, VplVideoCodecCapability,
};

fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

fn codec_label(codec_type: VideoCodecType) -> &'static str {
    match codec_type {
        VideoCodecType::H264 => "h264",
        VideoCodecType::H265 => "h265",
        VideoCodecType::Vp9 => "vp9",
        VideoCodecType::Av1 => "av1",
        _ => "unknown",
    }
}

fn codec_mime_type(codec_type: VideoCodecType) -> &'static str {
    match codec_type {
        VideoCodecType::H264 => "video/H264",
        VideoCodecType::H265 => "video/H265",
        VideoCodecType::Vp9 => "video/VP9",
        VideoCodecType::Av1 => "video/AV1",
        _ => "",
    }
}

fn video_setting(codec_type: VideoCodecType) -> Video {
    match codec_type {
        VideoCodecType::H264 => Video::new_h264(None, None),
        VideoCodecType::H265 => Video::new_h265(None, None),
        VideoCodecType::Vp9 => Video::new_vp9(None, None),
        VideoCodecType::Av1 => Video::new_av1(None, None),
        _ => panic!("未対応のコーデック種別です"),
    }
}

fn create_vpl_context() -> sora_sdk::Result<Arc<SoraConnectionContext>> {
    let capability: Box<dyn VideoCodecCapability> = Box::new(VplVideoCodecCapability::new()?);
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());

    let mut config = SoraConnectionContextConfig {
        video_codec_preference: preference,
        ..Default::default()
    };
    config.video_codec_capabilities.push(capability);

    SoraConnectionContext::new_with_config(config)
}

fn vpl_capability() -> Option<VplVideoCodecCapability> {
    match VplVideoCodecCapability::new() {
        Ok(capability) => Some(capability),
        Err(err) => {
            eprintln!("VPL の capability を利用できないためテストをスキップします: {err}");
            None
        }
    }
}

fn vpl_fully_supported_codecs() -> Option<Vec<VideoCodecType>> {
    let capability = vpl_capability()?;
    let mut codecs = Vec::new();
    for codec_type in [
        VideoCodecType::H264,
        VideoCodecType::H265,
        VideoCodecType::Vp9,
        VideoCodecType::Av1,
    ] {
        if capability.is_supported(CodecDirection::Encoder, codec_type)
            && capability.is_supported(CodecDirection::Decoder, codec_type)
        {
            codecs.push(codec_type);
        }
    }

    if codecs.is_empty() {
        eprintln!(
            "VPL にエンコーダーとデコーダーの両方をサポートするコーデックがないためテストをスキップします"
        );
        return None;
    }

    Some(codecs)
}

fn vpl_decoder_supported_only_codecs() -> Option<Vec<VideoCodecType>> {
    let capability = vpl_capability()?;
    let mut codecs = Vec::new();
    for codec_type in [
        VideoCodecType::H264,
        VideoCodecType::H265,
        VideoCodecType::Vp9,
        VideoCodecType::Av1,
    ] {
        if capability.is_supported(CodecDirection::Decoder, codec_type)
            && !capability.is_supported(CodecDirection::Encoder, codec_type)
        {
            codecs.push(codec_type);
        }
    }

    if codecs.is_empty() {
        eprintln!("VPL にデコーダーのみサポートするコーデックがないためテストをスキップします");
        return None;
    }

    Some(codecs)
}

fn vpl_encoder_supported_only_codecs() -> Option<Vec<VideoCodecType>> {
    let capability = vpl_capability()?;
    let mut codecs = Vec::new();
    for codec_type in [
        VideoCodecType::H264,
        VideoCodecType::H265,
        VideoCodecType::Vp9,
        VideoCodecType::Av1,
    ] {
        if capability.is_supported(CodecDirection::Encoder, codec_type)
            && !capability.is_supported(CodecDirection::Decoder, codec_type)
        {
            codecs.push(codec_type);
        }
    }

    if codecs.is_empty() {
        eprintln!("VPL にエンコーダーのみサポートするコーデックがないためテストをスキップします");
        return None;
    }

    Some(codecs)
}

fn default_context_supports_encoder(codec_type: VideoCodecType) -> bool {
    let config = SoraConnectionContextConfig::default();
    config
        .video_codec_capabilities
        .iter()
        .any(|capability| capability.is_supported(CodecDirection::Encoder, codec_type))
}

fn default_context_supports_decoder(codec_type: VideoCodecType) -> bool {
    let config = SoraConnectionContextConfig::default();
    config
        .video_codec_capabilities
        .iter()
        .any(|capability| capability.is_supported(CodecDirection::Decoder, codec_type))
}

async fn run_sendonly_recvonly_with_contexts(
    sendonly_context: Arc<SoraConnectionContext>,
    recvonly_context: Arc<SoraConnectionContext>,
    codec_type: VideoCodecType,
    suffix_prefix: &str,
) -> std::result::Result<(), String> {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必須です");
    let codec_label = codec_label(codec_type);
    let expected_mime_type = codec_mime_type(codec_type);
    let channel_id = test_channel_id(&format!("{suffix_prefix}-{codec_label}-sendonly-recvonly"));

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .map_err(|e| format!("FakeVideoCapturer の生成に失敗しました: {e}"))?;
    let (video_track, audio_track) = build_sender_tracks(&sendonly_context, &mut capturer)
        .map_err(|e| format!("トラックの生成に失敗しました: {e}"))?;

    let mut sendonly_builder = SoraTestConnection::builder(
        sendonly_context,
        urls.clone(),
        channel_id.clone(),
        Role::SendOnly,
    )
    .sender_video_track(video_track)
    .sender_audio_track(audio_track)
    .video(video_setting(codec_type))
    .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        sendonly_builder = sendonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut sendonly = sendonly_builder
        .connect()
        .map_err(|e| format!("sendonly クライアントの生成に失敗しました: {e}"))?;

    if sendonly
        .wait_for_connect(Duration::from_secs(10))
        .await
        .is_err()
    {
        let _ = sendonly.disconnect_and_wait(Duration::from_secs(10)).await;
        return Err("sendonly の接続がタイムアウトしました".to_string());
    }

    let mut recvonly_builder =
        SoraTestConnection::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let mut recvonly = recvonly_builder
        .connect()
        .map_err(|e| format!("recvonly クライアントの生成に失敗しました: {e}"))?;

    let result = async {
        if recvonly
            .wait_for_connect(Duration::from_secs(10))
            .await
            .is_err()
        {
            return Err("recvonly の接続がタイムアウトしました".to_string());
        }

        if recvonly
            .wait_for_video_track(Duration::from_secs(10))
            .await
            .is_err()
        {
            return Err("recvonly がトラックを受信しませんでした".to_string());
        }

        sendonly
            .wait_stats(
                |stats| {
                    verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                        && verify_video_codec_mime_type(stats, "outbound-rtp", expected_mime_type)
                },
                Duration::from_secs(15),
            )
            .await
            .map_err(|_| "sendonly の統計情報が期待した値に達しませんでした".to_string())?;
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
            .map_err(|_| "recvonly の統計情報が期待した値に達しませんでした".to_string())?;

        Ok(())
    }
    .await;

    let _ = sendonly.disconnect_and_wait(Duration::from_secs(10)).await;
    let _ = recvonly.disconnect_and_wait(Duration::from_secs(10)).await;
    result
}

async fn run_sendrecv_with_codec(codec_type: VideoCodecType) {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必須です");
    let codec_label = codec_label(codec_type);
    let expected_mime_type = codec_mime_type(codec_type);
    let channel_id = test_channel_id(&format!("vpl-{codec_label}-sendrecv"));

    let context1 = create_vpl_context().expect("client1 のコンテキスト生成に失敗しました");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("client1 の FakeVideoCapturer 生成に失敗しました");
    let (video_track1, audio_track1) = build_sender_tracks(&context1, &mut capturer1)
        .expect("client1 のトラック生成に失敗しました");

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .video(video_setting(codec_type))
            .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let mut client1 = builder1.connect().expect("client1 の構築に失敗しました");

    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("client1 の接続がタイムアウトしました");

    let context2 = create_vpl_context().expect("client2 のコンテキスト生成に失敗しました");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("client2 の FakeVideoCapturer 生成に失敗しました");
    let (video_track2, audio_track2) = build_sender_tracks(&context2, &mut capturer2)
        .expect("client2 のトラック生成に失敗しました");

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .video(video_setting(codec_type))
        .data_channel_signaling(true);

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let mut client2 = builder2.connect().expect("client2 の構築に失敗しました");

    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("client2 の接続がタイムアウトしました");
    client1
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("client1 がトラックを受信しませんでした");
    client2
        .wait_for_video_track(Duration::from_secs(10))
        .await
        .expect("client2 がトラックを受信しませんでした");

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
        .expect("client1 の統計情報が期待した値に達しませんでした");
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
        .expect("client2 の統計情報が期待した値に達しませんでした");

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("client1 の切断に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("client2 の切断に失敗しました");
}

#[tokio::test]
#[serial]
async fn test_vpl_sendonly_recvonly() {
    load_env();
    let Some(codec_types) = vpl_fully_supported_codecs() else {
        return;
    };

    let mut failures = Vec::new();
    for codec_type in codec_types {
        let sendonly_context =
            create_vpl_context().expect("sendonly のコンテキスト生成に失敗しました");
        let recvonly_context =
            create_vpl_context().expect("recvonly のコンテキスト生成に失敗しました");
        match run_sendonly_recvonly_with_contexts(
            sendonly_context,
            recvonly_context,
            codec_type,
            "vpl",
        )
        .await
        {
            Ok(()) => {
                eprintln!(
                    "vpl の sendonly/recvonly が {} で成功しました",
                    codec_label(codec_type)
                );
            }
            Err(error) => failures.push(format!("{}: {error}", codec_label(codec_type))),
        }
    }

    assert!(
        failures.is_empty(),
        "vpl の sendonly/recvonly で失敗しました:\n{}",
        failures.join("\n")
    );
}

#[tokio::test]
#[serial]
async fn test_vpl_sendrecv() {
    load_env();
    let Some(codec_types) = vpl_fully_supported_codecs() else {
        return;
    };

    for codec_type in codec_types {
        run_sendrecv_with_codec(codec_type).await;
    }
}

#[tokio::test]
#[serial]
async fn test_vpl_decoder_only_recvonly() {
    load_env();
    let Some(codec_types) = vpl_decoder_supported_only_codecs() else {
        return;
    };

    for codec_type in codec_types {
        if !default_context_supports_encoder(codec_type) {
            eprintln!(
                "既定のコンテキストが {} のエンコーダーをサポートしていないためスキップします",
                codec_label(codec_type)
            );
            continue;
        }

        let sendonly_context =
            SoraConnectionContext::new().expect("既定の sendonly コンテキストの生成に失敗しました");
        let recvonly_context =
            create_vpl_context().expect("recvonly のコンテキスト生成に失敗しました");
        run_sendonly_recvonly_with_contexts(
            sendonly_context,
            recvonly_context,
            codec_type,
            "vpl-decoder-only",
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "vpl のデコーダーのみの recvonly が {} で失敗しました: {error}",
                codec_label(codec_type)
            )
        });
    }
}

#[tokio::test]
#[serial]
async fn test_vpl_encoder_only_sendonly() {
    load_env();
    let Some(codec_types) = vpl_encoder_supported_only_codecs() else {
        return;
    };

    for codec_type in codec_types {
        if !default_context_supports_decoder(codec_type) {
            eprintln!(
                "既定のコンテキストが {} のデコーダーをサポートしていないためスキップします",
                codec_label(codec_type)
            );
            continue;
        }

        let sendonly_context =
            create_vpl_context().expect("sendonly のコンテキスト生成に失敗しました");
        let recvonly_context =
            SoraConnectionContext::new().expect("既定の recvonly コンテキストの生成に失敗しました");
        run_sendonly_recvonly_with_contexts(
            sendonly_context,
            recvonly_context,
            codec_type,
            "vpl-encoder-only",
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "vpl のエンコーダーのみの sendonly が {} で失敗しました: {error}",
                codec_label(codec_type)
            )
        });
    }
}
