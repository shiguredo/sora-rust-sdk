use std::collections::HashMap;

use shiguredo_webrtc::{SdpVideoFormat, VideoCodecType, VideoDecoder, VideoEncoder};
use shiguredo_webrtc::{VideoDecoderFactory, VideoEncoderFactory};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

use super::internal_factory::FactoryBackedVideoCodecCapability;

pub struct InternalHwaVideoCodecCapability {
    inner: FactoryBackedVideoCodecCapability,
}

impl InternalHwaVideoCodecCapability {
    pub fn new() -> Option<Self> {
        let encoder_factory = VideoEncoderFactory::from_objc_default()?;
        let decoder_factory = VideoDecoderFactory::from_objc_default()?;
        let inner = FactoryBackedVideoCodecCapability::new(
            VideoCodecImplementation::new("internal-hwa", "WebRTC ObjC default VideoCodecFactory"),
            encoder_factory,
            decoder_factory,
        );
        Some(Self { inner })
    }
}

impl VideoCodecCapability for InternalHwaVideoCodecCapability {
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
    fn internal_hwa_capability_is_available() {
        assert!(
            InternalHwaVideoCodecCapability::new().is_some(),
            "InternalHwaVideoCodecCapability must be available on Apple platforms",
        );
    }

    #[test]
    fn internal_hwa_capability_has_expected_implementation_name() {
        let capability = InternalHwaVideoCodecCapability::new()
            .expect("InternalHwaVideoCodecCapability must be available");
        assert_eq!(capability.get_implementation().name(), "internal-hwa");
    }

    #[test]
    fn internal_hwa_capability_supports_h264_h265() {
        let capability = InternalHwaVideoCodecCapability::new()
            .expect("InternalHwaVideoCodecCapability must be available");
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));

        // Apple WebRTC バイナリによって H265 の有無が変わるため、
        // ここではクラッシュせず問い合わせできることだけを確認する。
        let _ = capability.is_supported(CodecDirection::Encoder, VideoCodecType::H265);
        let _ = capability.is_supported(CodecDirection::Decoder, VideoCodecType::H265);
    }

    #[test]
    fn internal_hwa_capability_creates_supported_encoder_and_decoder() {
        let capability = InternalHwaVideoCodecCapability::new()
            .expect("InternalHwaVideoCodecCapability must be available");

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
}
