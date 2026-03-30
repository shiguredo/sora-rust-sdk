use std::collections::HashMap;

use shiguredo_webrtc::{SdpVideoFormat, VideoCodecType, VideoDecoderHandler, VideoEncoderHandler};

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
    fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool;
    fn resolve_sdp_format(
        &self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
        parameters: &HashMap<String, String>,
        scalability_mode: Option<&str>,
    ) -> Option<SdpVideoFormat>;
    fn create_video_encoder(&self, format: &SdpVideoFormat)
    -> Option<Box<dyn VideoEncoderHandler>>;
    fn create_video_decoder(&self, format: &SdpVideoFormat)
    -> Option<Box<dyn VideoDecoderHandler>>;
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

    struct StubVideoEncoder;
    impl VideoEncoderHandler for StubVideoEncoder {}

    struct StubVideoDecoder;
    impl VideoDecoderHandler for StubVideoDecoder {}

    struct MockCapability;

    impl VideoCodecCapability for MockCapability {
        fn get_implementation(&self) -> VideoCodecImplementation {
            VideoCodecImplementation::new("mock", "Mock Codec")
        }

        fn is_supported(&self, _direction: CodecDirection, codec_type: VideoCodecType) -> bool {
            codec_type == VideoCodecType::H264
        }

        fn resolve_sdp_format(
            &self,
            _direction: CodecDirection,
            codec_type: VideoCodecType,
            _parameters: &HashMap<String, String>,
            _scalability_mode: Option<&str>,
        ) -> Option<SdpVideoFormat> {
            if codec_type == VideoCodecType::H264 {
                Some(SdpVideoFormat::new("H264"))
            } else {
                None
            }
        }

        fn create_video_encoder(
            &self,
            format: &SdpVideoFormat,
        ) -> Option<Box<dyn VideoEncoderHandler>> {
            if format.name().ok().as_deref() == Some("H264") {
                Some(Box::new(StubVideoEncoder))
            } else {
                None
            }
        }

        fn create_video_decoder(
            &self,
            format: &SdpVideoFormat,
        ) -> Option<Box<dyn VideoDecoderHandler>> {
            if format.name().ok().as_deref() == Some("H264") {
                Some(Box::new(StubVideoDecoder))
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
        assert!(capability.create_video_encoder(&h264).is_some());
        assert!(capability.create_video_decoder(&h264).is_some());
    }
}
