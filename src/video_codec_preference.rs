use std::collections::HashSet;

use shiguredo_webrtc::{SdpVideoFormat, VideoCodecType};

use nojson::{DisplayJson, Json, JsonFormatter, JsonParseError, RawJsonValue};

use crate::error::{Error, Result};
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferenceCodec {
    direction: CodecDirection,
    codec_type: VideoCodecType,
    implementation: VideoCodecImplementation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoCodecPreference {
    codecs: Vec<PreferenceCodec>,
}

impl PreferenceCodec {
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

    pub fn direction(&self) -> CodecDirection {
        self.direction
    }

    pub fn codec_type(&self) -> VideoCodecType {
        self.codec_type
    }

    pub fn implementation(&self) -> &VideoCodecImplementation {
        &self.implementation
    }

    pub fn set_implementation(&mut self, implementation: VideoCodecImplementation) {
        self.implementation = implementation;
    }
}

impl VideoCodecPreference {
    pub fn new(codecs: Vec<PreferenceCodec>) -> Self {
        Self { codecs }
    }

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

    pub fn codecs(&self) -> &[PreferenceCodec] {
        &self.codecs
    }

    pub fn find(
        &self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
    ) -> Option<&PreferenceCodec> {
        self.codecs
            .iter()
            .find(|codec| codec.direction == direction && codec.codec_type == codec_type)
    }

    pub fn find_mut(
        &mut self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
    ) -> Option<&mut PreferenceCodec> {
        self.codecs
            .iter_mut()
            .find(|codec| codec.direction == direction && codec.codec_type == codec_type)
    }

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

    pub fn has_implementation(&self, implementation: VideoCodecImplementation) -> bool {
        self.codecs
            .iter()
            .any(|codec| codec.implementation == implementation)
    }

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

pub fn validate_video_codec_preference(
    preference: &VideoCodecPreference,
    capabilities: &Vec<Box<dyn VideoCodecCapability>>,
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

    if capability
        .resolve_sdp_format(
            codec.direction(),
            &SdpVideoFormat::new(
                codec
                    .codec_type()
                    .as_str()
                    .expect("known codec type must be converted to codec name"),
            ),
        )
        .is_none()
    {
        return Err(Error::InvalidVideoCodecPreference {
            reason: format!(
                "codec format not found: codec_preference={}, codec_capability={}",
                Json(codec),
                codec_capability_summary(capability, codec.codec_type())
            ),
        });
    }

    Ok(())
}

fn find_capability<'a>(
    capabilities: &'a [Box<dyn VideoCodecCapability>],
    implementation: &VideoCodecImplementation,
) -> Option<&'a dyn VideoCodecCapability> {
    capabilities
        .iter()
        .map(|capability| capability.as_ref())
        .find(|capability| capability.get_implementation() == implementation.clone())
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
    use shiguredo_webrtc::{
        EnvironmentRef, SdpVideoFormat, VideoDecoder, VideoDecoderHandler, VideoEncoder,
        VideoEncoderHandler,
    };

    struct StubVideoEncoder;
    impl VideoEncoderHandler for StubVideoEncoder {}

    struct StubVideoDecoder;
    impl VideoDecoderHandler for StubVideoDecoder {}

    struct MockVideoCodecCapability {
        implementation: VideoCodecImplementation,
        encoder_supported: Vec<VideoCodecType>,
        decoder_supported: Vec<VideoCodecType>,
    }

    impl MockVideoCodecCapability {
        fn new(
            implementation: VideoCodecImplementation,
            encoder_supported: Vec<VideoCodecType>,
            decoder_supported: Vec<VideoCodecType>,
        ) -> Self {
            Self {
                implementation,
                encoder_supported,
                decoder_supported,
            }
        }
    }

    impl VideoCodecCapability for MockVideoCodecCapability {
        fn get_implementation(&self) -> VideoCodecImplementation {
            self.implementation.clone()
        }

        fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
            match direction {
                CodecDirection::Encoder => self.encoder_supported.contains(&codec_type),
                CodecDirection::Decoder => self.decoder_supported.contains(&codec_type),
            }
        }

        fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
            let supported = match direction {
                CodecDirection::Encoder => &self.encoder_supported,
                CodecDirection::Decoder => &self.decoder_supported,
            };
            supported
                .iter()
                .filter_map(|codec_type| codec_type.as_str().map(SdpVideoFormat::new))
                .collect()
        }

        fn resolve_sdp_format(
            &self,
            direction: CodecDirection,
            format: &SdpVideoFormat,
        ) -> Option<shiguredo_webrtc::SdpVideoFormat> {
            let codec_type = format
                .name()
                .ok()
                .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
            let supported = self.is_supported(direction, codec_type);
            if !supported {
                return None;
            }
            let codec_name = codec_type.as_str()?;
            let mut format = shiguredo_webrtc::SdpVideoFormat::new(codec_name);
            if codec_type == VideoCodecType::H264 {
                format.parameters_mut().set("packetization-mode", "1");
            }
            Some(format)
        }

        fn create_video_encoder(
            &self,
            _env: EnvironmentRef<'_>,
            format: &SdpVideoFormat,
        ) -> Option<VideoEncoder> {
            let codec_type = format
                .name()
                .ok()
                .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
            if self.is_supported(CodecDirection::Encoder, codec_type) {
                Some(VideoEncoder::new_with_handler(Box::new(StubVideoEncoder)))
            } else {
                None
            }
        }

        fn create_video_decoder(
            &self,
            _env: EnvironmentRef<'_>,
            format: &SdpVideoFormat,
        ) -> Option<VideoDecoder> {
            let codec_type = format
                .name()
                .ok()
                .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
            if self.is_supported(CodecDirection::Decoder, codec_type) {
                Some(VideoDecoder::new_with_handler(Box::new(StubVideoDecoder)))
            } else {
                None
            }
        }
    }

    fn default_preference_codec(
        direction: CodecDirection,
        codec_type: VideoCodecType,
        implementation: VideoCodecImplementation,
    ) -> PreferenceCodec {
        PreferenceCodec::new(direction, codec_type, implementation)
    }

    fn sample_capabilities() -> Vec<Box<dyn VideoCodecCapability>> {
        vec![Box::new(MockVideoCodecCapability::new(
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
        let capability = MockVideoCodecCapability::new(
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
        let parsed: Json<T> = json_text.parse().expect("failed to parse JSON");
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
        let parsed: Json<VideoCodecPreference> = json.parse().expect("failed to parse preference");
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
        let error =
            validate_video_codec_preference(&preference, &capabilities).expect_err("must fail");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert_eq!(reason, "duplicate H264 encoder");
            }
            other => panic!("unexpected error: {other:?}"),
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
        let error =
            validate_video_codec_preference(&preference, &capabilities).expect_err("must fail");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert!(reason.contains("encoder implementation not found"));
            }
            other => panic!("unexpected error: {other:?}"),
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
        let error =
            validate_video_codec_preference(&preference, &capabilities).expect_err("must fail");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert!(reason.contains("codec type not found"));
            }
            other => panic!("unexpected error: {other:?}"),
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
        let error =
            validate_video_codec_preference(&preference, &capabilities).expect_err("must fail");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert!(reason.contains("encoder not supported"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_fails_on_duplicate_capability_implementation() {
        let preference = sample_preference();
        let capabilities: Vec<Box<dyn VideoCodecCapability>> = vec![
            Box::new(MockVideoCodecCapability::new(
                VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC"),
                vec![VideoCodecType::H264],
                vec![VideoCodecType::H264],
            )),
            Box::new(MockVideoCodecCapability::new(
                VideoCodecImplementation::new("nvcodec", "another description"),
                vec![VideoCodecType::Vp8],
                vec![VideoCodecType::Vp8],
            )),
        ];
        let error =
            validate_video_codec_preference(&preference, &capabilities).expect_err("must fail");
        match error {
            Error::InvalidVideoCodecCapability { reason } => {
                assert!(reason.contains("duplicate implementation in capabilities"));
            }
            other => panic!("unexpected error: {other:?}"),
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
        let error =
            validate_video_codec_preference(&preference, &capabilities).expect_err("must fail");
        match error {
            Error::InvalidVideoCodecPreference { reason } => {
                assert_eq!(reason, "duplicate H264 encoder");
            }
            other => panic!("unexpected error: {other:?}"),
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
            .expect("h264 encoder must exist after merge");
        assert_eq!(h264_encoder.implementation().name(), "vpl");
        let h264_decoder = preference
            .find(CodecDirection::Decoder, VideoCodecType::H264)
            .expect("h264 decoder must exist after merge");
        assert_eq!(h264_decoder.implementation().name(), "nvcodec");
    }
}
