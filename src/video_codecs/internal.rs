use std::collections::HashMap;

use shiguredo_webrtc::{SdpVideoFormat, VideoCodecType, VideoDecoder, VideoEncoder};
use shiguredo_webrtc::{VideoDecoderFactory, VideoEncoderFactory};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

use super::internal_factory::FactoryBackedVideoCodecCapability;

pub struct InternalVideoCodecCapability {
    inner: FactoryBackedVideoCodecCapability,
}

impl InternalVideoCodecCapability {
    pub fn new() -> Self {
        let inner = FactoryBackedVideoCodecCapability::new(
            VideoCodecImplementation::new("internal", "WebRTC built-in VideoCodecFactory"),
            VideoEncoderFactory::builtin(),
            VideoDecoderFactory::builtin(),
        );
        Self { inner }
    }
}

impl Default for InternalVideoCodecCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoCodecCapability for InternalVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        self.inner.get_implementation()
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Option<Vec<SdpVideoFormat>> {
        Some(self.inner.get_supported_formats(direction))
    }

    fn resolve_sdp_format(
        &self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
        parameters: &HashMap<String, String>,
        scalability_mode: Option<&str>,
    ) -> Option<SdpVideoFormat> {
        self.inner
            .resolve_sdp_format(direction, codec_type, parameters, scalability_mode)
    }

    fn create_video_encoder(&self, format: &SdpVideoFormat) -> Option<VideoEncoder> {
        self.inner.create_video_encoder(format)
    }

    fn create_video_decoder(&self, format: &SdpVideoFormat) -> Option<VideoDecoder> {
        self.inner.create_video_decoder(format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_capability_has_expected_implementation_name() {
        let capability = InternalVideoCodecCapability::new();
        assert_eq!(capability.get_implementation().name(), "internal");
    }

    #[test]
    fn internal_capability_creates_supported_encoder_and_decoder() {
        let capability = InternalVideoCodecCapability::new();

        let encoder_formats = capability
            .get_supported_formats(CodecDirection::Encoder)
            .expect("encoder formats must be available");
        for format in &encoder_formats {
            assert!(
                capability.create_video_encoder(format).is_some(),
                "encoder must be created for a supported format",
            );
        }

        let decoder_formats = capability
            .get_supported_formats(CodecDirection::Decoder)
            .expect("decoder formats must be available");
        for format in &decoder_formats {
            assert!(
                capability.create_video_decoder(format).is_some(),
                "decoder must be created for a supported format",
            );
        }
    }

    #[test]
    fn resolve_sdp_format_with_unknown_scalability_mode_falls_back() {
        let capability = InternalVideoCodecCapability::new();
        let codec_type = capability
            .get_supported_formats(CodecDirection::Encoder)
            .expect("encoder formats must be available")
            .into_iter()
            .find_map(|format| {
                let name = format.name().ok()?;
                VideoCodecType::try_from(name.as_str()).ok()
            })
            .expect("at least one encoder codec must be supported");
        let resolved = capability.resolve_sdp_format(
            CodecDirection::Encoder,
            codec_type,
            &HashMap::new(),
            Some("INVALID_MODE"),
        );
        assert!(resolved.is_some());
    }
}
