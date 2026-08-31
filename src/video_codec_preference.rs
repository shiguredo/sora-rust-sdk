//! ビデオコーデックの優先順位設定。
use std::collections::HashSet;

use shiguredo_webrtc::VideoCodecType;

use nojson::{DisplayJson, Json, JsonFormatter, JsonParseError, RawJsonValue};

use crate::codec_direction::CodecDirection;
use crate::error::{Error, Result};
use crate::video_codec_capability::{
    VideoCodecCapability, VideoCodecImplementation, find_capability,
};

/// 特定の方向・コーデック・実装の優先設定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferenceCodec {
    direction: CodecDirection,
    codec_type: VideoCodecType,
    implementation: VideoCodecImplementation,
}

/// ビデオコーデックの優先順位リスト。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoCodecPreference {
    codecs: Vec<PreferenceCodec>,
}

impl PreferenceCodec {
    /// [PreferenceCodec] を生成する。
    pub fn new(
        direction: CodecDirection,
        codec_type: VideoCodecType,
        implementation: VideoCodecImplementation,
    ) -> Self {
        Self {
            direction,
            codec_type,
            implementation,
        }
    }

    /// [CodecDirection] を返す。
    pub fn direction(&self) -> CodecDirection {
        self.direction
    }

    /// [VideoCodecType] を返す。
    pub fn codec_type(&self) -> VideoCodecType {
        self.codec_type
    }

    /// [VideoCodecImplementation] を返す。
    pub fn implementation(&self) -> &VideoCodecImplementation {
        &self.implementation
    }

    /// [VideoCodecImplementation] を上書きする。
    pub fn set_implementation(&mut self, implementation: VideoCodecImplementation) {
        self.implementation = implementation;
    }
}

impl VideoCodecPreference {
    /// [PreferenceCodec] のリストから [VideoCodecPreference] を生成する。
    pub fn new(codecs: Vec<PreferenceCodec>) -> Self {
        Self { codecs }
    }

    /// [VideoCodecCapability] から自動生成した優先設定を返す。
    pub fn new_from_capability(capability: &dyn VideoCodecCapability) -> Self {
        let implementation = capability.get_implementation();
        let mut codecs = Vec::new();
        for codec_type in [
            VideoCodecType::Vp8,
            VideoCodecType::Vp9,
            VideoCodecType::H264,
            VideoCodecType::H265,
            VideoCodecType::Av1,
        ] {
            for direction in [CodecDirection::Encoder, CodecDirection::Decoder] {
                if capability.is_supported(direction, codec_type) {
                    codecs.push(PreferenceCodec::new(
                        direction,
                        codec_type,
                        implementation.clone(),
                    ));
                }
            }
        }
        Self::new(codecs)
    }

    /// 全 [PreferenceCodec] を返す。
    pub fn codecs(&self) -> &[PreferenceCodec] {
        &self.codecs
    }

    /// 指定された方向とコーデック種別に合致する [PreferenceCodec] を検索する。
    pub fn find(
        &self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
    ) -> Option<&PreferenceCodec> {
        self.codecs
            .iter()
            .find(|codec| codec.direction == direction && codec.codec_type == codec_type)
    }

    /// 指定された方向とコーデック種別に合致する [PreferenceCodec] を可変参照で検索する。
    pub fn find_mut(
        &mut self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
    ) -> Option<&mut PreferenceCodec> {
        self.codecs
            .iter_mut()
            .find(|codec| codec.direction == direction && codec.codec_type == codec_type)
    }

    /// 指定された方向・コーデック・実装のエントリを取得し、なければ追加する。
    pub fn get_or_add(
        &mut self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
        implementation: VideoCodecImplementation,
    ) -> &mut PreferenceCodec {
        if let Some(index) = self
            .codecs
            .iter()
            .position(|codec| codec.direction == direction && codec.codec_type == codec_type)
        {
            return &mut self.codecs[index];
        }
        self.codecs
            .push(PreferenceCodec::new(direction, codec_type, implementation));
        self.codecs
            .last_mut()
            .expect("codecs must contain one element after push")
    }

    /// 指定された [VideoCodecImplementation] が含まれているかどうかを返す。
    pub fn has_implementation(&self, implementation: VideoCodecImplementation) -> bool {
        self.codecs
            .iter()
            .any(|codec| codec.implementation == implementation)
    }

    /// 別の [VideoCodecPreference] をマージする。
    ///
    /// 既存エントリと方向・コーデックが一致する項目は上書きし、
    /// 存在しない項目は追加する。
    pub fn merge(&mut self, preference: &VideoCodecPreference) {
        for codec in &preference.codecs {
            if let Some(existing) = self.find_mut(codec.direction, codec.codec_type) {
                existing.implementation = codec.implementation.clone();
            } else {
                self.codecs.push(codec.clone());
            }
        }
    }
}

impl DisplayJson for PreferenceCodec {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        let codec_type = video_codec_type_to_json_str(self.codec_type)?;
        f.object(|f| {
            f.member("direction", self.direction.as_str())?;
            f.member("codec_type", codec_type)?;
            f.member("implementation", &self.implementation)
        })
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for PreferenceCodec {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        let direction: CodecDirection = value.to_member("direction")?.required()?.try_into()?;
        let codec_type = parse_video_codec_type(value.to_member("codec_type")?.required()?)?;
        let implementation: VideoCodecImplementation =
            value.to_member("implementation")?.required()?.try_into()?;
        Ok(Self {
            direction,
            codec_type,
            implementation,
        })
    }
}

impl DisplayJson for VideoCodecPreference {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| f.member("codecs", &self.codecs))
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for VideoCodecPreference {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            codecs: value.to_member("codecs")?.required()?.try_into()?,
        })
    }
}

/// [VideoCodecPreference] の妥当性を検証する。
///
/// - 同じ方向・コーデック種別の重複がないこと
/// - 各エントリの実装が `capabilities` に存在すること
/// - 各エントリの方向・コーデックが実装でサポートされていること
pub fn validate_video_codec_preference(
    preference: &VideoCodecPreference,
    capabilities: &[Box<dyn VideoCodecCapability>],
) -> Result<()> {
    validate_capabilities(capabilities)?;

    for codec_type in [
        VideoCodecType::Vp8,
        VideoCodecType::Vp9,
        VideoCodecType::H264,
        VideoCodecType::H265,
        VideoCodecType::Av1,
    ] {
        for direction in [CodecDirection::Encoder, CodecDirection::Decoder] {
            let count = preference
                .codecs()
                .iter()
                .filter(|codec| codec.direction() == direction && codec.codec_type() == codec_type)
                .count();
            if count >= 2 {
                let codec_type_name =
                    video_codec_type_to_json_str(codec_type).expect("known codec type");
                return Err(Error::InvalidVideoCodecPreference {
                    reason: format!("duplicate {codec_type_name} {}", direction.as_label()),
                });
            }
        }
    }

    for codec in preference.codecs() {
        validate_codec(codec, capabilities)?;
    }

    Ok(())
}

fn validate_capabilities(capabilities: &[Box<dyn VideoCodecCapability>]) -> Result<()> {
    let mut implementation_names = HashSet::new();
    for capability in capabilities {
        let implementation = capability.get_implementation();
        let implementation_name = implementation.name().to_string();
        if !implementation_names.insert(implementation_name) {
            return Err(Error::InvalidVideoCodecCapability {
                reason: format!(
                    "duplicate implementation in capabilities: implementation={}",
                    Json(&implementation)
                ),
            });
        }
    }
    Ok(())
}

fn validate_codec(
    codec: &PreferenceCodec,
    capabilities: &[Box<dyn VideoCodecCapability>],
) -> Result<()> {
    let direction = codec.direction().as_label();
    let Some(capability) = find_capability(capabilities, codec.implementation()) else {
        return Err(Error::InvalidVideoCodecPreference {
            reason: format!(
                "{direction} implementation not found: codec_preference={}",
                Json(codec)
            ),
        });
    };
    // preference の可否判定は `is_supported` の結果で完結させる。
    // コーデック固有 parameter を必須とする capability も `is_supported` の
    // override で可否を表明することでこの検証を通せる。
    // 実 format の解決は `SoraVideoEncoderFactory::create` /
    // `SoraVideoDecoderFactory::create` が行う。
    let encoder_supported = capability.is_supported(CodecDirection::Encoder, codec.codec_type());
    let decoder_supported = capability.is_supported(CodecDirection::Decoder, codec.codec_type());
    let (direction_supported, opposite_supported) = match codec.direction() {
        CodecDirection::Encoder => (encoder_supported, decoder_supported),
        CodecDirection::Decoder => (decoder_supported, encoder_supported),
    };

    if !direction_supported && !opposite_supported {
        return Err(Error::InvalidVideoCodecPreference {
            reason: format!("codec type not found: codec_preference={}", Json(codec)),
        });
    }

    if !direction_supported {
        return Err(Error::InvalidVideoCodecPreference {
            reason: format!(
                "{direction} not supported: codec_preference={}, codec_capability={}",
                Json(codec),
                codec_capability_summary(capability, codec.codec_type())
            ),
        });
    }

    Ok(())
}

fn codec_capability_summary(
    capability: &dyn VideoCodecCapability,
    codec_type: VideoCodecType,
) -> String {
    let codec_type_name = video_codec_type_to_json_str(codec_type).unwrap_or("Unknown");
    format!(
        "{{\"implementation\":{},\"codec_type\":\"{}\",\"encoder_supported\":{},\"decoder_supported\":{}}}",
        Json(&capability.get_implementation()),
        codec_type_name,
        capability.is_supported(CodecDirection::Encoder, codec_type),
        capability.is_supported(CodecDirection::Decoder, codec_type),
    )
}

fn video_codec_type_to_json_str(
    codec_type: VideoCodecType,
) -> std::result::Result<&'static str, std::fmt::Error> {
    codec_type.as_str().ok_or(std::fmt::Error)
}

fn parse_video_codec_type(
    value: RawJsonValue<'_, '_>,
) -> std::result::Result<VideoCodecType, JsonParseError> {
    let codec_type_text: String = value.try_into()?;
    VideoCodecType::try_from(codec_type_text.as_str())
        .map_err(|_| value.invalid(format!("unsupported video codec type: {codec_type_text}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::testing::TestVideoCodecCapability;

    fn default_preference_codec(
        direction: CodecDirection,
        codec_type: VideoCodecType,
        implementation: VideoCodecImplementation,
    ) -> PreferenceCodec {
        PreferenceCodec::new(direction, codec_type, implementation)
    }

    fn sample_capabilities() -> Vec<Box<dyn VideoCodecCapability>> {
        vec![Box::new(TestVideoCodecCapability::new(
            VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            vec![VideoCodecType::H264, VideoCodecType::H265],
            vec![VideoCodecType::H264, VideoCodecType::Vp8],
        ))]
    }

    fn sample_preference() -> VideoCodecPreference {
        VideoCodecPreference::new(vec![
            default_preference_codec(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
            default_preference_codec(
                CodecDirection::Decoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
            default_preference_codec(
                CodecDirection::Decoder,
                VideoCodecType::Vp8,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
        ])
    }

    #[test]
    fn create_preference_from_single_capability() {
        let capability = TestVideoCodecCapability::new(
            VideoCodecImplementation::new("mock", "Mock Codec"),
            vec![VideoCodecType::H264, VideoCodecType::Vp8],
            vec![VideoCodecType::H264],
        );
        let preference = VideoCodecPreference::new_from_capability(&capability);
        assert!(
            preference
                .find(CodecDirection::Encoder, VideoCodecType::H264)
                .is_some()
        );
        assert!(
            preference
                .find(CodecDirection::Decoder, VideoCodecType::H264)
                .is_some()
        );
        assert!(
            preference
                .find(CodecDirection::Encoder, VideoCodecType::Vp8)
                .is_some()
        );
        assert!(
            preference
                .find(CodecDirection::Decoder, VideoCodecType::Vp8)
                .is_none()
        );
    }

    fn assert_round_trip<T>(value: T)
    where
        T: Clone
            + std::fmt::Debug
            + PartialEq
            + DisplayJson
            + for<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>, Error = JsonParseError>,
    {
        let json_text = Json(&value).to_string();
        let parsed: Json<T> = json_text.parse().expect("JSON のパースに失敗しました");
        assert_eq!(parsed.0, value);
    }

    #[test]
    fn preference_codec_round_trip() {
        let codec = PreferenceCodec::new(
            CodecDirection::Encoder,
            VideoCodecType::H264,
            VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
        );
        assert_round_trip(codec);
    }

    #[test]
    fn preference_round_trip() {
        assert_round_trip(sample_preference());
    }

    #[test]
    fn deserialize_ignores_unknown_fields() {
        let json = r#"{
            "codecs":[
                {
                    "codec_type":"H264",
                    "direction":"Encoder",
                    "implementation":{"name":"nvcodec","description":"NVIDIA NVENC/NVDEC","x":1},
                    "unknown":"ignored"
                }
            ],
            "unknown":"ignored"
        }"#;
        let parsed: Json<VideoCodecPreference> =
            json.parse().expect("preference のパースに失敗しました");
        assert_eq!(parsed.0.codecs().len(), 1);
    }

    #[test]
    fn old_json_format_is_rejected() {
        let old_json = r#"{
            "codecs":[
                {
                    "codec_type":"H264",
                    "encoder":{"name":"nvcodec","description":"NVIDIA NVENC/NVDEC"},
                    "parameters":{}
                }
            ]
        }"#;
        assert!(old_json.parse::<Json<VideoCodecPreference>>().is_err());
    }

    #[test]
    fn validate_succeeds_with_supported_capabilities() {
        let preference = sample_preference();
        let capabilities = sample_capabilities();
        assert!(validate_video_codec_preference(&preference, &capabilities).is_ok());
    }

    // preference 検証は `is_supported` だけを見る。
    // `resolve_sdp_format` が bare format を拒否しても、指定方向が supported なら通る。
    #[test]
    fn validate_succeeds_when_supported_even_if_resolve_sdp_format_is_none() {
        let preference = VideoCodecPreference::new(vec![default_preference_codec(
            CodecDirection::Encoder,
            VideoCodecType::H264,
            VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
        )]);
        let capabilities: Vec<Box<dyn VideoCodecCapability>> = vec![Box::new(
            TestVideoCodecCapability::new(
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
                vec![VideoCodecType::H264],
                Vec::new(),
            )
            .without_sdp_format_resolution(),
        )];
        assert!(
            validate_video_codec_preference(&preference, &capabilities).is_ok(),
            "is_supported が true なら resolve が None でも検証は成功するはずです"
        );
    }

    #[test]
    fn validate_accepts_slice_capabilities() {
        let preference = sample_preference();
        let capabilities = sample_capabilities();
        let capabilities_slice: &[Box<dyn VideoCodecCapability>] = capabilities.as_slice();
        assert!(validate_video_codec_preference(&preference, capabilities_slice).is_ok());
    }

    #[test]
    fn validate_fails_on_duplicate_codec_type_and_direction() {
        let preference = VideoCodecPreference::new(vec![
            default_preference_codec(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
            default_preference_codec(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("vpl", "Intel VPL"),
            ),
            default_preference_codec(
                CodecDirection::Decoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
            default_preference_codec(
                CodecDirection::Decoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("vpl", "Intel VPL"),
            ),
        ]);
        let capabilities = sample_capabilities();
        let error = validate_video_codec_preference(&preference, &capabilities)
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert_eq!(reason, "duplicate H264 encoder");
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_fails_when_implementation_not_found() {
        let preference = VideoCodecPreference::new(vec![
            default_preference_codec(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("vpl", "Intel VPL"),
            ),
            default_preference_codec(
                CodecDirection::Decoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("vpl", "Intel VPL"),
            ),
        ]);
        let capabilities = sample_capabilities();
        let error = validate_video_codec_preference(&preference, &capabilities)
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert!(reason.contains("encoder implementation not found"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_fails_when_codec_type_not_found() {
        let preference = VideoCodecPreference::new(vec![default_preference_codec(
            CodecDirection::Encoder,
            VideoCodecType::Av1,
            VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
        )]);
        let capabilities = sample_capabilities();
        let error = validate_video_codec_preference(&preference, &capabilities)
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert!(reason.contains("codec type not found"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_fails_when_encoder_or_decoder_not_supported() {
        let preference = VideoCodecPreference::new(vec![
            default_preference_codec(
                CodecDirection::Encoder,
                VideoCodecType::Vp8,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
            default_preference_codec(
                CodecDirection::Decoder,
                VideoCodecType::H265,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
        ]);
        let capabilities = sample_capabilities();
        let error = validate_video_codec_preference(&preference, &capabilities)
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert!(reason.contains("encoder not supported"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_fails_on_duplicate_capability_implementation() {
        let preference = sample_preference();
        let capabilities: Vec<Box<dyn VideoCodecCapability>> = vec![
            Box::new(TestVideoCodecCapability::new(
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
                vec![VideoCodecType::H264],
                vec![VideoCodecType::H264],
            )),
            Box::new(TestVideoCodecCapability::new(
                VideoCodecImplementation::new("nvcodec", "another description"),
                vec![VideoCodecType::Vp8],
                vec![VideoCodecType::Vp8],
            )),
        ];
        let error = validate_video_codec_preference(&preference, &capabilities)
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidVideoCodecCapability { reason } => {
                assert!(reason.contains("duplicate implementation in capabilities"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_stops_on_first_error() {
        let preference = VideoCodecPreference::new(vec![
            default_preference_codec(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("vpl", "Intel VPL"),
            ),
            default_preference_codec(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
        ]);
        let capabilities = sample_capabilities();
        let error = validate_video_codec_preference(&preference, &capabilities)
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert_eq!(reason, "duplicate H264 encoder");
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn get_or_add_has_implementation_and_merge_work() {
        let mut preference = VideoCodecPreference::default();
        let codec = preference.get_or_add(
            CodecDirection::Encoder,
            VideoCodecType::H264,
            VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
        );
        codec.set_implementation(VideoCodecImplementation::new(
            "nvcodec",
            "NVIDIA NVENC/NVDEC",
        ));
        assert!(preference.has_implementation(VideoCodecImplementation::new(
            "nvcodec",
            "NVIDIA NVENC/NVDEC"
        )));

        let merged = VideoCodecPreference::new(vec![
            default_preference_codec(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("vpl", "Intel VPL"),
            ),
            default_preference_codec(
                CodecDirection::Decoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
            ),
        ]);
        preference.merge(&merged);
        let h264_encoder = preference
            .find(CodecDirection::Encoder, VideoCodecType::H264)
            .expect("マージ後に h264 エンコーダーが存在する必要があります");
        assert_eq!(h264_encoder.implementation().name(), "vpl");
        let h264_decoder = preference
            .find(CodecDirection::Decoder, VideoCodecType::H264)
            .expect("マージ後に h264 デコーダーが存在する必要があります");
        assert_eq!(h264_decoder.implementation().name(), "nvcodec");
    }
}
