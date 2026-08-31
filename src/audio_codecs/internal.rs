//! WebRTC 組み込みの音声コーデック実装。
use shiguredo_webrtc::{
    AudioCodecSpec, AudioDecoder, AudioDecoderFactory, AudioEncoder, AudioEncoderFactory,
    AudioEncoderFactoryOptions, EnvironmentRef, SdpAudioFormatRef,
};

use crate::audio_codec_capability::{AudioCodecCapability, AudioCodecImplementation};
use crate::codec_direction::CodecDirection;

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

    fn get_supported_codec_specs(&self, direction: CodecDirection) -> Vec<AudioCodecSpec> {
        match direction {
            CodecDirection::Encoder => self.encoder_factory.get_supported_encoders(),
            CodecDirection::Decoder => self.decoder_factory.get_supported_decoders(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_direction::CodecDirection;
    use shiguredo_webrtc::SdpAudioFormat;

    /// 非 Opus コーデックは builtin ファクトリが広告する実情報をそのまま広告することを検証する。
    #[test]
    fn builtin_advertises_real_non_opus_spec_info() {
        let capability = InternalAudioCodecCapability::new();
        for direction in [CodecDirection::Encoder, CodecDirection::Decoder] {
            let specs = capability.get_supported_codec_specs(direction);
            for name in ["G722", "PCMU", "PCMA"] {
                let spec = specs
                    .iter()
                    .find(|spec| spec.format().name().ok().as_deref() == Some(name))
                    .expect("builtin が広告するコーデックが見つかりません");
                let info = spec.info();
                assert_eq!(
                    info.default_bitrate_bps(),
                    64000,
                    "{name} の既定ビットレートが builtin 実値と一致しません"
                );
                assert_eq!(
                    info.min_bitrate_bps(),
                    64000,
                    "{name} の最小ビットレートが builtin 実値と一致しません"
                );
                assert_eq!(
                    info.max_bitrate_bps(),
                    64000,
                    "{name} の最大ビットレートが builtin 実値と一致しません"
                );
            }
        }
    }

    /// Opus は builtin の実 spec (encoder: 48kHz/1ch/32kbps, decoder: 48kHz/1ch/64kbps) を広告する
    /// ことを検証する。SDK の公開値として builtin 実値を素通しする。
    #[test]
    fn builtin_advertises_real_opus_spec_info() {
        let capability = InternalAudioCodecCapability::new();
        let enc = capability
            .get_supported_codec_specs(CodecDirection::Encoder)
            .into_iter()
            .find(|spec| spec.format().name().ok().as_deref() == Some("opus"))
            .expect("builtin が Opus エンコーダーを広告すること");
        assert_eq!(enc.info().sample_rate_hz(), 48000);
        assert_eq!(enc.info().num_channels(), 1);
        assert_eq!(enc.info().default_bitrate_bps(), 32000);

        let dec = capability
            .get_supported_codec_specs(CodecDirection::Decoder)
            .into_iter()
            .find(|spec| spec.format().name().ok().as_deref() == Some("opus"))
            .expect("builtin が Opus デコーダーを広告すること");
        assert_eq!(dec.info().sample_rate_hz(), 48000);
        assert_eq!(dec.info().num_channels(), 1);
        assert_eq!(dec.info().default_bitrate_bps(), 64000);
    }

    /// 未支援コーデックの resolve は None を返すことを検証する。
    #[test]
    fn resolve_returns_none_for_unsupported_codec() {
        let capability = InternalAudioCodecCapability::new();
        let format = SdpAudioFormat::new("not-a-codec", 48000, 1);
        assert!(
            capability
                .resolve_sdp_codec_spec(CodecDirection::Encoder, format.as_ref())
                .is_none()
        );
    }

    /// 名前が一致しても互換性のない設定 (クロックレート不一致) は resolve しないことを検証する。
    ///
    /// 名前だけの一致では opus@16000Hz を opus@48000 の仕様に誤って解決してしまうため、
    /// matches による互換性判定が必要。
    #[test]
    fn resolve_rejects_incompatible_clockrate() {
        let capability = InternalAudioCodecCapability::new();
        // builtin の Opus は 48kHz を広告するため、16kHz の要求は不一致になる。
        let incompatible = SdpAudioFormat::new("opus", 16000, 1);
        assert!(
            capability
                .resolve_sdp_codec_spec(CodecDirection::Encoder, incompatible.as_ref())
                .is_none(),
            "互換性のないクロックレートは解決されるべきではありません"
        );
        // 一致する要求は解決できる。
        let compatible = SdpAudioFormat::new("opus", 48000, 2);
        assert!(
            capability
                .resolve_sdp_codec_spec(CodecDirection::Encoder, compatible.as_ref())
                .is_some(),
            "相容れる Opus は解決されるべきです"
        );
    }
}
