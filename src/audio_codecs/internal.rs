//! WebRTC 組み込みの音声コーデック実装。
use shiguredo_webrtc::{
    AudioDecoder, AudioDecoderFactory, AudioEncoder, AudioEncoderFactory,
    AudioEncoderFactoryOptions, EnvironmentRef, SdpAudioFormat, SdpAudioFormatRef,
};

use crate::audio_codec_capability::{AudioCodecCapability, AudioCodecImplementation};
use crate::video_codec_capability::CodecDirection;

/// WebRTC 組み込みのエンコーダー/デコーダーを使用する [AudioCodecCapability]。
pub struct InternalAudioCodecCapability {
    implementation: AudioCodecImplementation,
    encoder_factory: AudioEncoderFactory,
    decoder_factory: AudioDecoderFactory,
}

impl InternalAudioCodecCapability {
    /// 新しい `InternalAudioCodecCapability` を生成する。
    pub fn new() -> Self {
        Self {
            implementation: AudioCodecImplementation::new(
                "internal",
                "WebRTC built-in AudioCodecFactory",
            ),
            encoder_factory: AudioEncoderFactory::builtin(),
            decoder_factory: AudioDecoderFactory::builtin(),
        }
    }
}

impl Default for InternalAudioCodecCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioCodecCapability for InternalAudioCodecCapability {
    fn get_implementation(&self) -> AudioCodecImplementation {
        self.implementation.clone()
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpAudioFormat> {
        match direction {
            CodecDirection::Encoder => self
                .encoder_factory
                .get_supported_encoders()
                .into_iter()
                .map(|spec| spec.format())
                .collect(),
            CodecDirection::Decoder => self
                .decoder_factory
                .get_supported_decoders()
                .into_iter()
                .map(|spec| spec.format())
                .collect(),
        }
    }

    fn create_audio_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
        payload_type: i32,
    ) -> Option<AudioEncoder> {
        let mut options = AudioEncoderFactoryOptions::new();
        options.set_payload_type(payload_type);
        self.encoder_factory.create(env, format, &options)
    }

    fn create_audio_decoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
    ) -> Option<AudioDecoder> {
        self.decoder_factory.create(env, format)
    }
}
