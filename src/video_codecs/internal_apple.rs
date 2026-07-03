//! Apple プラットフォーム向けのビデオコーデック実装。
//!
//! `#[cfg(any(target_os = "macos", target_os = "ios"))]` でのみコンパイルされる。
use shiguredo_webrtc::{
    EnvironmentRef, SdpVideoFormat, SdpVideoFormatRef, VideoDecoder, VideoDecoderFactory,
    VideoEncoder, VideoEncoderFactory,
};

use crate::video_codec::SimulcastCapabilityHelper;
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

/// Apple プラットフォームの ObjC デフォルトビデオコーデックを使用する [VideoCodecCapability]。
///
/// `#[cfg(any(target_os = "macos", target_os = "ios"))]` でのみ利用可能。
pub struct InternalAppleVideoCodecCapability {
    implementation: VideoCodecImplementation,
    simulcast_capability_helper: SimulcastCapabilityHelper,
    decoder_factory: VideoDecoderFactory,
}

impl InternalAppleVideoCodecCapability {
    /// 新しい `InternalAppleVideoCodecCapability` を生成する。
    ///
    /// 利用可能なハードウェアエンコーダー/デコーダーが存在しない場合は `None` を返す。
    pub fn new() -> Option<Self> {
        let encoder_factory = VideoEncoderFactory::from_objc_default()?;
        let decoder_factory = VideoDecoderFactory::from_objc_default()?;
        Some(Self {
            implementation: VideoCodecImplementation::new(
                "internal-apple",
                "WebRTC ObjC default VideoCodecFactory",
            ),
            simulcast_capability_helper: SimulcastCapabilityHelper::new(encoder_factory),
            decoder_factory,
        })
    }
}

impl VideoCodecCapability for InternalAppleVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        self.implementation.clone()
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        match direction {
            CodecDirection::Encoder => self.simulcast_capability_helper.get_supported_formats(),
            CodecDirection::Decoder => self.decoder_factory.get_supported_formats(),
        }
    }

    fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        self.simulcast_capability_helper
            .create_video_encoder(env, format)
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
    use shiguredo_webrtc::VideoCodecType;

    #[test]
    fn internal_apple_capability_is_available() {
        assert!(
            InternalAppleVideoCodecCapability::new().is_some(),
            "InternalAppleVideoCodecCapability must be available on Apple platforms",
        );
    }

    #[test]
    fn internal_apple_capability_has_expected_implementation_name() {
        let capability = InternalAppleVideoCodecCapability::new()
            .expect("InternalAppleVideoCodecCapability must be available");
        assert_eq!(capability.get_implementation().name(), "internal-apple");
    }

    #[test]
    fn internal_apple_capability_supports_h264_h265() {
        let capability = InternalAppleVideoCodecCapability::new()
            .expect("InternalAppleVideoCodecCapability must be available");
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));

        // 利用している Apple のハードウェアによって H265 HWA の有無が変わるため、
        // 存在の有無は気にせず、呼び出しによってクラッシュしないことだけ確認する。
        let _ = capability.is_supported(CodecDirection::Encoder, VideoCodecType::H265);
        let _ = capability.is_supported(CodecDirection::Decoder, VideoCodecType::H265);
    }

    #[test]
    fn internal_apple_capability_creates_supported_encoder_and_decoder() {
        let capability = InternalAppleVideoCodecCapability::new()
            .expect("InternalAppleVideoCodecCapability must be available");

        let encoder_formats = capability.get_supported_formats(CodecDirection::Encoder);
        let env = shiguredo_webrtc::Environment::new();
        for format in &encoder_formats {
            assert!(
                capability
                    .create_video_encoder(env.as_ref(), format.as_ref())
                    .is_some(),
                "encoder must be created for a supported format",
            );
        }

        let decoder_formats = capability.get_supported_formats(CodecDirection::Decoder);
        for format in &decoder_formats {
            assert!(
                capability
                    .create_video_decoder(env.as_ref(), format.as_ref())
                    .is_some(),
                "decoder must be created for a supported format",
            );
        }
    }

    #[test]
    fn internal_apple_capability_create_video_encoder_uses_simulcast_adapter() {
        let capability = InternalAppleVideoCodecCapability::new()
            .expect("InternalAppleVideoCodecCapability must be available");
        let Some(format) = capability
            .get_supported_formats(CodecDirection::Encoder)
            .into_iter()
            .next()
        else {
            return;
        };
        let env = shiguredo_webrtc::Environment::new();
        let encoder = capability
            .create_video_encoder(env.as_ref(), format.as_ref())
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
