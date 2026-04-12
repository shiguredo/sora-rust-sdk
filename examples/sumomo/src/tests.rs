use super::*;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use sora_sdk::CodecDirection;
use sora_sdk::Role;

fn test_args(
    video_codec_implementation: VideoCodecImplementationSelections,
    openh264_path: Option<&str>,
) -> Args {
    Args {
        signaling_urls: vec!["wss://example.com/signaling".to_string()],
        channel_id: "test-channel".to_string(),
        role: Role::SendOnly,
        audio: None,
        video: None,
        video_codec_type: None,
        video_codec_implementation,
        video_bit_rate: None,
        input_mp4: None,
        openh264_path: openh264_path.map(ToString::to_string),
        video_codec_list: false,
        data_channel_signaling: None,
        ignore_disconnect_websocket: None,
        simulcast: None,
        insecure: false,
        client_cert: None,
        client_key: None,
        ca_cert: None,
        duration: None,
        turn_tls_insecure: false,
        turn_tls_ca_cert: None,
        use_libcamera: false,
        libcamera_controls: Vec::new(),
        #[cfg(feature = "raw-player")]
        use_raw_player: false,
        #[cfg(feature = "media-device")]
        video_input_device: None,
        #[cfg(feature = "media-device")]
        audio_input_device: None,
        #[cfg(feature = "media-device")]
        list_devices: false,
    }
}

fn capability_names(config: &SoraClientContextConfig) -> Vec<String> {
    config
        .video_codec_capabilities
        .iter()
        .map(|capability| capability.get_implementation().name().to_string())
        .collect()
}

fn make_raw_args(values: &[&str]) -> noargs::RawArgs {
    noargs::RawArgs::new(values.iter().map(|v| v.to_string()))
}

#[test]
fn parse_video_codec_implementation_auto() {
    let parsed = VideoCodecImplementationSelections::parse("auto")
        .expect("auto must be parsed successfully");
    assert_eq!(parsed, VideoCodecImplementationSelections::Auto);
}

#[test]
fn parse_video_codec_implementation_multiple() {
    let parsed = VideoCodecImplementationSelections::parse("internal-apple,internal")
        .expect("manual list must be parsed successfully");
    assert_eq!(
        parsed,
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::InternalApple,
            VideoCodecImplementationSelection::Internal,
        ])
    );
}

#[test]
fn parse_video_codec_implementation_rejects_auto_mix() {
    let err = VideoCodecImplementationSelections::parse("auto,amf")
        .expect_err("auto mixed list must fail");
    assert_eq!(
        err,
        "video-codec-implementation auto cannot be combined with other implementations"
    );
}

#[test]
fn parse_video_codec_implementation_rejects_duplicate() {
    let err =
        VideoCodecImplementationSelections::parse("amf,amf").expect_err("duplicate list must fail");
    assert_eq!(
        err,
        "video-codec-implementation must not contain duplicate implementations"
    );
}

#[test]
fn parse_video_codec_implementation_rejects_empty_entry() {
    let err = VideoCodecImplementationSelections::parse("amf,,nvcodec")
        .expect_err("empty entry must fail");
    assert_eq!(
        err,
        "video-codec-implementation must not contain empty entries"
    );
}

#[test]
fn parse_video_codec_implementation_rejects_unknown_value() {
    let err =
        VideoCodecImplementationSelections::parse("unknown").expect_err("unknown value must fail");
    assert_eq!(
        err,
        "video-codec-implementation must be auto/internal/internal-apple/amf/nvcodec/vpl/openh264"
    );
}

#[test]
fn parse_video_codec_implementation_accepts_vpl() {
    let parsed =
        VideoCodecImplementationSelections::parse("vpl").expect("vpl must be parsed successfully");
    assert_eq!(
        parsed,
        VideoCodecImplementationSelections::Manual(vec![VideoCodecImplementationSelection::Vpl,])
    );
}

#[test]
fn parse_args_accepts_multiple_libcamera_controls() {
    let raw_args = make_raw_args(&[
        "sumomo",
        "--signaling-url",
        "wss://example.com/signaling",
        "--channel-id",
        "test-channel",
        "--role",
        "sendonly",
        "--libcamera",
        "--libcamera-control",
        "Brightness=0.2",
        "--libcamera-control",
        "Contrast=1.5",
    ]);
    let args = crate::args::parse_args_from_raw_args(raw_args)
        .expect("multiple libcamera controls must be parsed successfully");
    assert!(args.use_libcamera);
    assert_eq!(
        args.libcamera_controls,
        vec![
            ("Brightness".to_string(), "0.2".to_string()),
            ("Contrast".to_string(), "1.5".to_string()),
        ]
    );
}

#[test]
fn parse_args_rejects_invalid_libcamera_control_format() {
    let raw_args = make_raw_args(&[
        "sumomo",
        "--signaling-url",
        "wss://example.com/signaling",
        "--channel-id",
        "test-channel",
        "--role",
        "sendonly",
        "--libcamera",
        "--libcamera-control",
        "Brightness",
    ]);
    let result = crate::args::parse_args_from_raw_args(raw_args);
    assert!(
        result.is_err(),
        "invalid libcamera control format must fail"
    );
    let err = result.err().expect("error must exist");
    assert!(
        err.to_string()
            .contains("--libcamera-control must be KEY=VALUE")
    );
}

#[test]
fn validate_args_accepts_openh264_with_path() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Openh264,
        ]),
        Some("/tmp/libopenh264.so"),
    );
    assert!(validate_args(&args).is_ok());
}

#[test]
fn validate_args_rejects_openh264_without_path() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Openh264,
        ]),
        None,
    );
    let err = validate_args(&args).expect_err("missing openh264 path must fail");
    assert!(
        err.to_string()
            .contains("--video-codec-implementation openh264 requires --openh264-path")
    );
}

#[test]
fn validate_args_rejects_openh264_path_without_openh264() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
        ]),
        Some("/tmp/libopenh264.so"),
    );
    let err = validate_args(&args).expect_err("unexpected openh264 path must fail");
    assert!(
        err.to_string()
            .contains("--openh264-path requires --video-codec-implementation to include openh264")
    );
}

#[test]
fn validate_args_rejects_openh264_path_with_auto() {
    let args = test_args(
        VideoCodecImplementationSelections::Auto,
        Some("/tmp/libopenh264.so"),
    );
    let err = validate_args(&args).expect_err("auto with openh264 path must fail");
    assert!(
        err.to_string()
            .contains("--openh264-path requires --video-codec-implementation to include openh264")
    );
}

#[test]
fn validate_args_rejects_libcamera_control_without_libcamera() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    args.libcamera_controls = vec![("Brightness".to_string(), "0.2".to_string())];
    let err = validate_args(&args).expect_err("libcamera control without libcamera must fail");
    assert!(
        err.to_string()
            .contains("--libcamera-control requires --libcamera")
    );
}

#[cfg(feature = "media-device")]
#[test]
fn validate_args_rejects_libcamera_with_video_input_device() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    args.use_libcamera = true;
    args.video_input_device = Some("/dev/video0".to_string());
    let err = validate_args(&args).expect_err("libcamera with video-input-device must fail");
    assert!(
        err.to_string()
            .contains("--libcamera and --video-input-device cannot be used together")
    );
}

#[cfg(not(feature = "libcamera"))]
#[test]
fn validate_args_rejects_libcamera_when_feature_is_disabled() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    args.use_libcamera = true;
    let err = validate_args(&args).expect_err("libcamera must fail when feature is disabled");
    assert!(err.to_string().contains(
        "libcamera is not enabled in this build. Rebuild sumomo with --features libcamera"
    ));
}

#[cfg(not(feature = "vpl"))]
#[test]
fn validate_args_rejects_vpl_when_feature_is_disabled() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![VideoCodecImplementationSelection::Vpl]),
        None,
    );
    let err = validate_args(&args).expect_err("vpl must fail when feature is disabled");
    assert!(
        err.to_string()
            .contains("VPL is not enabled in this build. Rebuild sumomo with --features vpl")
    );
}

#[serial_test::serial]
#[test]
fn build_context_config_auto_uses_default_capabilities() {
    let config = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        None,
        None,
        VideoCodecImplementationSelections::Auto,
    )
    .expect("auto config must be built");

    let default_config = SoraClientContextConfig::default();
    assert_eq!(capability_names(&config), capability_names(&default_config));
    assert_eq!(
        config.video_codec_preference,
        default_config.video_codec_preference
    );
}

#[serial_test::serial]
#[test]
fn build_context_config_manual_internal_only() {
    let config = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        None,
        None,
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
        ]),
    )
    .expect("manual config must be built");
    let names = capability_names(&config);
    assert_eq!(names, vec!["internal".to_string()]);
    assert!(
        config
            .video_codec_preference
            .codecs()
            .iter()
            .any(|codec| codec.implementation().name() == "internal")
    );
}

#[serial_test::serial]
#[test]
fn collect_video_codec_list_report_marks_internal_selected_in_auto() {
    let args = test_args(VideoCodecImplementationSelections::Auto, None);
    let report = collect_video_codec_list_report(&args);
    let internal = report
        .capabilities
        .iter()
        .find(|capability| capability.implementation == "internal")
        .expect("internal capability must exist");
    assert!(internal.selected);
    assert!(internal.available);
}

#[serial_test::serial]
#[test]
fn collect_video_codec_list_report_marks_openh264_reason_without_path() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Openh264,
        ]),
        None,
    );
    let report = collect_video_codec_list_report(&args);
    let openh264 = report
        .capabilities
        .iter()
        .find(|capability| capability.implementation == "openh264")
        .expect("openh264 capability must exist");
    assert!(openh264.selected);
    assert!(!openh264.available);
    assert!(
        openh264
            .unavailable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("--openh264-path is not specified"))
    );
}

#[serial_test::serial]
#[test]
fn collect_video_codec_list_report_preference_uses_selected_internal() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
        ]),
        None,
    );
    let report = collect_video_codec_list_report(&args);
    assert!(report.preference.iter().any(|preference| {
        preference.encoder.as_deref() == Some("internal")
            || preference.decoder.as_deref() == Some("internal")
    }));
}

#[serial_test::serial]
#[test]
fn build_video_codec_list_report_text_uses_single_line_format() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
        ]),
        None,
    );
    let report = collect_video_codec_list_report(&args);
    let text = build_video_codec_list_report_text(&report, false);
    assert!(text.contains("Video codec capability:"));
    assert!(text.contains("- [x] internal"));
    assert!(text.contains(":unavailable:"));
    assert!(text.contains("Video codec preference:"));
    assert!(text.contains("- vp8"));
    assert!(text.contains("- h264"));
    assert!(text.contains("- h265"));
    assert!(text.contains("enc: (none)"));
}

#[serial_test::serial]
#[test]
fn build_video_codec_list_report_text_applies_ansi_styles() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
        ]),
        None,
    );
    let report = collect_video_codec_list_report(&args);
    let text = build_video_codec_list_report_text(&report, true);
    assert!(text.contains("\x1b[1m- [x] internal"));
    assert!(text.contains("\x1b[2m- [ ] openh264"));
    assert!(text.contains(":unavailable: --openh264-path is not specified"));
    assert!(text.contains("\x1b[2m(none)\x1b[0m"));
}

#[test]
fn build_video_codec_list_report_text_aligns_preference_with_ansi_none() {
    let report = VideoCodecListReport {
        capabilities: vec![],
        preference: vec![
            VideoCodecPreferenceReport {
                codec: "vp8".to_string(),
                encoder: None,
                decoder: Some("nvcodec".to_string()),
            },
            VideoCodecPreferenceReport {
                codec: "av1".to_string(),
                encoder: Some("nvcodec".to_string()),
                decoder: Some("nvcodec".to_string()),
            },
        ],
    };
    let text = build_video_codec_list_report_text(&report, true);
    let plain = text.replace("\x1b[2m", "").replace("\x1b[0m", "");
    assert!(plain.contains("- vp8  enc: (none)  dec: nvcodec"));
    assert!(plain.contains("- av1  enc: nvcodec dec: nvcodec"));
}

#[test]
fn build_video_codec_list_report_text_aligns_capability_by_max_width() {
    let report = VideoCodecListReport {
        capabilities: vec![
            VideoCodecCapabilityReport {
                implementation: "a".to_string(),
                selected: false,
                available: true,
                unavailable_reason: None,
                encoder_codecs: vec!["vp8".to_string()],
                decoder_codecs: vec!["vp8".to_string()],
            },
            VideoCodecCapabilityReport {
                implementation: "bbbb".to_string(),
                selected: false,
                available: true,
                unavailable_reason: None,
                encoder_codecs: vec!["vp8".to_string()],
                decoder_codecs: vec!["vp8".to_string()],
            },
        ],
        preference: vec![],
    };
    let text = build_video_codec_list_report_text(&report, false);
    assert!(text.contains("- [ ] a    enc(vp8) dec(vp8)"));
    assert!(text.contains("- [ ] bbbb enc(vp8) dec(vp8)"));
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[serial_test::serial]
#[test]
fn build_context_config_rejects_internal_apple_on_unsupported_platform() {
    let result = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        None,
        None,
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::InternalApple,
        ]),
    );
    match result {
        Ok(_) => panic!("internal-apple must fail on unsupported platform"),
        Err(err) => {
            assert!(
                err.to_string()
                    .contains("internal-apple is not supported on this platform")
            );
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[serial_test::serial]
#[test]
fn build_context_config_manual_order_prefers_later_selection_on_apple() {
    let result = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        None,
        None,
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
            VideoCodecImplementationSelection::InternalApple,
        ]),
    );
    match result {
        Ok(config) => {
            let preference = config
                .video_codec_preference
                .find(
                    CodecDirection::Encoder,
                    shiguredo_webrtc::VideoCodecType::H264,
                )
                .expect("h264 encoder preference must exist");
            assert_eq!(preference.implementation().name(), "internal-apple");
        }
        Err(err) => {
            assert!(
                err.to_string()
                    .contains("internal-apple is not available on this device")
            );
        }
    }
}
