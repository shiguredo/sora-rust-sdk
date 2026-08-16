//! 各ハードウェアビデオコーデック実装が共通で使うヘルパー関数。
//!
//! 各バックエンドの実装で重複していた関数を 1 箇所に集約する。
//! `shiguredo_webrtc` の型に依存するが、バックエンド固有の型
//! （`shiguredo_vpl::CodecConfig` 等）には依存しない。
#[cfg(any(feature = "vpl", feature = "amf", feature = "nvcodec"))]
use std::collections::HashMap;

use shiguredo_webrtc::{VideoFrameType, VideoFrameTypeVectorRef};

#[cfg(any(feature = "vpl", feature = "amf", feature = "nvcodec"))]
use shiguredo_webrtc::{ScalabilityMode, SdpVideoFormat, VideoCodecType};

/// エンコード要求されたフレームタイプのうち、最初の要素を返す。
///
/// WebRTC の `Encode()` に渡される frame types は複数要素を持ち得るが、
/// 本 SDK の各バックエンドは先頭要素だけを参照する。
pub(crate) fn requested_frame_type(
    frame_types: Option<VideoFrameTypeVectorRef<'_>>,
) -> Option<VideoFrameType> {
    frame_types.and_then(|frame_types| frame_types.get(0))
}

/// 指定したコーデックの SDP フォーマット一覧を返す。
///
/// H.264 は level-asymmetry-allowed / packetization-mode を付与する。
/// VP9 は SDP で明示的にプロファイルを指定する方が安全なため `profile-id=0` を付与する。
/// 各バックエンドが対応していないコーデックは `collect_supported_formats` 側で
/// ハードウェアの対応状況により呼び出されないため、ここに全コーデックを列挙する。
///
/// H.264 専用の v4l2 / openh264 からは利用しないため、それらの feature のみの
/// ビルドでは未使用にならないように feature 条件でコンパイルを制御する。
#[cfg(any(feature = "vpl", feature = "amf", feature = "nvcodec"))]
pub(crate) fn supported_formats_for_codec(codec_type: VideoCodecType) -> Vec<SdpVideoFormat> {
    match codec_type {
        VideoCodecType::H264 => vec![SdpVideoFormat::new_with_parameters(
            "H264",
            &HashMap::from([
                (String::from("level-asymmetry-allowed"), String::from("1")),
                (String::from("packetization-mode"), String::from("1")),
            ]),
            &[ScalabilityMode::L1T1],
        )],
        VideoCodecType::H265 => vec![SdpVideoFormat::new("H265")],
        VideoCodecType::Vp9 => vec![SdpVideoFormat::new_with_parameters(
            "VP9",
            &HashMap::from([(String::from("profile-id"), String::from("0"))]),
            &[],
        )],
        VideoCodecType::Av1 => vec![SdpVideoFormat::new("AV1")],
        VideoCodecType::Vp8 => vec![SdpVideoFormat::new("VP8")],
        _ => Vec::new(),
    }
}

/// ビットレート (bps) を kbps に変換する。
///
/// 0 は 1 にクリップする。戻り値は `u32` で、呼び出し元の API が要求する型
/// （VPL の `u16` 等）への変換は呼び出し元で行う。
///
/// v4l2 / nvcodec / openh264 からは利用しないため、それらの feature のみの
/// ビルドでは未使用にならないように feature 条件でコンパイルを制御する。
#[cfg(any(feature = "vpl", feature = "amf"))]
pub(crate) fn target_kbps_from_bps(target_bitrate_bps: u32) -> u32 {
    (target_bitrate_bps.max(1) as u64).div_ceil(1000) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::VideoFrameTypeVector;

    #[test]
    fn requested_frame_type_uses_first_entry() {
        assert_eq!(requested_frame_type(None), None);

        let mut frame_types = VideoFrameTypeVector::new(2);
        frame_types.push(VideoFrameType::Empty);
        frame_types.push(VideoFrameType::Key);
        assert_eq!(
            requested_frame_type(Some(frame_types.as_ref())),
            Some(VideoFrameType::Empty)
        );
    }

    #[cfg(any(feature = "vpl", feature = "amf", feature = "nvcodec"))]
    #[test]
    fn supported_formats_for_codec_covers_all_codecs() {
        let mut h264 = supported_formats_for_codec(VideoCodecType::H264);
        assert_eq!(h264.len(), 1);
        let params = h264[0]
            .parameters_mut()
            .iter()
            .collect::<HashMap<String, String>>();
        assert_eq!(
            params.get("packetization-mode").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            params.get("level-asymmetry-allowed").map(String::as_str),
            Some("1")
        );

        assert_eq!(
            supported_formats_for_codec(VideoCodecType::H265)[0]
                .name()
                .expect("format name の取得に失敗"),
            "H265"
        );

        // VP9 は SDP で profile-id=0 を明示する。
        let mut vp9 = supported_formats_for_codec(VideoCodecType::Vp9);
        let vp9_params = vp9[0]
            .parameters_mut()
            .iter()
            .collect::<HashMap<String, String>>();
        assert_eq!(vp9_params.get("profile-id").map(String::as_str), Some("0"));

        assert_eq!(
            supported_formats_for_codec(VideoCodecType::Av1)[0]
                .name()
                .expect("format name の取得に失敗"),
            "AV1"
        );
        assert_eq!(
            supported_formats_for_codec(VideoCodecType::Vp8)[0]
                .name()
                .expect("format name の取得に失敗"),
            "VP8"
        );
    }

    #[cfg(any(feature = "vpl", feature = "amf"))]
    #[test]
    fn target_kbps_from_bps_rounds_up_and_clamps_zero() {
        assert_eq!(target_kbps_from_bps(1_000), 1);
        assert_eq!(target_kbps_from_bps(999), 1);
        assert_eq!(target_kbps_from_bps(1_000_001), 1001);
        // 0 は 1 kbps にクリップする。
        assert_eq!(target_kbps_from_bps(0), 1);
    }
}
