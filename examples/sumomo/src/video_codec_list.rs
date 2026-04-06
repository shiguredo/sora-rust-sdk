use std::fmt::Write as FmtWrite;
use std::io::IsTerminal;

#[cfg(feature = "amf")]
use sora_sdk::AmfVideoCodecCapability;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use sora_sdk::InternalAppleVideoCodecCapability;
#[cfg(feature = "nvcodec")]
use sora_sdk::NvCodecVideoCodecCapability;
#[cfg(feature = "vpl")]
use sora_sdk::VplVideoCodecCapability;
use sora_sdk::{
    InternalVideoCodecCapability, Openh264VideoCodecCapability, VideoCodecCapability,
    VideoCodecPreference,
};

use crate::args::{Args, VideoCodecImplementationSelection, VideoCodecImplementationSelections};
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VideoCodecCapabilityReport {
    pub(crate) implementation: String,
    pub(crate) selected: bool,
    pub(crate) available: bool,
    pub(crate) unavailable_reason: Option<String>,
    pub(crate) encoder_codecs: Vec<String>,
    pub(crate) decoder_codecs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VideoCodecPreferenceReport {
    pub(crate) codec: String,
    pub(crate) encoder: Option<String>,
    pub(crate) decoder: Option<String>,
}

// --video-codec-list の表示用に、capability と preference を分離して保持する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VideoCodecListReport {
    pub(crate) capabilities: Vec<VideoCodecCapabilityReport>,
    pub(crate) preference: Vec<VideoCodecPreferenceReport>,
}

// capability 実体を保持し、最終表示用 report を組み立てるための内部表現。
struct VideoCodecCapabilityProbe {
    selection: VideoCodecImplementationSelection,
    selected: bool,
    capability: Option<Box<dyn VideoCodecCapability>>,
    unavailable_reason: Option<String>,
}

fn known_video_codec_types() -> [shiguredo_webrtc::VideoCodecType; 5] {
    // preference は既知 codec を常に表示する。
    [
        shiguredo_webrtc::VideoCodecType::Vp8,
        shiguredo_webrtc::VideoCodecType::Vp9,
        shiguredo_webrtc::VideoCodecType::Av1,
        shiguredo_webrtc::VideoCodecType::H264,
        shiguredo_webrtc::VideoCodecType::H265,
    ]
}

fn codec_name(codec_type: shiguredo_webrtc::VideoCodecType) -> String {
    codec_type
        .as_str()
        .unwrap_or("unknown")
        .to_ascii_lowercase()
}

fn supported_codec_names(
    capability: &dyn VideoCodecCapability,
    direction: sora_sdk::CodecDirection,
) -> Vec<String> {
    known_video_codec_types()
        .into_iter()
        .filter(|codec| capability.is_supported(direction, *codec))
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
    available: bool,
) -> bool {
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let _ = available;

    match &args.video_codec_implementation {
        VideoCodecImplementationSelections::Manual(selections) => selections.contains(&selection),
        VideoCodecImplementationSelections::Auto => match selection {
            VideoCodecImplementationSelection::Internal => true,
            VideoCodecImplementationSelection::InternalApple => {
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                {
                    available
                }
                #[cfg(not(any(target_os = "macos", target_os = "ios")))]
                {
                    false
                }
            }
            VideoCodecImplementationSelection::Amf
            | VideoCodecImplementationSelection::Nvcodec
            | VideoCodecImplementationSelection::Vpl
            | VideoCodecImplementationSelection::Openh264 => false,
        },
    }
}

fn probe_internal(args: &Args) -> VideoCodecCapabilityProbe {
    let capability: Box<dyn VideoCodecCapability> = Box::new(InternalVideoCodecCapability::new());
    let selected = is_selection_selected(
        args,
        VideoCodecImplementationSelection::Internal,
        has_any_codec_support(capability.as_ref()),
    );
    VideoCodecCapabilityProbe {
        selection: VideoCodecImplementationSelection::Internal,
        selected,
        capability: Some(capability),
        unavailable_reason: None,
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

fn probe_vpl(args: &Args) -> VideoCodecCapabilityProbe {
    #[cfg(feature = "vpl")]
    let (capability, unavailable_reason) = match VplVideoCodecCapability::new() {
        Ok(capability) => {
            let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
            if has_any_codec_support(capability.as_ref()) {
                (Some(capability), None)
            } else {
                (
                    None,
                    Some("VPL does not support any encoder or decoder codec".to_string()),
                )
            }
        }
        Err(err) => (None, Some(err.to_string())),
    };
    #[cfg(not(feature = "vpl"))]
    let (capability, unavailable_reason) = (
        None,
        Some("VPL is not enabled in this build. Rebuild sumomo with --features vpl".to_string()),
    );
    let selected = is_selection_selected(
        args,
        VideoCodecImplementationSelection::Vpl,
        capability.is_some(),
    );
    VideoCodecCapabilityProbe {
        selection: VideoCodecImplementationSelection::Vpl,
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
        probe_vpl(args),
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

pub(crate) fn collect_video_codec_list_report(args: &Args) -> VideoCodecListReport {
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

pub(crate) fn build_video_codec_list_report_text(
    report: &VideoCodecListReport,
    ansi_enabled: bool,
) -> String {
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

pub(crate) fn run_video_codec_list(args: &Args) -> Result<()> {
    // --video-codec-list 専用の収集と描画だけ実行する。
    let report = collect_video_codec_list_report(args);
    render_video_codec_list_report(&report);
    Ok(())
}
