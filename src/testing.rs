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
pub(crate) struct NoopVideoEncoder;
impl VideoEncoderHandler for NoopVideoEncoder {}

/// `VideoDecoderHandler` を最小限に実装したテスト専用の型。
pub(crate) struct NoopVideoDecoder;
impl VideoDecoderHandler for NoopVideoDecoder {}

/// `VideoCodecCapability` を本物のコードで実装したテスト専用の型。
///
/// サポートするコーデック種別のリストと、`get_supported_formats()` に
/// 反映するフォーマットのリストを保持する。フォーマットのリストが未設定の場合は
/// サポートするコーデック種別からフォーマットを生成する。
pub(crate) struct TestVideoCodecCapability {
    implementation: VideoCodecImplementation,
    encoder_supported: Vec<VideoCodecType>,
    decoder_supported: Vec<VideoCodecType>,
    encoder_formats: Option<Vec<VideoCodecType>>,
    decoder_formats: Option<Vec<VideoCodecType>>,
}

impl TestVideoCodecCapability {
    /// サポートするコーデック種別を指定して生成する。
    pub(crate) fn new(
        implementation: VideoCodecImplementation,
        encoder_supported: Vec<VideoCodecType>,
        decoder_supported: Vec<VideoCodecType>,
    ) -> Self {
        Self {
            implementation,
            encoder_supported,
            decoder_supported,
            encoder_formats: None,
            decoder_formats: None,
        }
    }

    /// 指定した方向の `get_supported_formats()` の結果を明示的に設定する。
    pub(crate) fn with_supported_formats(
        mut self,
        direction: CodecDirection,
        formats: Vec<VideoCodecType>,
    ) -> Self {
        match direction {
            CodecDirection::Encoder => self.encoder_formats = Some(formats),
            CodecDirection::Decoder => self.decoder_formats = Some(formats),
        }
        self
    }
}

impl VideoCodecCapability for TestVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        self.implementation.clone()
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        let codec_types = match direction {
            CodecDirection::Encoder => self
                .encoder_formats
                .as_ref()
                .unwrap_or(&self.encoder_supported),
            CodecDirection::Decoder => self
                .decoder_formats
                .as_ref()
                .unwrap_or(&self.decoder_supported),
        };
        codec_types
            .iter()
            .filter_map(|codec_type| codec_type.as_str().map(SdpVideoFormat::new))
            .collect()
    }

    fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
        match direction {
            CodecDirection::Encoder => self.encoder_supported.contains(&codec_type),
            CodecDirection::Decoder => self.decoder_supported.contains(&codec_type),
        }
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
