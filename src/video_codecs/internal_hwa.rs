use shiguredo_webrtc::{
    EnvironmentRef, SdpVideoFormat, VideoDecoder, VideoDecoderFactory, VideoEncoder,
    VideoEncoderFactory,
};

use crate::video_codec::SimulcastEncoderAdapterSupport;
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

pub struct InternalHwaVideoCodecCapability {
    implementation: VideoCodecImplementation,
    simulcast_encoder_adapter_support: SimulcastEncoderAdapterSupport,
    decoder_factory: VideoDecoderFactory,
}

impl InternalHwaVideoCodecCapability {
    pub fn new() -> Option<Self> {
        let encoder_factory = VideoEncoderFactory::from_objc_default()?;
        let decoder_factory = VideoDecoderFactory::from_objc_default()?;
        Some(Self {
            implementation: VideoCodecImplementation::new(
                "internal-hwa",
                "WebRTC ObjC default VideoCodecFactory",
            ),
            simulcast_encoder_adapter_support: SimulcastEncoderAdapterSupport::new(encoder_factory),
            decoder_factory,
        })
    }
}

impl VideoCodecCapability for InternalHwaVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        self.implementation.clone()
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        match direction {
            CodecDirection::Encoder => self
                .simulcast_encoder_adapter_support
                .get_supported_formats(),
            CodecDirection::Decoder => self.decoder_factory.get_supported_formats(),
        }
    }

    fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: &SdpVideoFormat,
    ) -> Option<VideoEncoder> {
        self.simulcast_encoder_adapter_support
            .create_video_encoder(env, format)
    }

    fn create_video_decoder(
        &self,
        env: EnvironmentRef<'_>,
        format: &SdpVideoFormat,
    ) -> Option<VideoDecoder> {
        self.decoder_factory.create(env, format.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::VideoCodecType;

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

        let encoder_formats = capability.get_supported_formats(CodecDirection::Encoder);
        let env = shiguredo_webrtc::Environment::new();
        for format in &encoder_formats {
            assert!(
                capability
                    .create_video_encoder(env.as_ref(), format)
                    .is_some(),
                "encoder must be created for a supported format",
            );
        }

        let decoder_formats = capability.get_supported_formats(CodecDirection::Decoder);
        for format in &decoder_formats {
            assert!(
                capability
                    .create_video_decoder(env.as_ref(), format)
                    .is_some(),
                "decoder must be created for a supported format",
            );
        }
    }

    #[test]
    fn internal_hwa_capability_create_video_encoder_uses_simulcast_adapter() {
        let capability = InternalHwaVideoCodecCapability::new()
            .expect("InternalHwaVideoCodecCapability must be available");
        let Some(format) = capability
            .get_supported_formats(CodecDirection::Encoder)
            .into_iter()
            .next()
        else {
            return;
        };
        let env = shiguredo_webrtc::Environment::new();
        let encoder = capability
            .create_video_encoder(env.as_ref(), &format)
            .expect("encoder must be created for supported format");
        let info = encoder.get_encoder_info();
        let implementation_name = info
            .implementation_name()
            .expect("implementation_name の取得に失敗");
        assert!(
            implementation_name.contains("SimulcastEncoderAdapter"),
            "adapter encoder では SimulcastEncoderAdapter を含む実装名が必要: {implementation_name}",
        );
    }
}
