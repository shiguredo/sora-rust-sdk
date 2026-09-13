//! WebRTC 組み込みの音声コーデック実装。
use shiguredo_webrtc::{
    AudioCodecInfo, AudioCodecSpec, AudioDecoder, AudioDecoderFactory, AudioEncoder,
    AudioEncoderFactory, AudioEncoderFactoryOptions, EnvironmentRef, SdpAudioFormatRef,
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

    fn query(
        &self,
        direction: CodecDirection,
        format: SdpAudioFormatRef<'_>,
    ) -> Option<AudioCodecInfo> {
        match direction {
            // エンコーダーは下位ファクトリの問い合わせをそのまま利用する。
            // ネゴシエーションで決まったパラメータを反映した情報を返す。
            CodecDirection::Encoder => self.encoder_factory.query_audio_encoder(format),
            // デコーダー側は問い合わせ API が無いため、広告 spec (名前・クロックレート・
            // チャンネル数) との一致で判定する。パラメータの厳密な照合は create が受け持つ。
            CodecDirection::Decoder => {
                let request = format.to_owned();
                self.decoder_factory
                    .get_supported_decoders()
                    .into_iter()
                    .find(|spec| spec.format().matches(request.as_ref()))
                    .map(|spec| spec.info())
            }
        }
    }

    fn create_audio_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
        options: &AudioEncoderFactoryOptions,
    ) -> Option<AudioEncoder> {
        // Options を再構築せずそのまま渡すことで、payload_type だけでなく
        // codec_pair_id (Redundant Encoding のペアリング) 等も保持する。
        self.encoder_factory.create(env, format, options)
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

    /// 未対応コーデックの問い合わせは None を返すことを検証する。
    #[test]
    fn query_returns_none_for_unsupported_codec() {
        let capability = InternalAudioCodecCapability::new();
        for direction in [CodecDirection::Encoder, CodecDirection::Decoder] {
            let format = SdpAudioFormat::new("not-a-codec", 48000, 1);
            assert!(
                capability.query(direction, format.as_ref()).is_none(),
                "未対応コーデックは問い合わせできないはずです"
            );
        }
    }

    /// 名前が一致しても互換性のない設定 (クロックレート不一致) は受け付けないことを検証する。
    ///
    /// 名前だけの一致では opus@16000Hz を opus@48000 のコーデックとして誤って扱ってしまうため、
    /// SdpAudioFormat::matches による互換性判定を行う。
    #[test]
    fn query_rejects_incompatible_clockrate() {
        let capability = InternalAudioCodecCapability::new();
        for direction in [CodecDirection::Encoder, CodecDirection::Decoder] {
            // builtin の Opus は 48kHz/2ch を広告するため、16kHz の要求は不一致になる。
            let incompatible = SdpAudioFormat::new("opus", 16000, 1);
            assert!(
                capability.query(direction, incompatible.as_ref()).is_none(),
                "互換性のないクロックレートは問い合わせできないはずです"
            );
            // 一致する要求は問い合わせできる。
            let compatible = SdpAudioFormat::new("opus", 48000, 2);
            assert!(
                capability.query(direction, compatible.as_ref()).is_some(),
                "相容れる Opus は問い合わせできるはずです"
            );
        }
    }

    /// デコーダーでチャネル数不一致のフォーマットを受け付けないことを検証する。
    #[test]
    fn query_decoder_rejects_incompatible_channel_count() {
        let capability = InternalAudioCodecCapability::new();
        // builtin の Opus デコーダーは 2ch を広告するため、1ch の要求は不一致になる。
        let incompatible = SdpAudioFormat::new("opus", 48000, 1);
        assert!(
            capability
                .query(CodecDirection::Decoder, incompatible.as_ref())
                .is_none(),
            "チャネル数が不一致の Opus は受け付けないはずです"
        );
    }
}
