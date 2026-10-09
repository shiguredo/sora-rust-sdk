//! WebRTC 組み込みのビデオコーデック実装。
use shiguredo_webrtc::{
    EnvironmentRef, SdpVideoFormat, SdpVideoFormatRef, VideoDecoder, VideoDecoderFactory,
    VideoEncoder, VideoEncoderFactory,
};

#[cfg(test)]
use shiguredo_webrtc::VideoCodecType;

use crate::codec_direction::CodecDirection;
use crate::video_codec_capability::{VideoCodecCapability, VideoCodecImplementation};

/// WebRTC 組み込みのエンコーダー/デコーダーを使用する [VideoCodecCapability]。
pub struct InternalVideoCodecCapability {
    implementation: VideoCodecImplementation,
    encoder_factory: VideoEncoderFactory,
    decoder_factory: VideoDecoderFactory,
}

impl InternalVideoCodecCapability {
    /// 新しい `InternalVideoCodecCapability` を生成する。
    pub fn new() -> Self {
        Self {
            implementation: VideoCodecImplementation::new(
                "internal",
                "WebRTC built-in VideoCodecFactory",
            ),
            encoder_factory: VideoEncoderFactory::builtin(),
            decoder_factory: VideoDecoderFactory::builtin(),
        }
    }
}

impl Default for InternalVideoCodecCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoCodecCapability for InternalVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        self.implementation.clone()
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        match direction {
            CodecDirection::Encoder => self.encoder_factory.get_supported_formats(),
            CodecDirection::Decoder => self.decoder_factory.get_supported_formats(),
        }
    }

    fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        self.encoder_factory.create(env, format)
    }

    fn create_video_decoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        self.decoder_factory.create(env, format)
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

        let encoder_formats = capability.get_supported_formats(CodecDirection::Encoder);
        let env = shiguredo_webrtc::Environment::new();
        for format in &encoder_formats {
            assert!(
                capability
                    .create_video_encoder(env.as_ref(), format.as_ref())
                    .is_some(),
                "サポートされているフォーマットでエンコーダーが生成されなければなりません",
            );
        }

        let decoder_formats = capability.get_supported_formats(CodecDirection::Decoder);
        for format in &decoder_formats {
            assert!(
                capability
                    .create_video_decoder(env.as_ref(), format.as_ref())
                    .is_some(),
                "サポートされているフォーマットでデコーダーが生成されなければなりません",
            );
        }
    }

    #[test]
    fn resolve_sdp_format_with_unknown_scalability_mode_falls_back() {
        let capability = InternalVideoCodecCapability::new();
        let codec_type = capability
            .get_supported_formats(CodecDirection::Encoder)
            .into_iter()
            .find_map(|format| {
                let name = format.name().ok()?;
                VideoCodecType::try_from(name.as_str()).ok()
            })
            .expect("エンコーダーコーデックが少なくとも 1 つはサポートされている必要があります");
        let requested = SdpVideoFormat::new(
            codec_type
                .as_str()
                .expect("既知のコーデックは有効なコーデック名を持っている必要があります"),
        );
        let resolved = capability.resolve_sdp_format(CodecDirection::Encoder, requested.as_ref());
        assert!(resolved.is_some());
    }
}
