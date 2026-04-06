#[cfg(feature = "media-device")]
mod adm;
mod args;
mod error;
mod fake;
#[cfg(test)]
mod tests;
mod video_device;

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::io;
use std::io::IsTerminal;
use std::io::Write as IoWrite;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "raw-player")]
use std::thread;

#[cfg(feature = "media-device")]
use adm::{AudioDeviceCapturer, SumomoAdm};
use args::{
    Args, VideoCodecImplementationSelection, VideoCodecImplementationSelections, parse_args,
    validate_args,
};
use error::{AppError, Result};
use fake::{FakeVideoCapturer, FakeVideoCapturerConfig};
use rustls_pki_types::pem::PemObject;
use shiguredo_webrtc::{
    I420Buffer, LibyuvFourcc, VideoCodecType, VideoFrameRef, VideoSink, VideoSinkHandler,
    VideoSinkWants, convert_from_i420, log, rtc_log_info, rtc_log_warning,
};
#[cfg(feature = "amf")]
use sora_sdk::AmfVideoCodecCapability;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use sora_sdk::InternalAppleVideoCodecCapability;
#[cfg(feature = "nvcodec")]
use sora_sdk::NvCodecVideoCodecCapability;
use sora_sdk::{
    InternalVideoCodecCapability, Mp4PassthroughVideoCodecCapability, Mp4SampleReader,
    Mp4VideoCapturer, Openh264VideoCodecCapability, SoraClient, SoraClientContext,
    SoraClientContextConfig, VideoCodecCapability, VideoCodecPreference,
};
use tokio::sync::mpsc;
use video_device::VideoCapturerHolder;
#[cfg(feature = "media-device")]
use video_device::list_devices;

enum AppEvent {
    Notify(String),
    Push(String),
    OnTrack(shiguredo_webrtc::RtpTransceiver),
    OnRemoveTrack(shiguredo_webrtc::RtpReceiver),
}

fn add_video_codec_capability(
    context_config: &mut SoraClientContextConfig,
    capability: Box<dyn VideoCodecCapability>,
) {
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());
    context_config.video_codec_preference.merge(&preference);
    context_config.video_codec_capabilities.push(capability);
}

fn build_context_config(
    adm_config: sora_sdk::AdmConfig,
    mp4_codec_type: Option<VideoCodecType>,
    openh264_path: Option<&str>,
    video_codec_implementation: VideoCodecImplementationSelections,
) -> Result<SoraClientContextConfig> {
    let mut context_config = match video_codec_implementation {
        VideoCodecImplementationSelections::Auto => SoraClientContextConfig {
            adm_config,
            ..Default::default()
        },
        VideoCodecImplementationSelections::Manual(_) => SoraClientContextConfig {
            adm_config,
            video_codec_preference: VideoCodecPreference::default(),
            video_codec_capabilities: Vec::new(),
        },
    };

    if let Some(codec_type) = mp4_codec_type {
        let passthrough_capability: Box<dyn VideoCodecCapability> =
            Box::new(Mp4PassthroughVideoCodecCapability::new(codec_type));
        add_video_codec_capability(&mut context_config, passthrough_capability);
    }

    match video_codec_implementation {
        VideoCodecImplementationSelections::Auto => {}
        VideoCodecImplementationSelections::Manual(selections) => {
            for selection in selections {
                match selection {
                    VideoCodecImplementationSelection::Internal => {
                        let internal_capability: Box<dyn VideoCodecCapability> =
                            Box::new(InternalVideoCodecCapability::new());
                        add_video_codec_capability(&mut context_config, internal_capability);
                    }
                    VideoCodecImplementationSelection::InternalApple => {
                        #[cfg(any(target_os = "macos", target_os = "ios"))]
                        {
                            let capability =
                                InternalAppleVideoCodecCapability::new().ok_or_else(|| {
                                    io::Error::other(
                                        "internal-apple is not available on this device",
                                    )
                                })?;
                            let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
                            add_video_codec_capability(&mut context_config, capability);
                        }
                        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
                        {
                            return Err(io::Error::other(
                                "internal-apple is not supported on this platform",
                            )
                            .into());
                        }
                    }
                    VideoCodecImplementationSelection::Amf => {
                        #[cfg(feature = "amf")]
                        {
                            let amf_capability: Box<dyn VideoCodecCapability> =
                                Box::new(AmfVideoCodecCapability::new()?);
                            add_video_codec_capability(&mut context_config, amf_capability);
                        }
                        #[cfg(not(feature = "amf"))]
                        {
                            return Err(io::Error::other(
                                "AMF is not enabled in this build. Rebuild sumomo with --features amf",
                            )
                            .into());
                        }
                    }
                    VideoCodecImplementationSelection::Nvcodec => {
                        #[cfg(feature = "nvcodec")]
                        {
                            let nvcodec_capability: Box<dyn VideoCodecCapability> =
                                Box::new(NvCodecVideoCodecCapability::new());
                            add_video_codec_capability(&mut context_config, nvcodec_capability);
                        }
                        #[cfg(not(feature = "nvcodec"))]
                        {
                            return Err(io::Error::other(
                                "NVCodec is not enabled in this build. Rebuild sumomo with --features nvcodec",
                            )
                            .into());
                        }
                    }
                    VideoCodecImplementationSelection::Openh264 => {
                        let path = openh264_path.ok_or_else(|| {
                            io::Error::other(
                                "--video-codec-implementation openh264 requires --openh264-path",
                            )
                        })?;
                        let openh264_capability: Box<dyn VideoCodecCapability> =
                            Box::new(Openh264VideoCodecCapability::new(path)?);
                        add_video_codec_capability(&mut context_config, openh264_capability);
                    }
                }
            }
        }
    }

    Ok(context_config)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VideoCodecCapabilityReport {
    implementation: String,
    selected: bool,
    available: bool,
    unavailable_reason: Option<String>,
    encoder_codecs: Vec<String>,
    decoder_codecs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VideoCodecPreferenceReport {
    codec: String,
    encoder: Option<String>,
    decoder: Option<String>,
}

// --video-codec-list の表示用に、capability と preference を分離して保持する。
#[derive(Debug, Clone, PartialEq, Eq)]
struct VideoCodecListReport {
    capabilities: Vec<VideoCodecCapabilityReport>,
    preference: Vec<VideoCodecPreferenceReport>,
}

// capability 実体を保持し、最終表示用 report を組み立てるための内部表現。
struct VideoCodecCapabilityProbe {
    selection: VideoCodecImplementationSelection,
    selected: bool,
    capability: Option<Box<dyn VideoCodecCapability>>,
    unavailable_reason: Option<String>,
}

fn known_video_codec_types() -> [VideoCodecType; 5] {
    // preference は既知 codec を常に表示する。
    [
        VideoCodecType::Vp8,
        VideoCodecType::Vp9,
        VideoCodecType::Av1,
        VideoCodecType::H264,
        VideoCodecType::H265,
    ]
}

fn codec_name(codec_type: VideoCodecType) -> String {
    codec_type
        .as_str()
        .unwrap_or("unknown")
        .to_ascii_lowercase()
}

fn supported_codec_names(
    capability: &dyn VideoCodecCapability,
    direction: sora_sdk::CodecDirection,
) -> Vec<String> {
    // 既知 codec 一覧に対して capability の対応可否を評価する。
    known_video_codec_types()
        .into_iter()
        .filter(|codec_type| capability.is_supported(direction, *codec_type))
        .map(codec_name)
        .collect()
}

fn has_any_codec_support(capability: &dyn VideoCodecCapability) -> bool {
    known_video_codec_types().into_iter().any(|codec_type| {
        capability.is_supported(sora_sdk::CodecDirection::Encoder, codec_type)
            || capability.is_supported(sora_sdk::CodecDirection::Decoder, codec_type)
    })
}

fn is_selection_selected(
    args: &Args,
    selection: VideoCodecImplementationSelection,
    capability_available: bool,
) -> bool {
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let _ = capability_available;

    // selected は「ユーザーが選択したか」を示し、available とは独立して扱う。
    match &args.video_codec_implementation {
        VideoCodecImplementationSelections::Manual(selections) => selections.contains(&selection),
        VideoCodecImplementationSelections::Auto => match selection {
            VideoCodecImplementationSelection::Internal => true,
            VideoCodecImplementationSelection::InternalApple => {
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                {
                    capability_available
                }
                #[cfg(not(any(target_os = "macos", target_os = "ios")))]
                {
                    false
                }
            }
            VideoCodecImplementationSelection::Amf
            | VideoCodecImplementationSelection::Nvcodec
            | VideoCodecImplementationSelection::Openh264 => false,
        },
    }
}

fn probe_internal(args: &Args) -> VideoCodecCapabilityProbe {
    let capability: Box<dyn VideoCodecCapability> = Box::new(InternalVideoCodecCapability::new());
    let (capability, unavailable_reason) = if has_any_codec_support(capability.as_ref()) {
        (Some(capability), None)
    } else {
        (
            None,
            Some("internal does not support any encoder or decoder codec".to_string()),
        )
    };
    let selected = is_selection_selected(
        args,
        VideoCodecImplementationSelection::Internal,
        capability.is_some(),
    );
    VideoCodecCapabilityProbe {
        selection: VideoCodecImplementationSelection::Internal,
        selected,
        capability,
        unavailable_reason,
    }
}

fn probe_internal_apple(args: &Args) -> VideoCodecCapabilityProbe {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let (capability, unavailable_reason) = match InternalAppleVideoCodecCapability::new() {
        Some(capability) => {
            let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
            if has_any_codec_support(capability.as_ref()) {
                (Some(capability), None)
            } else {
                (
                    None,
                    Some(
                        "internal-apple does not support any encoder or decoder codec".to_string(),
                    ),
                )
            }
        }
        None => (
            None,
            Some("internal-apple is not available on this device".to_string()),
        ),
    };
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let (capability, unavailable_reason) = (
        None,
        Some("internal-apple is not supported on this platform".to_string()),
    );
    let selected = is_selection_selected(
        args,
        VideoCodecImplementationSelection::InternalApple,
        capability.is_some(),
    );
    VideoCodecCapabilityProbe {
        selection: VideoCodecImplementationSelection::InternalApple,
        selected,
        capability,
        unavailable_reason,
    }
}

fn probe_amf(args: &Args) -> VideoCodecCapabilityProbe {
    #[cfg(feature = "amf")]
    let (capability, unavailable_reason) = match AmfVideoCodecCapability::new() {
        Ok(capability) => {
            let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
            if has_any_codec_support(capability.as_ref()) {
                (Some(capability), None)
            } else {
                (
                    None,
                    Some("AMF does not support any encoder or decoder codec".to_string()),
                )
            }
        }
        Err(err) => (None, Some(err.to_string())),
    };
    #[cfg(not(feature = "amf"))]
    let (capability, unavailable_reason) = (
        None,
        Some("AMF is not enabled in this build. Rebuild sumomo with --features amf".to_string()),
    );
    let selected = is_selection_selected(
        args,
        VideoCodecImplementationSelection::Amf,
        capability.is_some(),
    );
    VideoCodecCapabilityProbe {
        selection: VideoCodecImplementationSelection::Amf,
        selected,
        capability,
        unavailable_reason,
    }
}

fn probe_nvcodec(args: &Args) -> VideoCodecCapabilityProbe {
    #[cfg(feature = "nvcodec")]
    let (capability, unavailable_reason) = {
        let capability: Box<dyn VideoCodecCapability> =
            Box::new(NvCodecVideoCodecCapability::new());
        if has_any_codec_support(capability.as_ref()) {
            (Some(capability), None)
        } else {
            (
                None,
                Some(
                    "NVCodec does not support any encoder or decoder codec on this device"
                        .to_string(),
                ),
            )
        }
    };
    #[cfg(not(feature = "nvcodec"))]
    let (capability, unavailable_reason) = (
        None,
        Some(
            "NVCodec is not enabled in this build. Rebuild sumomo with --features nvcodec"
                .to_string(),
        ),
    );
    let selected = is_selection_selected(
        args,
        VideoCodecImplementationSelection::Nvcodec,
        capability.is_some(),
    );
    VideoCodecCapabilityProbe {
        selection: VideoCodecImplementationSelection::Nvcodec,
        selected,
        capability,
        unavailable_reason,
    }
}

fn probe_openh264(args: &Args) -> VideoCodecCapabilityProbe {
    let (capability, unavailable_reason) = match args.openh264_path.as_deref() {
        Some(path) => match Openh264VideoCodecCapability::new(path) {
            Ok(capability) => {
                let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
                if has_any_codec_support(capability.as_ref()) {
                    (Some(capability), None)
                } else {
                    (
                        None,
                        Some("OpenH264 does not support any encoder or decoder codec".to_string()),
                    )
                }
            }
            Err(err) => (None, Some(format!("failed to initialize openh264: {err}"))),
        },
        None => (None, Some("--openh264-path is not specified".to_string())),
    };
    let selected = is_selection_selected(
        args,
        VideoCodecImplementationSelection::Openh264,
        capability.is_some(),
    );
    VideoCodecCapabilityProbe {
        selection: VideoCodecImplementationSelection::Openh264,
        selected,
        capability,
        unavailable_reason,
    }
}

fn collect_video_codec_capability_probes(args: &Args) -> Vec<VideoCodecCapabilityProbe> {
    // 表示順を固定するため、実装ごとに明示的な順序で probe する。
    vec![
        probe_internal(args),
        probe_internal_apple(args),
        probe_amf(args),
        probe_nvcodec(args),
        probe_openh264(args),
    ]
}

fn selected_implementations(
    args: &Args,
    probes: &[VideoCodecCapabilityProbe],
) -> Vec<VideoCodecImplementationSelection> {
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let _ = probes;

    // preference 合成対象となる実装順を決定する。
    match &args.video_codec_implementation {
        VideoCodecImplementationSelections::Manual(selections) => selections.clone(),
        VideoCodecImplementationSelections::Auto => {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                let mut selections = vec![VideoCodecImplementationSelection::Internal];
                if probes.iter().any(|probe| {
                    probe.selection == VideoCodecImplementationSelection::InternalApple
                        && probe.capability.is_some()
                }) {
                    selections.push(VideoCodecImplementationSelection::InternalApple);
                }
                selections
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            {
                vec![VideoCodecImplementationSelection::Internal]
            }
        }
    }
}

fn collect_video_codec_preference_report(
    args: &Args,
    probes: &[VideoCodecCapabilityProbe],
) -> Vec<VideoCodecPreferenceReport> {
    let mut preference = VideoCodecPreference::default();
    // selected された実装だけを順番に merge して最終 preference を作る。
    for selection in selected_implementations(args, probes) {
        let capability = probes
            .iter()
            .find(|probe| probe.selection == selection)
            .and_then(|probe| probe.capability.as_deref());
        if let Some(capability) = capability {
            preference.merge(&VideoCodecPreference::new_from_capability(capability));
        }
    }

    let mut reports = Vec::new();
    // 表示は既知 codec を固定順で全て出す。
    for codec_type in known_video_codec_types() {
        let encoder = preference
            .find(sora_sdk::CodecDirection::Encoder, codec_type)
            .map(|codec| codec.implementation().name().to_string());
        let decoder = preference
            .find(sora_sdk::CodecDirection::Decoder, codec_type)
            .map(|codec| codec.implementation().name().to_string());
        reports.push(VideoCodecPreferenceReport {
            codec: codec_name(codec_type),
            encoder,
            decoder,
        });
    }
    reports
}

fn collect_video_codec_list_report(args: &Args) -> VideoCodecListReport {
    let probes = collect_video_codec_capability_probes(args);

    // capability probe 結果を表示用構造へ正規化する。
    let capabilities = probes
        .iter()
        .map(|probe| {
            let (encoder_codecs, decoder_codecs) = match probe.capability.as_deref() {
                Some(capability) => (
                    supported_codec_names(capability, sora_sdk::CodecDirection::Encoder),
                    supported_codec_names(capability, sora_sdk::CodecDirection::Decoder),
                ),
                None => (Vec::new(), Vec::new()),
            };
            VideoCodecCapabilityReport {
                implementation: probe.selection.name().to_string(),
                selected: probe.selected,
                available: probe.capability.is_some(),
                unavailable_reason: probe.unavailable_reason.clone(),
                encoder_codecs,
                decoder_codecs,
            }
        })
        .collect();

    let preference = collect_video_codec_preference_report(args, &probes);

    // capability と preference をひとまとまりで返す。
    VideoCodecListReport {
        capabilities,
        preference,
    }
}

fn is_ansi_output_enabled() -> bool {
    // TTY 以外や no-color 指定時は装飾しない。
    if !std::io::stdout().is_terminal() {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    !matches!(std::env::var("TERM").ok().as_deref(), Some("dumb"))
}

fn ansi_style(text: &str, code: &str, enabled: bool) -> String {
    // ANSI 無効時は入力文字列をそのまま返す。
    if !enabled {
        return text.to_string();
    }
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn build_preference_display_value(value: Option<&str>, ansi_enabled: bool) -> String {
    // 未選択は (none) で表示し、ANSI 有効時のみ薄色にする。
    match value {
        Some(value) => value.to_string(),
        None => ansi_style("(none)", "2", ansi_enabled),
    }
}

fn build_video_codec_list_report_text(report: &VideoCodecListReport, ansi_enabled: bool) -> String {
    let mut out = String::new();
    // implementation 列は実データの最大幅に合わせて揃える。
    let implementation_width = report
        .capabilities
        .iter()
        .map(|capability| capability.implementation.chars().count())
        .max()
        .unwrap_or(0);

    writeln!(out, "Video codec capability:").expect("write to string");
    for capability in &report.capabilities {
        let selected_mark = if capability.selected { "x" } else { " " };
        let implementation = capability.implementation.as_str();
        // capability は利用可否で表示内容を切り替える。
        let body = if capability.available {
            let encoder = if capability.encoder_codecs.is_empty() {
                "(none)".to_string()
            } else {
                capability.encoder_codecs.join(", ")
            };
            let decoder = if capability.decoder_codecs.is_empty() {
                "(none)".to_string()
            } else {
                capability.decoder_codecs.join(", ")
            };
            format!(
                "- [{selected_mark}] {implementation:<implementation_width$} enc({encoder}) dec({decoder})",
            )
        } else if let Some(reason) = &capability.unavailable_reason {
            format!(
                "- [{selected_mark}] {implementation:<implementation_width$} :unavailable: {reason}",
            )
        } else {
            format!(
                "- [{selected_mark}] {implementation:<implementation_width$} :unavailable: unknown reason",
            )
        };

        // selected は強調、unavailable は薄色で見分けやすくする。
        let line = if capability.available {
            if capability.selected {
                ansi_style(&body, "1", ansi_enabled)
            } else {
                body
            }
        } else if capability.selected {
            ansi_style(&body, "1;2", ansi_enabled)
        } else {
            ansi_style(&body, "2", ansi_enabled)
        };
        writeln!(out, "{line}").expect("write to string");
    }

    writeln!(out).expect("write to string");
    writeln!(out, "Video codec preference:").expect("write to string");
    // enc 列も実データの最大幅で揃える。
    let encoder_width = report
        .preference
        .iter()
        .map(|preference| {
            preference
                .encoder
                .as_deref()
                .unwrap_or("(none)")
                .chars()
                .count()
        })
        .max()
        .unwrap_or(0);
    for preference in &report.preference {
        let encoder_plain = preference.encoder.as_deref().unwrap_or("(none)");
        let encoder = format!(
            "{}{}",
            build_preference_display_value(preference.encoder.as_deref(), ansi_enabled),
            // encoder_width の幅になるように右側を埋める。
            " ".repeat(encoder_width.saturating_sub(encoder_plain.chars().count())),
        );
        let decoder = build_preference_display_value(preference.decoder.as_deref(), ansi_enabled);
        writeln!(
            out,
            "- {:<4} enc: {} dec: {}",
            preference.codec, encoder, decoder
        )
        .expect("write to string");
    }
    out
}

fn render_video_codec_list_report(report: &VideoCodecListReport) {
    // ANSI 可否判定を反映して最終テキストを描画する。
    let text = build_video_codec_list_report_text(report, is_ansi_output_enabled());
    print!("{text}");
}

fn run_video_codec_list(args: &Args) -> Result<()> {
    // --video-codec-list 専用の収集と描画だけ実行する。
    let report = collect_video_codec_list_report(args);
    render_video_codec_list_report(&report);
    Ok(())
}

/// ANSI 描画用の簡易レンダラー。
struct AnsiRenderer {
    width: i32,
    height: i32,
}

impl AnsiRenderer {
    fn new() -> Self {
        Self {
            width: 80,
            height: 45,
        }
    }

    fn render(&self, frame: VideoFrameRef) {
        render_frame(frame, self.width, self.height);
    }
}

#[cfg(feature = "raw-player")]
struct I420Frame {
    width: i32,
    height: i32,
    y_data: Vec<u8>,
    y_stride: i32,
    u_data: Vec<u8>,
    u_stride: i32,
    v_data: Vec<u8>,
    v_stride: i32,
}

#[cfg(feature = "raw-player")]
impl I420Frame {
    fn from_buffer(buffer: &I420Buffer) -> Self {
        Self {
            width: buffer.width(),
            height: buffer.height(),
            y_data: buffer.y_data().to_vec(),
            y_stride: buffer.stride_y(),
            u_data: buffer.u_data().to_vec(),
            u_stride: buffer.stride_u(),
            v_data: buffer.v_data().to_vec(),
            v_stride: buffer.stride_v(),
        }
    }
}

#[cfg(feature = "raw-player")]
struct RawPlayerRenderer {
    window: raw_player::Window,
    renderer: raw_player::Renderer,
    texture: Option<raw_player::Texture>,
    running: bool,
}

#[cfg(feature = "raw-player")]
impl RawPlayerRenderer {
    fn new(title: &str, width: i32, height: i32) -> Result<Self> {
        raw_player::init()?;
        let window = raw_player::Window::new(title, width, height)?;
        let renderer = raw_player::Renderer::new(&window)?;
        Ok(Self {
            window,
            renderer,
            texture: None,
            running: true,
        })
    }

    fn render(&mut self, frame: &I420Frame) {
        let width = frame.width;
        let height = frame.height;

        let needs_recreate = self
            .texture
            .as_ref()
            .map_or(true, |t| t.width() != width || t.height() != height);
        if needs_recreate {
            let _ = self.window.set_size(width, height);
            self.texture = raw_player::Texture::new_yuv(&self.renderer, width, height).ok();
        }

        if let Some(ref mut texture) = self.texture {
            let _ = texture.update_yuv(
                &frame.y_data,
                frame.y_stride,
                &frame.u_data,
                frame.u_stride,
                &frame.v_data,
                frame.v_stride,
            );

            let _ = self.renderer.set_draw_color(0, 0, 0, 255);
            let _ = self.renderer.clear();
            let _ = self.renderer.copy(texture);
            let _ = self.renderer.present();
        }
    }

    fn poll_events(&mut self) {
        while let Some(event) = raw_player::poll_event() {
            match event {
                raw_player::Event::Quit | raw_player::Event::WindowClose => {
                    self.running = false;
                }
                raw_player::Event::KeyDown { keycode } if keycode == raw_player::KEYCODE_ESCAPE => {
                    self.running = false;
                }
                _ => {}
            }
        }
    }

    fn is_running(&self) -> bool {
        self.running
    }
}

struct TrackEntry {
    sink: VideoSink,
}

#[cfg(feature = "raw-player")]
struct RawPlayerTrackSinkHandler {
    frame_tx: std::sync::mpsc::SyncSender<I420Frame>,
    first_frame: Arc<AtomicBool>,
    track_id_for_log: String,
}

#[cfg(feature = "raw-player")]
impl VideoSinkHandler for RawPlayerTrackSinkHandler {
    fn on_frame(&mut self, frame: VideoFrameRef<'_>) {
        if !self.first_frame.swap(true, Ordering::Relaxed) {
            rtc_log_info!(
                "ビデオ フレームを受信しました: track_id={}",
                self.track_id_for_log
            );
        }
        let mut buffer = frame.buffer();
        let Some(i420_buffer) = buffer.to_i420() else {
            return;
        };
        let i420_frame = I420Frame::from_buffer(&i420_buffer);
        let _ = self.frame_tx.try_send(i420_frame);
    }
}

struct AnsiTrackSinkHandler {
    renderer: Arc<AnsiRenderer>,
    first_frame: Arc<AtomicBool>,
    track_id_for_log: String,
}

impl VideoSinkHandler for AnsiTrackSinkHandler {
    fn on_frame(&mut self, frame: VideoFrameRef<'_>) {
        if !self.first_frame.swap(true, Ordering::Relaxed) {
            rtc_log_info!(
                "ビデオ フレームを受信しました: track_id={}",
                self.track_id_for_log
            );
        }
        self.renderer.render(frame);
    }
}

fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> i32 {
    let r6 = (r as i32 * 5) / 255;
    let g6 = (g as i32 * 5) / 255;
    let b6 = (b as i32 * 5) / 255;
    16 + (r6 * 36) + (g6 * 6) + b6
}

fn render_frame(frame: VideoFrameRef, width: i32, height: i32) {
    let mut src = frame.buffer();
    let Some(src_i420) = src.to_i420() else {
        return;
    };
    let mut scaled = I420Buffer::new(width, height);
    scaled.scale_from(&src_i420);

    let width_u = width.max(0) as usize;
    let height_u = height.max(0) as usize;
    let Some(dst_stride) = width_u.checked_mul(4) else {
        return;
    };
    let Some(dst_bytes) = dst_stride.checked_mul(height_u) else {
        return;
    };
    let mut image = vec![0u8; dst_bytes];
    if !convert_from_i420(
        scaled.y_data(),
        scaled.stride_y(),
        scaled.u_data(),
        scaled.stride_u(),
        scaled.v_data(),
        scaled.stride_v(),
        &mut image,
        dst_stride as i32,
        width,
        height,
        LibyuvFourcc::Argb,
    ) {
        return;
    }
    let capacity = width_u.saturating_mul(height_u).saturating_mul(20);
    let mut output = String::with_capacity(capacity);
    output.push_str("\x1b[H");

    // 2x1 ピクセルを 1 文字で表現する。
    for y in (0..height_u).step_by(2) {
        output.push_str("\x1b[2K");
        for x in 0..width_u {
            let upper_offset = (y * width_u + x) * 4;
            let upper_r = image[upper_offset + 2];
            let upper_g = image[upper_offset + 1];
            let upper_b = image[upper_offset];

            let (lower_r, lower_g, lower_b) = if y + 1 < height_u {
                let lower_offset = ((y + 1) * width_u + x) * 4;
                let lower_r = image[lower_offset + 2];
                let lower_g = image[lower_offset + 1];
                let lower_b = image[lower_offset];
                (lower_r, lower_g, lower_b)
            } else {
                (upper_r, upper_g, upper_b)
            };
            let upper_color = rgb_to_ansi256(upper_r, upper_g, upper_b);
            let lower_color = rgb_to_ansi256(lower_r, lower_g, lower_b);
            let _ = write!(
                output,
                "\x1b[38;5;{}m\x1b[48;5;{}m▀",
                upper_color, lower_color
            );
        }
        output.push_str("\x1b[0m\n");
    }

    let mut stdout = io::stdout();
    let _ = stdout.write_all(output.as_bytes());
    let _ = stdout.flush();
}

#[cfg(feature = "raw-player")]
fn run_with_raw_player(args: Args) -> Result<()> {
    log::log_to_debug(log::Severity::Warning);
    log::enable_timestamps();
    log::enable_threads();

    let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel::<I420Frame>(2);
    let (event_tx, event_rx) = std::sync::mpsc::channel::<AppEvent>();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();

    let signaling_urls = args.signaling_urls.clone();
    let channel_id = args.channel_id.clone();
    let role = args.role;
    let data_channel_signaling = args.data_channel_signaling;
    let simulcast = args.simulcast;
    let insecure = args.insecure;
    let client_cert = args.client_cert.clone();
    let client_key = args.client_key.clone();
    let ca_cert = args.ca_cert.clone();
    let video_codec_implementation = args.video_codec_implementation.clone();
    let context_config = build_context_config(
        sora_sdk::AdmConfig::NoAudioDevice,
        None,
        args.openh264_path.as_deref(),
        video_codec_implementation,
    )?;
    let context = SoraClientContext::new_with_config(context_config)?;
    let context_for_thread = context.clone();

    let handle = thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Tokio ランタイムの作成に失敗しました");

        let _ = rt.block_on(async {
            let context = context_for_thread.clone();
            let mut builder =
                SoraClient::builder(context.clone(), signaling_urls, channel_id, role)
                .on_notify({
                    let event_tx = event_tx.clone();
                    move |text| {
                        let _ = event_tx.send(AppEvent::Notify(text.to_string()));
                    }
                })
                .on_push({
                    let event_tx = event_tx.clone();
                    move |text| {
                        let _ = event_tx.send(AppEvent::Push(text.to_string()));
                    }
                })
                .on_track({
                    let event_tx = event_tx.clone();
                    move |transceiver| {
                        rtc_log_info!("on_track コールバックが呼ばれました");
                        let _ = event_tx.send(AppEvent::OnTrack(transceiver));
                    }
                })
                .on_remove_track({
                    let event_tx = event_tx.clone();
                    move |receiver| {
                        let _ = event_tx.send(AppEvent::OnRemoveTrack(receiver));
                    }
                });

            let mut _video_capturer: Option<VideoCapturerHolder> = None;
            if role.wants_send() {
                let mut capturer = VideoCapturerHolder::Fake(
                    FakeVideoCapturer::new(FakeVideoCapturerConfig::default())?
                );
                capturer.start()?;
                let video_track = context.create_video_track(&capturer.video_source())?;
                builder = builder.sender_video_track(video_track);
                _video_capturer = Some(capturer);
            }
            if role.wants_send() {
                let audio_source = context.create_audio_source()?;
                let audio_track = context.create_audio_track(&audio_source)?;
                builder = builder.sender_audio_track(audio_track);
            }

            if let Some(data_channel_signaling) = data_channel_signaling {
                builder = builder.data_channel_signaling(data_channel_signaling);
            }
            if let Some(simulcast) = simulcast {
                builder = builder.simulcast(simulcast);
            }
            builder = builder.insecure(insecure);
            if let (Some(cert), Some(key)) = (client_cert, client_key) {
                builder = builder.client_cert(cert, key);
            }
            if let Some(ca) = ca_cert {
                builder = builder.ca_cert(ca);
            }

            let (client, _handle) = builder.build()?;
            let mut tracks: HashMap<String, TrackEntry> = HashMap::new();
            let mut run = Box::pin(client.run());

            loop {
                if stop_for_thread.load(Ordering::Relaxed) {
                    break;
                }

                tokio::select! {
                    result = &mut run => {
                        return result.map_err(AppError::Sora);
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                        while let Ok(event) = event_rx.try_recv() {
                            match event {
                                AppEvent::Notify(text) => {
                                    rtc_log_info!("notify を受信しました: {}", text);
                                }
                                AppEvent::Push(text) => {
                                    rtc_log_info!("push を受信しました: {}", text);
                                }
                                AppEvent::OnTrack(transceiver) => {
                                    let receiver = transceiver.receiver();
                                    let track = receiver.track();
                                    let kind = match track.kind() {
                                        Ok(kind) => kind,
                                        Err(_) => "unknown".to_string(),
                                    };
                                    if kind != "video" {
                                        rtc_log_warning!("ビデオ以外のトラックを受信しました: kind={}", kind);
                                        continue;
                                    }
                                    let track_id = match track.id() {
                                        Ok(id) => id,
                                        Err(_) => {
                                            rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
                                            continue;
                                        }
                                    };
                                    let mut video_track = track.cast_to_video_track();
                                    if let Some(old_entry) = tracks.remove(&track_id) {
                                        rtc_log_info!("既存のトラックを削除します: track_id={}", track_id);
                                        video_track.remove_sink(&old_entry.sink);
                                    }
                                    rtc_log_info!("ビデオ トラックが追加されました: track_id={}", track_id);
                                    let frame_tx = frame_tx.clone();
                                    let first_frame = Arc::new(AtomicBool::new(false));
                                    let sink = VideoSink::new_with_handler(Box::new(
                                        RawPlayerTrackSinkHandler {
                                            frame_tx,
                                            first_frame,
                                            track_id_for_log: track_id.clone(),
                                        },
                                    ));
                                    let wants = VideoSinkWants::new();
                                    video_track.add_or_update_sink(&sink, &wants);
                                    tracks.insert(track_id, TrackEntry { sink });
                                }
                                AppEvent::OnRemoveTrack(receiver) => {
                                    let track = receiver.track();
                                    let track_id = match track.id() {
                                        Ok(id) => id,
                                        Err(_) => {
                                            rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
                                            continue;
                                        }
                                    };
                                    let kind = match track.kind() {
                                        Ok(kind) => kind,
                                        Err(_) => "unknown".to_string(),
                                    };
                                    if kind != "video" {
                                        rtc_log_warning!("ビデオ以外のトラックが削除されました: kind={}", kind);
                                        continue;
                                    }
                                    let mut video_track = track.cast_to_video_track();
                                    if let Some(entry) = tracks.remove(&track_id) {
                                        video_track.remove_sink(&entry.sink);
                                    }
                                    rtc_log_info!("ビデオ トラックが削除されました: track_id={}", track_id);
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        });
    });

    let mut raw_player_renderer = RawPlayerRenderer::new("Sumomo - Video", 640, 480)?;

    let mut frame_count = 0u64;
    while raw_player_renderer.is_running() {
        raw_player_renderer.poll_events();

        while let Ok(frame) = frame_rx.try_recv() {
            frame_count += 1;
            if frame_count == 1 {
                rtc_log_info!(
                    "raw_player: 最初のフレームを受信しました: {}x{}",
                    frame.width,
                    frame.height
                );
            }
            raw_player_renderer.render(&frame);
        }

        thread::sleep(std::time::Duration::from_millis(1));
    }

    stop.store(true, Ordering::Relaxed);
    let _ = handle.join();
    raw_player::quit();
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = parse_args()?;

    // codec list モードは接続処理を行わず早期終了する。
    if args.video_codec_list {
        return run_video_codec_list(&args);
    }

    log::log_to_debug(log::Severity::Info);
    log::enable_timestamps();
    log::enable_threads();

    validate_args(&args)?;

    #[cfg(feature = "media-device")]
    if args.list_devices {
        return list_devices();
    }

    let video_enabled = args.video.unwrap_or(true);
    #[cfg(feature = "raw-player")]
    if args.use_raw_player {
        return run_with_raw_player(args);
    }

    let renderer = Arc::new(AnsiRenderer::new());
    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(32);

    // --audio-input-device が指定された場合は SumomoAdm を使用する
    #[cfg(feature = "media-device")]
    let external_adm = if args.audio_input_device.is_some() {
        Some(SumomoAdm::new())
    } else {
        None
    };

    // --input-mp4 が指定されている場合は MP4 を読み込んでパススルーの準備をする
    let mp4_state = if let Some(ref mp4_path) = args.input_mp4 {
        let reader = Mp4SampleReader::new(mp4_path)?;
        let codec_type = reader.codec_type();
        Some((reader, codec_type))
    } else {
        None
    };

    #[cfg(feature = "media-device")]
    let adm_config = if external_adm.is_some() {
        sora_sdk::AdmConfig::UseExternal(external_adm.as_ref().unwrap().audio_device_module())
    } else {
        sora_sdk::AdmConfig::NoAudioDevice
    };
    #[cfg(not(feature = "media-device"))]
    let adm_config = sora_sdk::AdmConfig::NoAudioDevice;

    let context_config = build_context_config(
        adm_config,
        mp4_state.as_ref().map(|(_, codec_type)| *codec_type),
        args.openh264_path.as_deref(),
        args.video_codec_implementation.clone(),
    )?;
    let context = SoraClientContext::new_with_config(context_config)?;

    // --audio-input-device が指定された場合は AudioDeviceCapturer を使用する
    #[cfg(feature = "media-device")]
    let mut _audio_capturer = if let Some(ref device_id) = args.audio_input_device {
        let state = external_adm
            .as_ref()
            .expect("BUG: external_adm が None です")
            .state();
        let mut capturer = AudioDeviceCapturer::new(Some(device_id.clone()), state)?;
        capturer.start()?;
        rtc_log_info!("オーディオ入力デバイスを開始しました: {}", device_id);
        Some(capturer)
    } else {
        None
    };

    let mut builder = SoraClient::builder(
        context.clone(),
        args.signaling_urls,
        args.channel_id,
        args.role,
    )
    .on_notify({
        let event_tx = event_tx.clone();
        move |text| {
            let _ = event_tx.try_send(AppEvent::Notify(text.to_string()));
        }
    })
    .on_push({
        let event_tx = event_tx.clone();
        move |text| {
            let _ = event_tx.try_send(AppEvent::Push(text.to_string()));
        }
    })
    .on_track({
        let event_tx = event_tx.clone();
        move |transceiver| {
            let _ = event_tx.try_send(AppEvent::OnTrack(transceiver));
        }
    })
    .on_remove_track({
        let event_tx = event_tx.clone();
        move |receiver| {
            let _ = event_tx.try_send(AppEvent::OnRemoveTrack(receiver));
        }
    });

    if let Some(audio) = args.audio {
        builder = builder.audio(sora_sdk::Audio::new_bool(audio));
    }

    let video_bit_rate = args.video_bit_rate;
    if let Some(video) = args.video {
        if video {
            let video_setting = match args.video_codec_type.as_deref() {
                Some("vp8") => sora_sdk::Video::new_vp8(video_bit_rate),
                Some("vp9") => sora_sdk::Video::new_vp9(video_bit_rate, None),
                Some("av1") => sora_sdk::Video::new_av1(video_bit_rate, None),
                Some("h264") => sora_sdk::Video::new_h264(video_bit_rate, None),
                Some("h265") => sora_sdk::Video::new_h265(video_bit_rate, None),
                None => sora_sdk::Video::new_bool(true),
                _ => sora_sdk::Video::new_bool(true),
            };
            builder = builder.video(video_setting);
        } else {
            builder = builder.video(sora_sdk::Video::new_bool(false));
        }
    } else if let Some(ref codec) = args.video_codec_type {
        let video_setting = match codec.as_str() {
            "vp8" => sora_sdk::Video::new_vp8(video_bit_rate),
            "vp9" => sora_sdk::Video::new_vp9(video_bit_rate, None),
            "av1" => sora_sdk::Video::new_av1(video_bit_rate, None),
            "h264" => sora_sdk::Video::new_h264(video_bit_rate, None),
            "h265" => sora_sdk::Video::new_h265(video_bit_rate, None),
            _ => sora_sdk::Video::new_bool(true),
        };
        builder = builder.video(video_setting);
    }

    let mut _video_capturer: Option<VideoCapturerHolder> = None;
    if args.role.wants_send() && video_enabled {
        let mut capturer = if let Some((reader, _codec_type)) = mp4_state {
            // --input-mp4 が最優先
            let mp4_capturer = Mp4VideoCapturer::new(reader)?;
            VideoCapturerHolder::Mp4(mp4_capturer)
        } else {
            #[cfg(feature = "media-device")]
            {
                if let Some(ref device_id) = args.video_input_device {
                    VideoCapturerHolder::Device(VideoDeviceCapturer::new(Some(device_id.clone()))?)
                } else {
                    let fake = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())?;
                    VideoCapturerHolder::Fake(fake)
                }
            }
            #[cfg(not(feature = "media-device"))]
            {
                let fake = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())?;
                VideoCapturerHolder::Fake(fake)
            }
        };
        capturer.start()?;
        let video_track = context.create_video_track(&capturer.video_source())?;
        builder = builder.sender_video_track(video_track);
        _video_capturer = Some(capturer);
    }

    if args.role.wants_send() {
        let audio_source = context.create_audio_source()?;
        let audio_track = context.create_audio_track(&audio_source)?;
        builder = builder.sender_audio_track(audio_track);
    }

    if let Some(data_channel_signaling) = args.data_channel_signaling {
        builder = builder.data_channel_signaling(data_channel_signaling);
    }
    if let Some(ignore_disconnect_websocket) = args.ignore_disconnect_websocket {
        builder = builder.ignore_disconnect_websocket(ignore_disconnect_websocket);
    }
    if let Some(simulcast) = args.simulcast {
        builder = builder.simulcast(simulcast);
    }
    builder = builder.insecure(args.insecure);
    if let (Some(cert), Some(key)) = (args.client_cert, args.client_key) {
        builder = builder.client_cert(cert, key);
    }
    if let Some(ca) = args.ca_cert {
        builder = builder.ca_cert(ca);
    }

    if args.turn_tls_insecure {
        builder = builder.turn_tls_insecure(true);
    }
    if let Some(ca_cert_path) = args.turn_tls_ca_cert {
        let pem_data = std::fs::read(&ca_cert_path)?;
        let cert = rustls_pki_types::CertificateDer::from_pem_slice(&pem_data)?;
        builder = builder.turn_tls_ca_cert(cert.to_vec());
    }

    let (client, _handle) = builder.build()?;
    let renderer_for_events = renderer.clone();
    let mut tracks: HashMap<String, TrackEntry> = HashMap::new();
    let mut run = Box::pin(client.run());

    // duration が指定されている場合はタイマーを設定
    let duration_sleep = args.duration.map(|secs| {
        rtc_log_info!("{} 秒後に切断します", secs);
        tokio::time::sleep(std::time::Duration::from_secs(secs))
    });
    tokio::pin!(duration_sleep);

    loop {
        tokio::select! {
            result = &mut run => {
                return result.map_err(AppError::Sora);
            }
            _ = async { duration_sleep.as_mut().as_pin_mut().unwrap().await }, if duration_sleep.is_some() => {
                rtc_log_info!("指定された時間が経過しました。切断します");
                break;
            }
            event = event_rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                match event {
                    AppEvent::Notify(text) => {
                        rtc_log_info!("notify を受信しました: {}", text);
                    }
                    AppEvent::Push(text) => {
                        rtc_log_info!("push を受信しました: {}", text);
                    }
                    AppEvent::OnTrack(transceiver) => {
                        let receiver = transceiver.receiver();
                        let track = receiver.track();
                        let kind = match track.kind() {
                            Ok(kind) => kind,
                            Err(_) => "unknown".to_string(),
                        };
                        if kind != "video" {
                            rtc_log_warning!("ビデオ以外のトラックを受信しました: kind={}", kind);
                            continue;
                        }
                        let track_id = match track.id() {
                            Ok(id) => id,
                            Err(_) => {
                                rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
                                continue;
                            }
                        };
                        let mut video_track = track.cast_to_video_track();
                        if tracks.contains_key(&track_id) {
                            continue;
                        }
                        rtc_log_info!("ビデオ トラックが追加されました: track_id={}", track_id);
                        let first_frame = Arc::new(AtomicBool::new(false));
                        let sink = VideoSink::new_with_handler(Box::new(AnsiTrackSinkHandler {
                            renderer: renderer_for_events.clone(),
                            first_frame,
                            track_id_for_log: track_id.clone(),
                        }));
                        let wants = VideoSinkWants::new();
                        video_track.add_or_update_sink(&sink, &wants);
                        tracks.insert(track_id, TrackEntry { sink });
                    }
                    AppEvent::OnRemoveTrack(receiver) => {
                        let track = receiver.track();
                        let track_id = match track.id() {
                            Ok(id) => id,
                            Err(_) => {
                                rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
                                continue;
                            }
                        };
                        let kind = match track.kind() {
                            Ok(kind) => kind,
                            Err(_) => "unknown".to_string(),
                        };
                        if kind != "video" {
                            rtc_log_warning!("ビデオ以外のトラックが削除されました: kind={}", kind);
                            continue;
                        }
                        let mut video_track = track.cast_to_video_track();
                        if let Some(entry) = tracks.remove(&track_id) {
                            video_track.remove_sink(&entry.sink);
                        }
                        rtc_log_info!("ビデオ トラックが削除されました: track_id={}", track_id);
                    }
                }
            }
        }
    }
    run.await?;
    Ok(())
}
