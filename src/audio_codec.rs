//! 音声コーデックのエンコーダー/デコーダーファクトリを提供する。
use std::sync::{Arc, Mutex};

use shiguredo_webrtc::{
    AudioCodecInfo, AudioCodecSpec, AudioCodecType, AudioDecoder, AudioDecoderFactoryHandler,
    AudioEncoder, AudioEncoderFactoryHandler, AudioEncoderFactoryOptions, EnvironmentRef,
    SdpAudioFormatRef,
};

use crate::audio_codec_capability::{AudioCodecCapability, find_audio_capability};
use crate::audio_codec_preference::AudioCodecPreference;
use crate::video_codec_capability::CodecDirection;

type AudioCodecCapabilities = Vec<Box<dyn AudioCodecCapability>>;
type SharedAudioCodecCapabilities = Arc<Mutex<AudioCodecCapabilities>>;

/// [AudioCodecPreference] に基づき、利用可能な音声エンコーダーを提供するファクトリ。
pub struct SoraAudioEncoderFactory {
    preference: AudioCodecPreference,
    capabilities: SharedAudioCodecCapabilities,
}

/// [AudioCodecPreference] に基づき、利用可能な音声デコーダーを提供するファクトリ。
pub struct SoraAudioDecoderFactory {
    preference: AudioCodecPreference,
    capabilities: SharedAudioCodecCapabilities,
}

impl SoraAudioEncoderFactory {
    pub(crate) fn new(
        preference: AudioCodecPreference,
        capabilities: SharedAudioCodecCapabilities,
    ) -> Self {
        Self {
            preference,
            capabilities,
        }
    }
}

impl SoraAudioDecoderFactory {
    pub(crate) fn new(
        preference: AudioCodecPreference,
        capabilities: SharedAudioCodecCapabilities,
    ) -> Self {
        Self {
            preference,
            capabilities,
        }
    }
}

impl AudioEncoderFactoryHandler for SoraAudioEncoderFactory {
    fn get_supported_encoders(&mut self) -> Vec<AudioCodecSpec> {
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
        collect_supported_encoders(&self.preference, &capabilities, CodecDirection::Encoder)
    }

    fn query_audio_encoder(&mut self, format: SdpAudioFormatRef<'_>) -> Option<AudioCodecInfo> {
        let format_name = format.name().ok()?;
        let codec_type = AudioCodecType::try_from(format_name.as_str()).ok()?;
        let preference = self.preference.find(CodecDirection::Encoder, codec_type)?;
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
        let capability = find_audio_capability(&capabilities, preference.implementation())?;
        let info = default_audio_codec_info(&format);
        if capability
            .resolve_sdp_format(CodecDirection::Encoder, format)
            .is_some()
        {
            Some(info)
        } else {
            None
        }
    }

    // 要求された `format` を `capability.resolve_sdp_format` に通し、その返り値
    // （解決済み format）を `capability.create_audio_encoder` に渡す。
    fn create(
        &mut self,
        env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
        options: &AudioEncoderFactoryOptions,
    ) -> Option<AudioEncoder> {
        let format_name = format.name().ok()?;
        let codec_type = AudioCodecType::try_from(format_name.as_str()).ok()?;
        let preference = self.preference.find(CodecDirection::Encoder, codec_type)?;
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
        let capability = find_audio_capability(&capabilities, preference.implementation())?;
        let resolved = capability.resolve_sdp_format(CodecDirection::Encoder, format)?;
        capability.create_audio_encoder(env, resolved.as_ref(), options.payload_type())
    }
}

impl AudioDecoderFactoryHandler for SoraAudioDecoderFactory {
    fn get_supported_decoders(&mut self) -> Vec<AudioCodecSpec> {
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
        collect_supported_encoders(&self.preference, &capabilities, CodecDirection::Decoder)
    }

    fn is_supported_decoder(&mut self, format: SdpAudioFormatRef<'_>) -> bool {
        let Ok(format_name) = format.name() else {
            return false;
        };
        let Ok(codec_type) = AudioCodecType::try_from(format_name.as_str()) else {
            return false;
        };
        let Some(preference) = self.preference.find(CodecDirection::Decoder, codec_type) else {
            return false;
        };
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
        let Some(capability) = find_audio_capability(&capabilities, preference.implementation())
        else {
            return false;
        };
        capability
            .resolve_sdp_format(CodecDirection::Decoder, format)
            .is_some()
    }

    // Encoder 側と同じ規則。要求された `format` を `capability.resolve_sdp_format` に
    // 通し、返り値を `capability.create_audio_decoder` に渡す。
    fn create(
        &mut self,
        env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
    ) -> Option<AudioDecoder> {
        let format_name = format.name().ok()?;
        let codec_type = AudioCodecType::try_from(format_name.as_str()).ok()?;
        let preference = self.preference.find(CodecDirection::Decoder, codec_type)?;
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
        let capability = find_audio_capability(&capabilities, preference.implementation())?;
        let resolved = capability.resolve_sdp_format(CodecDirection::Decoder, format)?;
        capability.create_audio_decoder(env, resolved.as_ref())
    }
}

/// 指定方向の [AudioCodecPreference] から公開する [AudioCodecSpec] 一覧を構築する。
fn collect_supported_encoders(
    preference: &AudioCodecPreference,
    capabilities: &[Box<dyn AudioCodecCapability>],
    direction: CodecDirection,
) -> Vec<AudioCodecSpec> {
    let mut specs = Vec::new();
    for codec in preference.codecs() {
        if codec.direction() != direction {
            continue;
        }
        let Some(capability) = find_audio_capability(capabilities, codec.implementation()) else {
            continue;
        };
        for format in capability.get_supported_formats(codec.direction()) {
            let format_codec_type = format
                .name()
                .ok()
                .and_then(|name| AudioCodecType::try_from(name.as_str()).ok());
            if format_codec_type != Some(codec.codec_type()) {
                continue;
            }
            if specs
                .iter()
                .any(|existing: &AudioCodecSpec| existing.format().is_equal(format.as_ref()))
            {
                continue;
            }
            let info = default_audio_codec_info(&format.as_ref());
            specs.push(AudioCodecSpec::new(format, info));
        }
    }
    specs
}

/// フォーマットから [AudioCodecInfo] を推定する。
///
/// 現時点で SDK が対象とするのは Opus のみであり、Opus の実値を返す。
/// それ以外のコーデックはクロックレートを既定ビットレートとして扱う。
fn default_audio_codec_info(format: &SdpAudioFormatRef<'_>) -> AudioCodecInfo {
    let clockrate_hz = format.clockrate_hz();
    let num_channels = format.num_channels();
    match format.name().ok().as_deref() {
        Some("opus") => AudioCodecInfo::new(48000, 2, 32000, 6000, 510000),
        _ => {
            let default_bitrate = clockrate_hz * 16;
            AudioCodecInfo::new(
                clockrate_hz,
                num_channels,
                default_bitrate,
                0,
                default_bitrate,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_codec_capability::{AudioCodecCapability, AudioCodecImplementation};
    use crate::audio_codec_preference::AudioPreferenceCodec;
    use crate::testing::TestAudioCodecCapability;
    use crate::video_codec_capability::CodecDirection;
    use shiguredo_webrtc::{AudioCodecType, AudioSpeechType, Buffer, Environment, SdpAudioFormat};

    #[test]
    fn encoder_factory_uses_preference_order() {
        let preference = AudioCodecPreference::new(vec![AudioPreferenceCodec::new(
            CodecDirection::Encoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("impl-a", "Implementation A"),
        )]);
        let capabilities: Vec<Box<dyn AudioCodecCapability>> =
            vec![Box::new(TestAudioCodecCapability::new(
                AudioCodecImplementation::new("impl-a", "Implementation A"),
                vec![AudioCodecType::Opus],
                Vec::new(),
            ))];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraAudioEncoderFactory::new(preference, shared);
        let specs = AudioEncoderFactoryHandler::get_supported_encoders(&mut factory);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].format().name().expect("name 取得失敗"), "opus");
    }

    #[test]
    fn encoder_factory_ignores_resolve_when_capability_formats_missing_codec() {
        let preference = AudioCodecPreference::new(vec![AudioPreferenceCodec::new(
            CodecDirection::Encoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("impl-a", "Implementation A"),
        )]);
        let capabilities: Vec<Box<dyn AudioCodecCapability>> =
            vec![Box::new(TestAudioCodecCapability::new(
                AudioCodecImplementation::new("impl-a", "Implementation A"),
                vec![AudioCodecType::Isac],
                Vec::new(),
            ))];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraAudioEncoderFactory::new(preference, shared);
        let specs = AudioEncoderFactoryHandler::get_supported_encoders(&mut factory);
        assert!(specs.is_empty());
    }

    #[test]
    fn decoder_factory_create_requires_supported_codec_type() {
        let preference = AudioCodecPreference::new(vec![AudioPreferenceCodec::new(
            CodecDirection::Decoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("impl-a", "Implementation A"),
        )]);
        let capabilities: Vec<Box<dyn AudioCodecCapability>> =
            vec![Box::new(TestAudioCodecCapability::new(
                AudioCodecImplementation::new("impl-a", "Implementation A"),
                Vec::new(),
                vec![AudioCodecType::Opus],
            ))];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraAudioDecoderFactory::new(preference, shared);
        let env = shiguredo_webrtc::Environment::new();

        // サポート済みの opus は生成でき、未サポートの ISAC は生成できないことを検証する。
        let opus = SdpAudioFormat::new("opus", 48000, 2);
        assert!(
            AudioDecoderFactoryHandler::create(&mut factory, env.as_ref(), opus.as_ref()).is_some()
        );

        let isac = SdpAudioFormat::new("ISAC", 16000, 1);
        assert!(
            AudioDecoderFactoryHandler::create(&mut factory, env.as_ref(), isac.as_ref()).is_none()
        );
    }

    #[test]
    fn encoder_decoder_round_trip_through_capability() {
        // AudioCodecPreference と AudioCodecCapability を使って、エンコード→デコードの
        // 一連の流れが機能することを統合的に検証する。ハンドラは testing.rs の
        // TestAudioEncoder / TestAudioDecoder を使用する。
        let preference = AudioCodecPreference::new(vec![
            AudioPreferenceCodec::new(
                CodecDirection::Encoder,
                AudioCodecType::Opus,
                AudioCodecImplementation::new("roundtrip", "Roundtrip Codec"),
            ),
            AudioPreferenceCodec::new(
                CodecDirection::Decoder,
                AudioCodecType::Opus,
                AudioCodecImplementation::new("roundtrip", "Roundtrip Codec"),
            ),
        ]);
        let capabilities: Vec<Box<dyn AudioCodecCapability>> =
            vec![Box::new(TestAudioCodecCapability::new(
                AudioCodecImplementation::new("roundtrip", "Roundtrip Codec"),
                vec![AudioCodecType::Opus],
                vec![AudioCodecType::Opus],
            ))];
        let shared = Arc::new(Mutex::new(capabilities));
        let env = Environment::new();
        let format = SdpAudioFormat::new("opus", 48000, 2);

        // エンコーダーを生成し、実際にエンコードする。
        let mut encoder_factory = SoraAudioEncoderFactory::new(preference.clone(), shared.clone());
        let mut options = AudioEncoderFactoryOptions::new();
        options.set_payload_type(111);
        let mut encoder = AudioEncoderFactoryHandler::create(
            &mut encoder_factory,
            env.as_ref(),
            format.as_ref(),
            &options,
        )
        .expect("カスタムエンコーダーの生成に失敗しました");
        let mut out = Buffer::new();
        let info = encoder.encode(0, &[0i16; 960], &mut out);
        assert_eq!(out.size(), 3, "エンコード結果が書き込まれていません");
        assert_eq!(info.encoded_bytes(), 3);
        assert_eq!(info.payload_type(), 111);

        // デコーダーを生成し、エンコード結果をデコードする。
        let mut decoder_factory = SoraAudioDecoderFactory::new(preference, shared);
        let mut decoder =
            AudioDecoderFactoryHandler::create(&mut decoder_factory, env.as_ref(), format.as_ref())
                .expect("カスタムデコーダーの生成に失敗しました");
        let mut decoded = [0x7FFFi16; 320];
        let (samples, speech) = decoder.decode(out.data(), 48000, &mut decoded);
        assert_eq!(samples, 160);
        assert_eq!(speech, AudioSpeechType::Speech);
        assert!(
            decoded[..160].iter().all(|&v| v == 0x1111),
            "デコード結果が書き込まれていません"
        );
        assert!(
            decoded[160..].iter().all(|&v| v == 0x7FFF),
            "未書き込み領域の番兵が破壊されました"
        );
    }
}
