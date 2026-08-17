//! テストで共有するテスト用ヘルパー型。
//!
//! 本モジュールはテストビルド (`#[cfg(test)]`) でのみコンパイルされる。
use shiguredo_webrtc::{
    EnvironmentRef, SdpVideoFormat, SdpVideoFormatRef, VideoCodecType, VideoDecoder,
    VideoDecoderHandler, VideoEncoder, VideoEncoderHandler,
};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

/// `VideoEncoderHandler` を最小限に実装したテスト専用の型。
struct NoopVideoEncoder;
impl VideoEncoderHandler for NoopVideoEncoder {}

/// `VideoDecoderHandler` を最小限に実装したテスト専用の型。
struct NoopVideoDecoder;
impl VideoDecoderHandler for NoopVideoDecoder {}

/// `VideoCodecCapability` を本物のコードで実装したテスト専用の型。
pub(crate) struct TestVideoCodecCapability {
    implementation: VideoCodecImplementation,
    encoder_formats: Vec<VideoCodecType>,
    decoder_formats: Vec<VideoCodecType>,
}

impl TestVideoCodecCapability {
    /// 方向ごとのコーデック種別リストを指定して生成する。
    pub(crate) fn new(
        implementation: VideoCodecImplementation,
        encoder_formats: Vec<VideoCodecType>,
        decoder_formats: Vec<VideoCodecType>,
    ) -> Self {
        Self {
            implementation,
            encoder_formats,
            decoder_formats,
        }
    }

    /// 指定した方向のコーデック種別リストを返す。
    fn formats(&self, direction: CodecDirection) -> &[VideoCodecType] {
        match direction {
            CodecDirection::Encoder => &self.encoder_formats,
            CodecDirection::Decoder => &self.decoder_formats,
        }
    }
}

impl VideoCodecCapability for TestVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        self.implementation.clone()
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        self.formats(direction)
            .iter()
            .filter_map(|codec_type| codec_type.as_str().map(SdpVideoFormat::new))
            .collect()
    }

    fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
        self.formats(direction).contains(&codec_type)
    }

    fn resolve_sdp_format(
        &self,
        direction: CodecDirection,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<SdpVideoFormat> {
        let codec_type = format
            .name()
            .ok()
            .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
        if !self.is_supported(direction, codec_type) {
            return None;
        }
        let codec_name = codec_type.as_str()?;
        let mut resolved = SdpVideoFormat::new(codec_name);
        if codec_type == VideoCodecType::H264 {
            resolved.parameters_mut().set("packetization-mode", "1");
        }
        Some(resolved)
    }

    fn create_video_encoder(
        &self,
        _env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        let codec_type = format
            .name()
            .ok()
            .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
        if self.is_supported(CodecDirection::Encoder, codec_type) {
            Some(VideoEncoder::new_with_handler(Box::new(NoopVideoEncoder)))
        } else {
            None
        }
    }

    fn create_video_decoder(
        &self,
        _env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        let codec_type = format
            .name()
            .ok()
            .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
        if self.is_supported(CodecDirection::Decoder, codec_type) {
            Some(VideoDecoder::new_with_handler(Box::new(NoopVideoDecoder)))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_methods_share_the_same_format_list() {
        let capability = TestVideoCodecCapability::new(
            VideoCodecImplementation::new("test", "Test Codec"),
            vec![VideoCodecType::Vp9, VideoCodecType::H264],
            Vec::new(),
        );
        // 公開フォーマットのリストが is_supported / resolve_sdp_format /
        // create_video_encoder にも効くことを確認する。
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp9));
        let vp9 = SdpVideoFormat::new("VP9");
        assert!(
            capability
                .resolve_sdp_format(CodecDirection::Encoder, vp9.as_ref())
                .is_some()
        );
        assert!(
            capability
                .create_video_encoder(shiguredo_webrtc::Environment::new().as_ref(), vp9.as_ref())
                .is_some()
        );
    }
}
