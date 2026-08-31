//! テストで共有するテスト用ヘルパー型。
//!
//! 本モジュールはテストビルド (`#[cfg(test)]`) でのみコンパイルされる。
use shiguredo_webrtc::{
    AudioCodecInfo, AudioCodecSpec, AudioCodecType, AudioDecoder, AudioDecoderHandler,
    AudioEncoder, AudioEncoderEncodedInfo, AudioEncoderHandler, AudioSpeechType, BufferRef,
    EnvironmentRef, RawBufferWriter, SdpAudioFormat, SdpAudioFormatRef, SdpVideoFormat,
    SdpVideoFormatRef, VideoCodecType, VideoDecoder, VideoDecoderHandler, VideoEncoder,
    VideoEncoderHandler,
};

use crate::audio_codec_capability::{AudioCodecCapability, AudioCodecImplementation};
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
pub(crate) struct TestVideoCodecCapability {
    implementation: VideoCodecImplementation,
    encoder_formats: Vec<VideoCodecType>,
    decoder_formats: Vec<VideoCodecType>,
    /// false のときは `is_supported` が true でも `resolve_sdp_format` は None を返す。
    /// コーデック固有 parameter を必須とし、コーデック名だけの入力を拒否する capability を表す。
    resolves_sdp_format: bool,
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
            resolves_sdp_format: true,
        }
    }

    /// `resolve_sdp_format` が常に None を返す capability に変換する。
    /// コーデック固有 parameter を必須としてコーデック名だけの入力を拒否する
    /// capability の挙動をシミュレートする。
    pub(crate) fn without_sdp_format_resolution(mut self) -> Self {
        self.resolves_sdp_format = false;
        self
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
        if !self.resolves_sdp_format {
            return None;
        }
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

/// `AudioEncoderHandler` を実装したテスト専用の型。
///
/// `encode` は決まったバイト列 (`0x01, 0x02, 0x03`) をバッファへ追記し、
/// エンコード結果の整合 (追記バイト数 = encoded_bytes) を満たす。
pub(crate) struct TestAudioEncoder {
    payload_type: i32,
}

impl TestAudioEncoder {
    /// 指定したペイロードタイプを報告するエンコーダーを生成する。
    pub(crate) fn with_payload_type(payload_type: i32) -> Self {
        Self { payload_type }
    }
}

impl AudioEncoderHandler for TestAudioEncoder {
    fn sample_rate_hz(&mut self) -> i32 {
        48000
    }
    fn num_channels(&mut self) -> usize {
        2
    }
    fn num_10ms_frames_in_next_packet(&mut self) -> usize {
        1
    }
    fn max_10ms_frames_in_a_packet(&mut self) -> usize {
        1
    }
    fn get_target_bitrate(&mut self) -> i32 {
        32000
    }
    fn encode(
        &mut self,
        _rtp_timestamp: u32,
        _audio: &[i16],
        encoded: &mut BufferRef<'_>,
    ) -> AudioEncoderEncodedInfo {
        encoded.append_data(&[0x01, 0x02, 0x03]);
        let mut info = AudioEncoderEncodedInfo::new();
        info.set_encoded_bytes(encoded.size());
        info.set_payload_type(self.payload_type);
        info
    }
    fn reset(&mut self) {}
    fn get_frame_length_range(&mut self) -> Option<(i64, i64)> {
        None
    }
}

/// `AudioDecoderHandler` を実装したテスト専用の型。
///
/// `decode` は決まったサンプル列 (`0x1111` × 160) を書き込み、160 サンプルと Speech を返す。
pub(crate) struct TestAudioDecoder;
impl AudioDecoderHandler for TestAudioDecoder {
    fn sample_rate_hz(&mut self) -> i32 {
        48000
    }
    fn channels(&mut self) -> usize {
        2
    }
    fn decode(
        &mut self,
        _encoded: &[u8],
        _sample_rate_hz: i32,
        decoded: &mut RawBufferWriter<'_, i16>,
    ) -> (i32, AudioSpeechType) {
        decoded.write(&[0x1111i16; 160]);
        (160, AudioSpeechType::Speech)
    }
    fn reset(&mut self) {}
}

/// `TestAudioCodecCapability` が広告するテスト用のコーデック情報。
///
/// テストではビットレート等の実値に依存しないため、固定値を返す。
fn test_audio_codec_info() -> AudioCodecInfo {
    AudioCodecInfo::new(48000, 2, 32000, 6000, 510000)
}

/// `AudioCodecCapability` を本物のコードで実装したテスト専用の型。
pub(crate) struct TestAudioCodecCapability {
    implementation: AudioCodecImplementation,
    encoder_formats: Vec<AudioCodecType>,
    decoder_formats: Vec<AudioCodecType>,
    /// false のときは `is_supported` が true でも `resolve_sdp_codec_spec` は None を返す。
    resolves_sdp_format: bool,
}

impl TestAudioCodecCapability {
    /// 方向ごとのコーデック種別リストを指定して生成する。
    pub(crate) fn new(
        implementation: AudioCodecImplementation,
        encoder_formats: Vec<AudioCodecType>,
        decoder_formats: Vec<AudioCodecType>,
    ) -> Self {
        Self {
            implementation,
            encoder_formats,
            decoder_formats,
            resolves_sdp_format: true,
        }
    }

    /// `resolve_sdp_codec_spec` が常に None を返す capability に変換する。
    pub(crate) fn without_sdp_format_resolution(mut self) -> Self {
        self.resolves_sdp_format = false;
        self
    }

    /// 指定した方向のコーデック種別リストを返す。
    fn formats(&self, direction: CodecDirection) -> &[AudioCodecType] {
        match direction {
            CodecDirection::Encoder => &self.encoder_formats,
            CodecDirection::Decoder => &self.decoder_formats,
        }
    }
}

impl AudioCodecCapability for TestAudioCodecCapability {
    fn get_implementation(&self) -> AudioCodecImplementation {
        self.implementation.clone()
    }

    fn get_supported_codec_specs(&self, direction: CodecDirection) -> Vec<AudioCodecSpec> {
        self.formats(direction)
            .iter()
            .filter_map(|codec_type| {
                let name = codec_type.as_str()?;
                Some(AudioCodecSpec::new(
                    SdpAudioFormat::new(name, 48000, 2),
                    test_audio_codec_info(),
                ))
            })
            .collect()
    }

    fn is_supported(&self, direction: CodecDirection, codec_type: AudioCodecType) -> bool {
        self.formats(direction).contains(&codec_type)
    }

    fn resolve_sdp_codec_spec(
        &self,
        direction: CodecDirection,
        format: SdpAudioFormatRef<'_>,
    ) -> Option<AudioCodecSpec> {
        if !self.resolves_sdp_format {
            return None;
        }
        let codec_type = format
            .name()
            .ok()
            .and_then(|name| AudioCodecType::try_from(name.as_str()).ok())?;
        if !self.is_supported(direction, codec_type) {
            return None;
        }
        let codec_name = codec_type.as_str()?;
        Some(AudioCodecSpec::new(
            SdpAudioFormat::new(codec_name, 48000, 2),
            test_audio_codec_info(),
        ))
    }

    fn create_audio_encoder(
        &self,
        _env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
        payload_type: i32,
    ) -> Option<AudioEncoder> {
        let codec_type = format
            .name()
            .ok()
            .and_then(|name| AudioCodecType::try_from(name.as_str()).ok())?;
        if self.is_supported(CodecDirection::Encoder, codec_type) {
            Some(AudioEncoder::new_with_handler(Box::new(
                TestAudioEncoder::with_payload_type(payload_type),
            )))
        } else {
            None
        }
    }

    fn create_audio_decoder(
        &self,
        _env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
    ) -> Option<AudioDecoder> {
        let codec_type = format
            .name()
            .ok()
            .and_then(|name| AudioCodecType::try_from(name.as_str()).ok())?;
        if self.is_supported(CodecDirection::Decoder, codec_type) {
            Some(AudioDecoder::new_with_handler(Box::new(TestAudioDecoder)))
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
        assert!(
            capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp9),
            "公開フォーマットに含まれる VP9 は is_supported で true になるべきです"
        );
        let vp9 = SdpVideoFormat::new("VP9");
        assert!(
            capability
                .resolve_sdp_format(CodecDirection::Encoder, vp9.as_ref())
                .is_some(),
            "公開フォーマットに含まれる VP9 は resolve_sdp_format で解決できるべきです"
        );
        assert!(
            capability
                .create_video_encoder(shiguredo_webrtc::Environment::new().as_ref(), vp9.as_ref())
                .is_some(),
            "公開フォーマットに含まれる VP9 はエンコーダーを生成できるべきです"
        );
    }
}
