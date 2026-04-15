#![cfg(feature = "nvcodec")]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, build_metadata_with_access_token,
    build_sender_tracks, generate_channel_id, load_env, secret_key, signaling_urls,
    verify_video_codec_mime_type, verify_video_stats_field_positive,
};
use serial_test::serial;
use shiguredo_webrtc::{
    VideoCodecType, VideoFrameRef, VideoSink, VideoSinkHandler, VideoSinkWants,
};
use sora_sdk::{
    CodecDirection, NvCodecVideoCodecCapability, Role, SoraClient, SoraClientContext,
    SoraClientContextConfig, Video, VideoCodecCapability, VideoCodecPreference,
};

const MIN_DECODED_VIDEO_FRAMES: usize = 3;

struct DecodeCountVideoSinkHandler {
    decoded_video_frames: Arc<AtomicUsize>,
}

impl DecodeCountVideoSinkHandler {
    fn new(decoded_video_frames: Arc<AtomicUsize>) -> Self {
        Self {
            decoded_video_frames,
        }
    }
}

impl VideoSinkHandler for DecodeCountVideoSinkHandler {
    fn on_frame(&mut self, _frame: VideoFrameRef<'_>) {
        self.decoded_video_frames.fetch_add(1, Ordering::SeqCst);
    }
}

fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

fn codec_label(codec_type: VideoCodecType) -> &'static str {
    match codec_type {
        VideoCodecType::H264 => "h264",
        VideoCodecType::H265 => "h265",
        VideoCodecType::Av1 => "av1",
        VideoCodecType::Vp8 => "vp8",
        VideoCodecType::Vp9 => "vp9",
        _ => "unknown",
    }
}

fn codec_mime_type(codec_type: VideoCodecType) -> &'static str {
    match codec_type {
        VideoCodecType::H264 => "video/H264",
        VideoCodecType::H265 => "video/H265",
        VideoCodecType::Av1 => "video/AV1",
        VideoCodecType::Vp8 => "video/VP8",
        VideoCodecType::Vp9 => "video/VP9",
        _ => "",
    }
}

fn video_setting(codec_type: VideoCodecType) -> Video {
    match codec_type {
        VideoCodecType::H264 => Video::new_h264(None, None),
        VideoCodecType::H265 => Video::new_h265(None, None),
        VideoCodecType::Av1 => Video::new_av1(None, None),
        VideoCodecType::Vp8 => Video::new_vp8(None),
        VideoCodecType::Vp9 => Video::new_vp9(None, None),
        _ => panic!("unsupported codec type"),
    }
}

fn create_nvcodec_context() -> sora_sdk::Result<Arc<SoraClientContext>> {
    let capability: Box<dyn VideoCodecCapability> = Box::new(NvCodecVideoCodecCapability::new()?);
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());

    let mut config = SoraClientContextConfig {
        video_codec_preference: preference,
        ..Default::default()
    };
    config.video_codec_capabilities.push(capability);

    SoraClientContext::new_with_config(config)
}

fn nvcodec_fully_supported_codecs() -> Option<Vec<VideoCodecType>> {
    let capability =
        NvCodecVideoCodecCapability::new().expect("Failed to create NvCodecVideoCodecCapability");
    let mut codecs = Vec::new();

    for codec_type in [
        VideoCodecType::H264,
        VideoCodecType::H265,
        VideoCodecType::Av1,
    ] {
        if capability.is_supported(CodecDirection::Encoder, codec_type)
            && capability.is_supported(CodecDirection::Decoder, codec_type)
        {
            codecs.push(codec_type);
        }
    }

    if codecs.is_empty() {
        eprintln!("NVCodec has no codec with both encoder and decoder support, skipping test");
        return None;
    }

    Some(codecs)
}

fn nvcodec_decoder_supported_only_codecs() -> Option<Vec<VideoCodecType>> {
    let capability =
        NvCodecVideoCodecCapability::new().expect("Failed to create NvCodecVideoCodecCapability");
    let mut codecs = Vec::new();

    for codec_type in [VideoCodecType::Vp8, VideoCodecType::Vp9] {
        if capability.is_supported(CodecDirection::Decoder, codec_type)
            && !capability.is_supported(CodecDirection::Encoder, codec_type)
        {
            codecs.push(codec_type);
        }
    }

    if codecs.is_empty() {
        eprintln!("NVCodec has no decoder-only codec in VP8/VP9, skipping test");
        return None;
    }

    Some(codecs)
}

fn default_context_supports_encoder(codec_type: VideoCodecType) -> bool {
    let config = SoraClientContextConfig::default();
    config
        .video_codec_capabilities
        .iter()
        .any(|capability| capability.is_supported(CodecDirection::Encoder, codec_type))
}

async fn run_sendonly_recvonly_with_contexts(
    sendonly_context: Arc<SoraClientContext>,
    recvonly_context: Arc<SoraClientContext>,
    codec_type: VideoCodecType,
    suffix_prefix: &str,
) -> std::result::Result<(), String> {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS is required");
    let codec_label = codec_label(codec_type);
    let expected_mime_type = codec_mime_type(codec_type);
    let channel_id = test_channel_id(&format!("{suffix_prefix}-{codec_label}-sendonly-recvonly"));

    let sendonly_connected = Arc::new(AtomicBool::new(false));
    let sendonly_connected_clone = sendonly_connected.clone();

    let recvonly_connected = Arc::new(AtomicBool::new(false));
    let recvonly_connected_clone = recvonly_connected.clone();
    let video_track_received = Arc::new(AtomicUsize::new(0));
    let video_track_received_clone = video_track_received.clone();
    let decoded_video_frames = Arc::new(AtomicUsize::new(0));
    let decoded_video_frames_clone = decoded_video_frames.clone();
    let recvonly_video_sinks: Arc<Mutex<HashMap<String, VideoSink>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let recvonly_video_sinks_clone = recvonly_video_sinks.clone();

    let mut capturer = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .map_err(|e| format!("failed to create FakeVideoCapturer: {e}"))?;
    let (video_track, audio_track) = build_sender_tracks(&sendonly_context, &mut capturer)
        .map_err(|e| format!("failed to build tracks: {e}"))?;

    let mut sendonly_builder = SoraClient::builder(
        sendonly_context,
        urls.clone(),
        channel_id.clone(),
        Role::SendOnly,
    )
    .sender_video_track(video_track)
    .sender_audio_track(audio_track)
    .video(video_setting(codec_type))
    .data_channel_signaling(true)
    .on_notify(move |_| {
        sendonly_connected_clone.store(true, Ordering::SeqCst);
    });

    if let Some(token) = secret_key() {
        sendonly_builder = sendonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (sendonly_client, sendonly_handle) = sendonly_builder
        .build()
        .map_err(|e| format!("failed to build sendonly client: {e}"))?;
    let sendonly_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), sendonly_client.run()).await;
    });

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
        sendonly_task.abort();
        return Err("sendonly connection timed out".to_string());
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    let mut recvonly_builder =
        SoraClient::builder(recvonly_context, urls, channel_id, Role::RecvOnly)
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

                let track_id = match track.id() {
                    Ok(id) => id,
                    Err(_) => return,
                };
                let mut video_track = track.cast_to_video_track();
                let mut sinks = match recvonly_video_sinks_clone.lock() {
                    Ok(sinks) => sinks,
                    Err(_) => return,
                };
                if sinks.contains_key(&track_id) {
                    return;
                }

                let sink = VideoSink::new_with_handler(Box::new(DecodeCountVideoSinkHandler::new(
                    decoded_video_frames_clone.clone(),
                )));
                let wants = VideoSinkWants::new();
                video_track.add_or_update_sink(&sink, &wants);
                sinks.insert(track_id, sink);
                video_track_received_clone.fetch_add(1, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        recvonly_builder = recvonly_builder.metadata(build_metadata_with_access_token(&token));
    }

    let (recvonly_client, recvonly_handle) = recvonly_builder
        .build()
        .map_err(|e| format!("failed to build recvonly client: {e}"))?;
    let recvonly_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), recvonly_client.run()).await;
    });

    let result = async {
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
            return Err("recvonly connection timed out".to_string());
        }

        let video_track_wait = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if video_track_received.load(Ordering::SeqCst) > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        if video_track_wait.is_err() || video_track_received.load(Ordering::SeqCst) == 0 {
            return Err("recvonly did not receive video tracks".to_string());
        }

        let decoded_wait = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if decoded_video_frames.load(Ordering::SeqCst) >= MIN_DECODED_VIDEO_FRAMES {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        if decoded_wait.is_err() || decoded_video_frames.load(Ordering::SeqCst) < MIN_DECODED_VIDEO_FRAMES {
            return Err(format!(
                "recvonly did not decode enough video frames: decoded={}, required={MIN_DECODED_VIDEO_FRAMES}",
                decoded_video_frames.load(Ordering::SeqCst)
            ));
        }

        tokio::time::sleep(Duration::from_secs(5)).await;

        let sendonly_stats = sendonly_handle
            .get_stats()
            .await
            .map_err(|e| format!("failed to get sendonly stats: {e}"))?;
        if !verify_video_stats_field_positive(&sendonly_stats, "outbound-rtp", "packetsSent") {
            return Err("sendonly must send video packets".to_string());
        }
        if !verify_video_codec_mime_type(&sendonly_stats, "outbound-rtp", expected_mime_type) {
            return Err(format!(
                "sendonly outbound codec must match: expected={expected_mime_type}"
            ));
        }

        let recvonly_stats = recvonly_handle
            .get_stats()
            .await
            .map_err(|e| format!("failed to get recvonly stats: {e}"))?;
        if !verify_video_stats_field_positive(&recvonly_stats, "inbound-rtp", "packetsReceived") {
            return Err("recvonly must receive video packets".to_string());
        }
        if !verify_video_codec_mime_type(&recvonly_stats, "inbound-rtp", expected_mime_type) {
            return Err(format!(
                "recvonly inbound codec must match: expected={expected_mime_type}"
            ));
        }

        Ok(())
    }
    .await;

    let _ = sendonly_handle.disconnect().await;
    let _ = recvonly_handle.disconnect().await;

    sendonly_task.abort();
    recvonly_task.abort();
    result
}

async fn run_sendrecv_with_codec(codec_type: VideoCodecType) {
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS is required");
    let codec_label = codec_label(codec_type);
    let expected_mime_type = codec_mime_type(codec_type);
    let channel_id = test_channel_id(&format!("nvcodec-{codec_label}-sendrecv"));

    let client1_connected = Arc::new(AtomicBool::new(false));
    let client1_connected_clone = client1_connected.clone();
    let client1_track_received = Arc::new(AtomicUsize::new(0));
    let client1_track_received_clone = client1_track_received.clone();

    let client2_connected = Arc::new(AtomicBool::new(false));
    let client2_connected_clone = client2_connected.clone();
    let client2_track_received = Arc::new(AtomicUsize::new(0));
    let client2_track_received_clone = client2_track_received.clone();

    let context1 = create_nvcodec_context().expect("failed to create client1 context");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("failed to create FakeVideoCapturer for client1");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("failed to build client1 tracks");

    let mut builder1 =
        SoraClient::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .video(video_setting(codec_type))
            .data_channel_signaling(true)
            .on_notify(move |_| {
                client1_connected_clone.store(true, Ordering::SeqCst);
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
                client1_track_received_clone.fetch_add(1, Ordering::SeqCst);
            });

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let (client1, handle1) = builder1.build().expect("failed to build client1");
    let client1_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), client1.run()).await;
    });

    let client1_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if client1_connected.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(client1_wait.is_ok(), "client1 connection timed out");

    tokio::time::sleep(Duration::from_secs(1)).await;

    let context2 = create_nvcodec_context().expect("failed to create client2 context");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("failed to create FakeVideoCapturer for client2");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("failed to build client2 tracks");

    let mut builder2 = SoraClient::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .video(video_setting(codec_type))
        .data_channel_signaling(true)
        .on_notify(move |_| {
            client2_connected_clone.store(true, Ordering::SeqCst);
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
            client2_track_received_clone.fetch_add(1, Ordering::SeqCst);
        });

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let (client2, handle2) = builder2.build().expect("failed to build client2");
    let client2_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(30), client2.run()).await;
    });

    let client2_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if client2_connected.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(client2_wait.is_ok(), "client2 connection timed out");

    let track_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if client1_track_received.load(Ordering::SeqCst) > 0
                && client2_track_received.load(Ordering::SeqCst) > 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(
        track_wait.is_ok()
            && client1_track_received.load(Ordering::SeqCst) > 0
            && client2_track_received.load(Ordering::SeqCst) > 0,
        "both clients must receive tracks"
    );

    tokio::time::sleep(Duration::from_secs(5)).await;

    let stats1 = handle1
        .get_stats()
        .await
        .expect("failed to get client1 stats");
    let stats2 = handle2
        .get_stats()
        .await
        .expect("failed to get client2 stats");

    assert!(
        verify_video_stats_field_positive(&stats1, "outbound-rtp", "packetsSent"),
        "client1 outbound video packets must be sent"
    );
    assert!(
        verify_video_stats_field_positive(&stats1, "inbound-rtp", "packetsReceived"),
        "client1 inbound video packets must be received"
    );
    assert!(
        verify_video_stats_field_positive(&stats2, "outbound-rtp", "packetsSent"),
        "client2 outbound video packets must be sent"
    );
    assert!(
        verify_video_stats_field_positive(&stats2, "inbound-rtp", "packetsReceived"),
        "client2 inbound video packets must be received"
    );

    assert!(
        verify_video_codec_mime_type(&stats1, "outbound-rtp", expected_mime_type),
        "client1 outbound codec must match"
    );
    assert!(
        verify_video_codec_mime_type(&stats1, "inbound-rtp", expected_mime_type),
        "client1 inbound codec must match"
    );
    assert!(
        verify_video_codec_mime_type(&stats2, "outbound-rtp", expected_mime_type),
        "client2 outbound codec must match"
    );
    assert!(
        verify_video_codec_mime_type(&stats2, "inbound-rtp", expected_mime_type),
        "client2 inbound codec must match"
    );

    handle1
        .disconnect()
        .await
        .expect("failed to disconnect client1");
    handle2
        .disconnect()
        .await
        .expect("failed to disconnect client2");

    client1_task.abort();
    client2_task.abort();
}

#[tokio::test]
#[serial]
async fn test_nvcodec_sendonly_recvonly() {
    load_env();
    let Some(codec_types) = nvcodec_fully_supported_codecs() else {
        return;
    };

    let mut failures = Vec::new();
    for codec_type in codec_types {
        let sendonly_context = create_nvcodec_context().expect("failed to create sendonly context");
        let recvonly_context = create_nvcodec_context().expect("failed to create recvonly context");
        match run_sendonly_recvonly_with_contexts(
            sendonly_context,
            recvonly_context,
            codec_type,
            "nvcodec",
        )
        .await
        {
            Ok(()) => {
                eprintln!(
                    "nvcodec sendonly/recvonly passed for {}",
                    codec_label(codec_type)
                );
            }
            Err(error) => failures.push(format!("{}: {error}", codec_label(codec_type))),
        }
    }

    assert!(
        failures.is_empty(),
        "nvcodec sendonly/recvonly failures:\n{}",
        failures.join("\n")
    );
}

#[tokio::test]
#[serial]
async fn test_nvcodec_sendrecv() {
    load_env();
    let Some(codec_types) = nvcodec_fully_supported_codecs() else {
        return;
    };

    for codec_type in codec_types {
        run_sendrecv_with_codec(codec_type).await;
    }
}

#[tokio::test]
#[serial]
async fn test_nvcodec_decoder_only_recvonly() {
    load_env();
    let Some(codec_types) = nvcodec_decoder_supported_only_codecs() else {
        return;
    };

    for codec_type in codec_types {
        if !default_context_supports_encoder(codec_type) {
            eprintln!(
                "default context does not support encoder for {}, skipping",
                codec_label(codec_type)
            );
            continue;
        }

        let sendonly_context =
            SoraClientContext::new().expect("failed to create default sendonly context");
        let recvonly_context = create_nvcodec_context().expect("failed to create recvonly context");
        run_sendonly_recvonly_with_contexts(
            sendonly_context,
            recvonly_context,
            codec_type,
            "nvcodec-decoder-only",
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "nvcodec decoder-only recvonly failed for {}: {error}",
                codec_label(codec_type)
            )
        });
    }
}
