use super::*;
use sora_sdk::{CodecDirection, Role};
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
        metadata: None,
        turn_tls_insecure: false,
        turn_tls_ca_cert: None,
        use_libcamera: false,
        use_libcamera_native: false,
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

fn capability_names(config: &SoraConnectionContextConfig) -> Vec<String> {
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
    let parsed =
        VideoCodecImplementationSelections::parse("auto").expect("auto はパースに成功するはずです");
    assert_eq!(parsed, VideoCodecImplementationSelections::Auto);
}

#[test]
fn parse_video_codec_implementation_multiple() {
    let parsed = VideoCodecImplementationSelections::parse("internal-apple,internal")
        .expect("手動リストはパースに成功するはずです");
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
        .expect_err("auto 混在リストは失敗するはずです");
    assert_eq!(
        err,
        "video-codec-implementation auto cannot be combined with other implementations"
    );
}

#[test]
fn parse_video_codec_implementation_rejects_duplicate() {
    let err = VideoCodecImplementationSelections::parse("amf,amf")
        .expect_err("重複リストは失敗するはずです");
    assert_eq!(
        err,
        "video-codec-implementation must not contain duplicate implementations"
    );
}

#[test]
fn parse_video_codec_implementation_rejects_empty_entry() {
    let err = VideoCodecImplementationSelections::parse("amf,,nvcodec")
        .expect_err("空エントリは失敗するはずです");
    assert_eq!(
        err,
        "video-codec-implementation must not contain empty entries"
    );
}

#[test]
fn parse_video_codec_implementation_rejects_unknown_value() {
    let err = VideoCodecImplementationSelections::parse("unknown")
        .expect_err("未知の値は失敗するはずです");
    assert_eq!(
        err,
        "video-codec-implementation must be auto/internal/internal-apple/amf/nvcodec/vpl/v4l2/openh264"
    );
}

#[test]
fn parse_video_codec_implementation_accepts_vpl() {
    let parsed =
        VideoCodecImplementationSelections::parse("vpl").expect("vpl はパースに成功するはずです");
    assert_eq!(
        parsed,
        VideoCodecImplementationSelections::Manual(vec![VideoCodecImplementationSelection::Vpl,])
    );
}

#[test]
fn parse_video_codec_implementation_accepts_v4l2() {
    let parsed =
        VideoCodecImplementationSelections::parse("v4l2").expect("v4l2 はパースに成功するはずです");
    assert_eq!(
        parsed,
        VideoCodecImplementationSelections::Manual(vec![VideoCodecImplementationSelection::V4l2,])
    );
}

#[test]
fn parse_args_accepts_mp4_with_video_codec_implementation_manual() {
    let raw_args = make_raw_args(&[
        "sumomo",
        "--signaling-url",
        "wss://example.com/signaling",
        "--channel-id",
        "test-channel",
        "--role",
        "sendonly",
        "--input-mp4",
        "/tmp/video.mp4",
        "--video-codec-implementation",
        "internal",
    ]);
    let args = crate::args::parse_args(raw_args)
        .expect("MP4 と codec 実装の併用はパースに成功するはずです");
    assert_eq!(args.input_mp4.as_deref(), Some("/tmp/video.mp4"));
    assert_eq!(
        args.video_codec_implementation,
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
        ])
    );
}

#[test]
fn parse_args_accepts_mp4_with_video_codec_implementation_auto() {
    let raw_args = make_raw_args(&[
        "sumomo",
        "--signaling-url",
        "wss://example.com/signaling",
        "--channel-id",
        "test-channel",
        "--role",
        "sendonly",
        "--input-mp4",
        "/tmp/video.mp4",
        "--video-codec-implementation",
        "auto",
    ]);
    let args = crate::args::parse_args(raw_args)
        .expect("MP4 と auto 明示の併用はパースに成功するはずです");
    assert_eq!(args.input_mp4.as_deref(), Some("/tmp/video.mp4"));
    assert_eq!(
        args.video_codec_implementation,
        VideoCodecImplementationSelections::Auto
    );
}

#[test]
fn parse_args_accepts_mp4_without_video_codec_implementation() {
    let raw_args = make_raw_args(&[
        "sumomo",
        "--signaling-url",
        "wss://example.com/signaling",
        "--channel-id",
        "test-channel",
        "--role",
        "sendonly",
        "--input-mp4",
        "/tmp/video.mp4",
    ]);
    let args = crate::args::parse_args(raw_args).expect("MP4 のみの指定はパースに成功するはずです");
    assert_eq!(args.input_mp4.as_deref(), Some("/tmp/video.mp4"));
    assert_eq!(
        args.video_codec_implementation,
        VideoCodecImplementationSelections::Auto
    );
    assert_eq!(args.video_codec_type, None);
}

#[test]
fn validate_args_rejects_mp4_with_video_codec_type() {
    let raw_args = make_raw_args(&[
        "sumomo",
        "--signaling-url",
        "wss://example.com/signaling",
        "--channel-id",
        "test-channel",
        "--role",
        "sendonly",
        "--input-mp4",
        "/tmp/video.mp4",
        "--video-codec-type",
        "h264",
    ]);
    let args =
        crate::args::parse_args(raw_args).expect("MP4 と codec type のパースは成功するはずです");
    let err = validate_args(&args).expect_err("MP4 と codec type の併用は失敗するはずです");
    let message = err.to_string();
    assert!(
        message.contains("--input-mp4 and --video-codec-type cannot be used together"),
        "エラーメッセージが期待と異なります: {message}"
    );
}

#[test]
fn video_from_codec_type_builds_codec_specific_video() {
    let bit_rate = Some(30000);
    assert_eq!(
        video_from_codec_type(shiguredo_webrtc::VideoCodecType::Vp8, bit_rate)
            .expect("vp8 は Video を生成できるはずです"),
        sora_sdk::Video::new_vp8(bit_rate)
    );
    assert_eq!(
        video_from_codec_type(shiguredo_webrtc::VideoCodecType::Vp9, bit_rate)
            .expect("vp9 は Video を生成できるはずです"),
        sora_sdk::Video::new_vp9(bit_rate, None)
    );
    assert_eq!(
        video_from_codec_type(shiguredo_webrtc::VideoCodecType::Av1, bit_rate)
            .expect("av1 は Video を生成できるはずです"),
        sora_sdk::Video::new_av1(bit_rate, None)
    );
    assert_eq!(
        video_from_codec_type(shiguredo_webrtc::VideoCodecType::H264, bit_rate)
            .expect("h264 は Video を生成できるはずです"),
        sora_sdk::Video::new_h264(bit_rate, None)
    );
    assert_eq!(
        video_from_codec_type(shiguredo_webrtc::VideoCodecType::H265, bit_rate)
            .expect("h265 は Video を生成できるはずです"),
        sora_sdk::Video::new_h265(bit_rate, None)
    );
}

#[test]
fn video_from_codec_type_rejects_unknown_codec() {
    let err = video_from_codec_type(shiguredo_webrtc::VideoCodecType::Generic, None)
        .expect_err("Generic はエラーになるはずです");
    let message = err.to_string();
    assert!(
        message.contains("unsupported video codec type"),
        "エラーメッセージが期待と異なります: {message}"
    );
    let err = video_from_codec_type(shiguredo_webrtc::VideoCodecType::Unknown(0), None)
        .expect_err("Unknown はエラーになるはずです");
    let message = err.to_string();
    assert!(
        message.contains("unsupported video codec type"),
        "エラーメッセージが期待と異なります: {message}"
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
    let args = crate::args::parse_args(raw_args)
        .expect("複数の libcamera コントロールはパースに成功するはずです");
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
    let result = crate::args::parse_args(raw_args);
    assert!(
        result.is_err(),
        "不正な libcamera コントロール形式は失敗するはずです"
    );
    let err = result.err().expect("エラーは必ず存在するはずです");
    assert!(
        err.to_string()
            .contains("--libcamera-control must be KEY=VALUE")
    );
}

#[test]
fn parse_args_accepts_libcamera_native_flag() {
    let raw_args = make_raw_args(&[
        "sumomo",
        "--signaling-url",
        "wss://example.com/signaling",
        "--channel-id",
        "test-channel",
        "--role",
        "sendonly",
        "--libcamera",
        "--libcamera-native",
    ]);
    let args =
        crate::args::parse_args(raw_args).expect("libcamera-native フラグの解析に失敗しました");
    assert!(args.use_libcamera);
    assert!(args.use_libcamera_native);
}

#[test]
fn validate_args_rejects_client_cert_without_client_key() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    // エラーが期待メッセージと完全一致することを検証する。
    // 完全一致なので、certificate の内容 (CERT-SECRET-DATA) がエラーに含まれないことも保証される。
    args.client_cert = Some("CERT-SECRET-DATA".to_string());
    let err = match validate_args(&args) {
        Err(AppError::Io(err)) => err,
        Err(other) => panic!("AppError::Io が返るはずです: {other:?}"),
        Ok(_) => panic!("certificate 単独指定は失敗するはずです"),
    };
    assert_eq!(
        err.to_string(),
        "--client-cert and --client-key must be specified together"
    );
}

#[test]
fn validate_args_rejects_client_key_without_client_cert() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    // エラーが期待メッセージと完全一致することを検証する。
    // 完全一致なので、private key の内容 (KEY-SECRET-DATA) がエラーに含まれないことも保証される。
    args.client_key = Some("KEY-SECRET-DATA".to_string());
    let err = match validate_args(&args) {
        Err(AppError::Io(err)) => err,
        Err(other) => panic!("AppError::Io が返るはずです: {other:?}"),
        Ok(_) => panic!("private key 単独指定は失敗するはずです"),
    };
    assert_eq!(
        err.to_string(),
        "--client-cert and --client-key must be specified together"
    );
}

#[test]
fn validate_args_accepts_client_cert_and_client_key() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    args.client_cert = Some("CERT-DATA".to_string());
    args.client_key = Some("KEY-DATA".to_string());
    assert!(
        validate_args(&args).is_ok(),
        "certificate と private key の両方指定は成功するはずです"
    );
}

#[test]
fn validate_args_accepts_no_client_cert_or_key() {
    let args = test_args(VideoCodecImplementationSelections::Auto, None);
    assert!(
        validate_args(&args).is_ok(),
        "certificate と private key の両方未指定は成功するはずです"
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
fn validate_args_accepts_mp4_with_openh264() {
    let mut args = test_args(
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Openh264,
        ]),
        Some("/tmp/libopenh264.so"),
    );
    args.input_mp4 = Some("/tmp/video.mp4".to_string());
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
    let err = validate_args(&args).expect_err("openh264 パスが無い場合は失敗するはずです");
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
    let err = validate_args(&args).expect_err("予期しない openh264 パスは失敗するはずです");
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
    let err = validate_args(&args).expect_err("auto と openh264 パスの併用は失敗するはずです");
    assert!(
        err.to_string()
            .contains("--openh264-path requires --video-codec-implementation to include openh264")
    );
}

#[test]
fn validate_args_rejects_libcamera_control_without_libcamera() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    args.libcamera_controls = vec![("Brightness".to_string(), "0.2".to_string())];
    let err = validate_args(&args)
        .expect_err("libcamera なしの libcamera コントロールは失敗するはずです");
    assert!(
        err.to_string()
            .contains("--libcamera-control requires --libcamera")
    );
}

#[test]
fn validate_args_rejects_libcamera_native_without_libcamera() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    args.use_libcamera_native = true;
    let err = validate_args(&args).expect_err("libcamera-native 単独指定は失敗する必要があります");
    assert!(
        err.to_string()
            .contains("--libcamera-native requires --libcamera")
    );
}

#[cfg(feature = "libcamera")]
#[test]
fn validate_args_accepts_libcamera_native_with_libcamera() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    args.use_libcamera = true;
    args.use_libcamera_native = true;
    assert!(validate_args(&args).is_ok());
}

// 併用不可チェックは libcamera が有効なときだけ到達する。
// libcamera 無効時は validate_args_rejects_libcamera_when_feature_is_disabled が
// 「libcamera が無効」エラーを検証する。
#[cfg(all(feature = "media-device", feature = "libcamera"))]
#[test]
fn validate_args_rejects_libcamera_with_video_input_device() {
    let mut args = test_args(VideoCodecImplementationSelections::Auto, None);
    args.use_libcamera = true;
    args.video_input_device = Some("/dev/video0".to_string());
    let err =
        validate_args(&args).expect_err("libcamera と video-input-device の併用は失敗するはずです");
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
    let err = validate_args(&args).expect_err("機能が無効な場合は libcamera は失敗するはずです");
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
    let err = validate_args(&args).expect_err("機能が無効な場合は vpl は失敗するはずです");
    assert!(
        err.to_string()
            .contains("VPL is not enabled in this build. Rebuild sumomo with --features vpl")
    );
}

#[cfg(not(feature = "v4l2"))]
#[test]
fn validate_args_rejects_v4l2_when_feature_is_disabled() {
    let args = test_args(
        VideoCodecImplementationSelections::Manual(vec![VideoCodecImplementationSelection::V4l2]),
        None,
    );
    let err = validate_args(&args).expect_err("機能が無効な場合は v4l2 は失敗するはずです");
    assert!(
        err.to_string()
            .contains("V4L2 is not enabled in this build. Rebuild sumomo with --features v4l2")
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
    .expect("auto 設定は構築できるはずです");

    let default_config = SoraConnectionContextConfig::default();
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
    .expect("manual 設定は構築できるはずです");
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

// MP4 使用時は送信 (Encoder) に passthrough のみを使い、受信 (Decoder) は選択された実装を維持することを確認する。
#[serial_test::serial]
#[test]
fn build_context_config_mp4_encoder_preference_uses_only_passthrough() {
    let config = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        Some(shiguredo_webrtc::VideoCodecType::H264),
        None,
        VideoCodecImplementationSelections::Auto,
    )
    .expect("MP4 設定は構築できるはずです");

    // Encoder 方向は MP4 の実 codec の passthrough 1 件のみを利用する。
    let encoders: Vec<&PreferenceCodec> = config
        .video_codec_preference
        .codecs()
        .iter()
        .filter(|codec| codec.direction() == CodecDirection::Encoder)
        .collect();
    assert_eq!(
        encoders.len(),
        1,
        "Encoder preference は passthrough のみのはずです"
    );
    assert_eq!(
        encoders[0].codec_type(),
        shiguredo_webrtc::VideoCodecType::H264
    );
    assert_eq!(encoders[0].implementation().name(), "mp4-passthrough");
    // Decoder 方向は MP4 なしの Auto 構成と同じ実装が維持される。
    let base_config = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        None,
        None,
        VideoCodecImplementationSelections::Auto,
    )
    .expect("Auto 設定は構築できるはずです");
    let decoders: Vec<&PreferenceCodec> = config
        .video_codec_preference
        .codecs()
        .iter()
        .filter(|codec| codec.direction() == CodecDirection::Decoder)
        .collect();
    let base_decoders: Vec<&PreferenceCodec> = base_config
        .video_codec_preference
        .codecs()
        .iter()
        .filter(|codec| codec.direction() == CodecDirection::Decoder)
        .collect();
    assert_eq!(decoders, base_decoders);
}

// MP4 使用時に Manual で internal を選んでも、送信 (Encoder) は passthrough に固定されることを確認する。
#[serial_test::serial]
#[test]
fn build_context_config_mp4_manual_internal_encoder_is_passthrough() {
    let config = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        Some(shiguredo_webrtc::VideoCodecType::H264),
        None,
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
        ]),
    )
    .expect("MP4 + internal 設定は構築できるはずです");

    let names = capability_names(&config);
    assert_eq!(
        names,
        vec!["internal".to_string(), "mp4-passthrough".to_string()]
    );
    // Encoder 方向は passthrough 1 件のみで、internal の Encoder エントリは残らない。
    let encoders: Vec<&PreferenceCodec> = config
        .video_codec_preference
        .codecs()
        .iter()
        .filter(|codec| codec.direction() == CodecDirection::Encoder)
        .collect();
    assert_eq!(
        encoders.len(),
        1,
        "Encoder preference は passthrough のみのはずです"
    );
    assert_eq!(
        encoders[0].codec_type(),
        shiguredo_webrtc::VideoCodecType::H264
    );
    assert_eq!(encoders[0].implementation().name(), "mp4-passthrough");
    // Decoder 方向は MP4 なしの Manual(internal) 構成と同じ実装が維持される。
    let base_config = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        None,
        None,
        VideoCodecImplementationSelections::Manual(vec![
            VideoCodecImplementationSelection::Internal,
        ]),
    )
    .expect("Manual 設定は構築できるはずです");
    let decoders: Vec<&PreferenceCodec> = config
        .video_codec_preference
        .codecs()
        .iter()
        .filter(|codec| codec.direction() == CodecDirection::Decoder)
        .collect();
    let base_decoders: Vec<&PreferenceCodec> = base_config
        .video_codec_preference
        .codecs()
        .iter()
        .filter(|codec| codec.direction() == CodecDirection::Decoder)
        .collect();
    assert_eq!(decoders, base_decoders);
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
        .expect("internal capability は必ず存在するはずです");
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
        .expect("openh264 capability は必ず存在するはずです");
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
        Ok(_) => panic!("internal-apple はサポート外のプラットフォームでは失敗するはずです"),
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
                .expect("h264 エンコーダーの preference は必ず存在するはずです");
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

/// `run_handle` のタスクが panic した場合、shutdown_connection が WorkerPanic を
/// 返すことを検証する。
#[tokio::test]
async fn shutdown_connection_converts_task_panic_to_worker_panic() {
    let context = SoraConnectionContext::new().expect("context の作成に失敗しました");
    let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(32);
    let builder = SoraConnection::builder(
        context,
        vec!["wss://127.0.0.1:1/signaling".to_string()],
        "test-channel".to_string(),
        Role::RecvOnly,
        AppEventHandler { event_tx },
    );
    let (connection, handle) = builder.build().expect("connection の build に失敗しました");

    // run タスクが panic する。is_finished() が true になるため、
    // shutdown_connection は disconnect を送らず run_handle.await の JoinError を WorkerPanic に変換する。
    let run_handle = tokio::spawn(async {
        panic!("intentional panic for test");
    });
    let _keep_connection = connection;

    // panic タスクが実行されて is_finished() が true になるまで待つ。
    while !run_handle.is_finished() {
        tokio::task::yield_now().await;
    }

    let deadline = tokio::time::Instant::now() + Duration::from_millis(50);
    let result = shutdown_connection(handle, run_handle, deadline).await;
    assert!(matches!(result, Err(AppError::WorkerPanic)));
}

/// 実 connection を build し、run を開始しない状態で shutdown_connection が
/// application deadline で timeout することを検証する。
///
/// `disconnect()` は run() が command を処理して初めて ack を返すため、
/// run を開始しないと ack が返らず timeout する。
#[tokio::test]
async fn shutdown_connection_times_out() {
    let context = SoraConnectionContext::new().expect("context の作成に失敗しました");
    let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(32);
    let builder = SoraConnection::builder(
        context,
        vec!["wss://127.0.0.1:1/signaling".to_string()],
        "test-channel".to_string(),
        Role::RecvOnly,
        AppEventHandler { event_tx },
    );
    let (connection, handle) = builder.build().expect("connection の build に失敗しました");

    // run() を開始せず、完了しない run_handle を渡す。
    // connection を保持したまま (drop しない) なので command channel は開いたまま。
    // run() が command_rx を poll しないため disconnect の ack が返らず timeout する。
    let run_handle = tokio::spawn(async {
        std::future::pending::<()>().await;
        Ok::<(), sora_sdk::Error>(())
    });
    let _keep_connection = connection;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(50);
    let result = shutdown_connection(handle, run_handle, deadline).await;
    assert!(matches!(result, Err(AppError::ConnectionShutdownTimeout)));
}

/// read-only の実 OS file へ ANSI output helper から書き込み、write error が伝播する。
#[test]
fn write_ansi_output_fails_on_readonly_file() {
    let path = std::env::temp_dir().join(format!(
        "sumomo-ansi-write-{}-{}.txt",
        std::process::id(),
        "write",
    ));
    std::fs::write(&path, "test").expect("temp file write failed");
    let mut perms = std::fs::metadata(&path)
        .expect("metadata failed")
        .permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms).expect("set readonly failed");

    let file = std::fs::File::open(&path).expect("open readonly file failed");
    let mut writer = file;
    let result = ansi_renderer::write_ansi_output(&mut writer, "output");
    assert!(
        matches!(result, Err(AppError::Io(_))),
        "write error は AppError::Io として伝播する必要があります: {result:?}"
    );

    // 後片付け用に明示的な権限 (owner rw, group/other r) へ戻してから削除する。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("restore permissions failed");
    }
    #[cfg(not(unix))]
    {
        let mut perms = std::fs::metadata(&path)
            .expect("metadata failed")
            .permissions();
        perms.set_readonly(false);
        std::fs::set_permissions(&path, perms).expect("unset readonly failed");
    }
    std::fs::remove_file(&path).expect("remove temp file failed");
}
