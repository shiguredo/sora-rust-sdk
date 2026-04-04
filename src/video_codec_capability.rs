use shiguredo_webrtc::{
    EnvironmentRef, SdpVideoFormat, VideoCodecType, VideoDecoder, VideoEncoder,
    fuzzy_match_sdp_video_format,
};

use nojson::{DisplayJson, JsonFormatter, JsonParseError, RawJsonValue};

#[derive(Debug, Clone, Eq)]
pub struct VideoCodecImplementation {
    name: String,
    description: String,
}

impl VideoCodecImplementation {
    pub fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
        }
    }

    fn new_internal(name: String, description: String) -> Self {
        Self { name, description }
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn description(&self) -> &str {
        self.description.as_str()
    }
}

impl PartialEq for VideoCodecImplementation {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecDirection {
    Encoder,
    Decoder,
}

impl CodecDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Encoder => "Encoder",
            Self::Decoder => "Decoder",
        }
    }

    pub fn as_label(self) -> &'static str {
        match self {
            Self::Encoder => "encoder",
            Self::Decoder => "decoder",
        }
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for CodecDirection {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        let direction_text: String = value.try_into()?;
        match direction_text.as_str() {
            "Encoder" => Ok(Self::Encoder),
            "Decoder" => Ok(Self::Decoder),
            _ => Err(value.invalid(format!("unsupported codec direction: {direction_text}"))),
        }
    }
}

pub trait VideoCodecCapability: Send {
    fn get_implementation(&self) -> VideoCodecImplementation;
    /// 指定した方向とコーデックタイプでサポートされている SDP フォーマットのリストを返す。
    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat>;
    fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
        let Some(codec_name) = codec_type.as_str() else {
            return false;
        };
        let requested = SdpVideoFormat::new(codec_name);
        self.resolve_sdp_format(direction, &requested).is_some()
    }
    fn resolve_sdp_format(
        &self,
        direction: CodecDirection,
        format: &SdpVideoFormat,
    ) -> Option<SdpVideoFormat> {
        fuzzy_match_sdp_video_format(&self.get_supported_formats(direction), format.as_ref())
    }
    /// 指定したフォーマットでエンコーダーがサポートされている場合は VideoEncoder を返す。
    ///
    /// create_video_encoder() は get_supported_formats() で返されるフォーマットのいずれかとマッチするフォーマットで呼び出されることが想定されている。
    /// get_supported_formats() で返されないフォーマットで呼び出された場合の動作は実装に依存するため、None が返されることは保証されない。
    #[expect(unused_variables)]
    fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: &SdpVideoFormat,
    ) -> Option<VideoEncoder> {
        None
    }
    /// 指定したフォーマットでデコーダーがサポートされている場合は VideoDecoder を返す。
    ///
    /// create_video_decoder() は get_supported_formats() で返されるフォーマットのいずれかとマッチするフォーマットで呼び出されることが想定されている。
    /// get_supported_formats() で返されないフォーマットで呼び出された場合の動作は実装に依存するため、None が返されることは保証されない。
    #[expect(unused_variables)]
    fn create_video_decoder(
        &self,
        env: EnvironmentRef<'_>,
        format: &SdpVideoFormat,
    ) -> Option<VideoDecoder> {
        None
    }
}

impl DisplayJson for VideoCodecImplementation {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("name", &self.name)?;
            f.member("description", &self.description)
        })
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for VideoCodecImplementation {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        let name: String = value.to_member("name")?.required()?.try_into()?;
        let description: String = value.to_member("description")?.required()?.try_into()?;
        Ok(Self::new_internal(name, description))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nojson::Json;
    use shiguredo_webrtc::{VideoDecoderHandler, VideoEncoderHandler};

    struct StubVideoEncoder;
    impl VideoEncoderHandler for StubVideoEncoder {}

    struct StubVideoDecoder;
    impl VideoDecoderHandler for StubVideoDecoder {}

    struct MockCapability;

    impl VideoCodecCapability for MockCapability {
        fn get_implementation(&self) -> VideoCodecImplementation {
            VideoCodecImplementation::new("mock", "Mock Codec")
        }

        fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
            match direction {
                CodecDirection::Encoder | CodecDirection::Decoder => {
                    vec![SdpVideoFormat::new("H264")]
                }
            }
        }

        fn create_video_encoder(
            &self,
            _env: EnvironmentRef<'_>,
            format: &SdpVideoFormat,
        ) -> Option<VideoEncoder> {
            if format.name().ok().as_deref() == Some("H264") {
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
            if format.name().ok().as_deref() == Some("H264") {
                Some(VideoDecoder::new_with_handler(Box::new(StubVideoDecoder)))
            } else {
                None
            }
        }
    }

    #[test]
    fn video_codec_implementation_round_trip() {
        let value = VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC");
        let json_text = Json(&value).to_string();
        let parsed: Json<VideoCodecImplementation> =
            json_text.parse().expect("failed to parse JSON");
        assert_eq!(parsed.0, value);
    }

    #[test]
    fn trait_works_with_trait_object() {
        let capability: Box<dyn VideoCodecCapability> = Box::new(MockCapability);
        assert_eq!(capability.get_implementation().name(), "mock");
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));
        let h264 = SdpVideoFormat::new("H264");
        let env = shiguredo_webrtc::Environment::new();
        assert!(
            capability
                .create_video_encoder(env.as_ref(), &h264)
                .is_some()
        );
        assert!(
            capability
                .create_video_decoder(env.as_ref(), &h264)
                .is_some()
        );
    }
}
