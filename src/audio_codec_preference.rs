//! 音声コーデックの優先順位設定。
use std::collections::HashSet;

use shiguredo_webrtc::AudioCodecType;

use nojson::{DisplayJson, Json, JsonFormatter, JsonParseError, RawJsonValue};

use crate::audio_codec_capability::{
    AudioCodecCapability, AudioCodecImplementation, find_audio_capability,
};
use crate::codec_direction::CodecDirection;
use crate::error::{Error, Result};

/// 特定の方向・コーデック・実装の優先設定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPreferenceCodec {
    direction: CodecDirection,
    codec_type: AudioCodecType,
    implementation: AudioCodecImplementation,
}

/// 音声コーデックの優先順位リスト。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioCodecPreference {
    codecs: Vec<AudioPreferenceCodec>,
}

/// 定義順で列挙する音声コーデック種別。
const AUDIO_CODEC_TYPES: [AudioCodecType; 5] = [
    AudioCodecType::Opus,
    AudioCodecType::Isac,
    AudioCodecType::G722,
    AudioCodecType::PcmA,
    AudioCodecType::PcmU,
];

impl AudioPreferenceCodec {
    /// [AudioPreferenceCodec] を生成する。
    pub fn new(
        direction: CodecDirection,
        codec_type: AudioCodecType,
        implementation: AudioCodecImplementation,
    ) -> Self {
        Self {
            direction,
            codec_type,
            implementation,
        }
    }

    /// [CodecDirection] を返す。
    pub fn direction(&self) -> CodecDirection {
        self.direction
    }

    /// [AudioCodecType] を返す。
    pub fn codec_type(&self) -> AudioCodecType {
        self.codec_type
    }

    /// [AudioCodecImplementation] を返す。
    pub fn implementation(&self) -> &AudioCodecImplementation {
        &self.implementation
    }

    /// [AudioCodecImplementation] を上書きする。
    pub fn set_implementation(&mut self, implementation: AudioCodecImplementation) {
        self.implementation = implementation;
    }
}

impl AudioCodecPreference {
    /// [AudioPreferenceCodec] のリストから [AudioCodecPreference] を生成する。
    pub fn new(codecs: Vec<AudioPreferenceCodec>) -> Self {
        Self { codecs }
    }

    /// [AudioCodecCapability] から自動生成した優先設定を返す。
    pub fn new_from_capability(capability: &dyn AudioCodecCapability) -> Self {
        let implementation = capability.get_implementation();
        let mut codecs = Vec::new();
        for codec_type in AUDIO_CODEC_TYPES {
            for direction in [CodecDirection::Encoder, CodecDirection::Decoder] {
                if capability.is_supported(direction, codec_type) {
                    codecs.push(AudioPreferenceCodec::new(
                        direction,
                        codec_type,
                        implementation.clone(),
                    ));
                }
            }
        }
        Self::new(codecs)
    }

    /// 全 [AudioPreferenceCodec] を返す。
    pub fn codecs(&self) -> &[AudioPreferenceCodec] {
        &self.codecs
    }

    /// 指定された方向とコーデック種別に合致する [AudioPreferenceCodec] を検索する。
    pub fn find(
        &self,
        direction: CodecDirection,
        codec_type: AudioCodecType,
    ) -> Option<&AudioPreferenceCodec> {
        self.codecs
            .iter()
            .find(|codec| codec.direction == direction && codec.codec_type == codec_type)
    }

    /// 指定された方向とコーデック種別に合致する [AudioPreferenceCodec] を可変参照で検索する。
    pub fn find_mut(
        &mut self,
        direction: CodecDirection,
        codec_type: AudioCodecType,
    ) -> Option<&mut AudioPreferenceCodec> {
        self.codecs
            .iter_mut()
            .find(|codec| codec.direction == direction && codec.codec_type == codec_type)
    }

    /// 指定された方向・コーデック・実装のエントリを取得し、なければ追加する。
    pub fn get_or_add(
        &mut self,
        direction: CodecDirection,
        codec_type: AudioCodecType,
        implementation: AudioCodecImplementation,
    ) -> &mut AudioPreferenceCodec {
        if let Some(index) = self
            .codecs
            .iter()
            .position(|codec| codec.direction == direction && codec.codec_type == codec_type)
        {
            return &mut self.codecs[index];
        }
        self.codecs.push(AudioPreferenceCodec::new(
            direction,
            codec_type,
            implementation,
        ));
        self.codecs
            .last_mut()
            .expect("codecs must contain one element after push")
    }

    /// 指定された [AudioCodecImplementation] が含まれているかどうかを返す。
    pub fn has_implementation(&self, implementation: AudioCodecImplementation) -> bool {
        self.codecs
            .iter()
            .any(|codec| codec.implementation == implementation)
    }

    /// 別の [AudioCodecPreference] をマージする。
    ///
    /// 既存エントリと方向・コーデックが一致する項目は上書きし、
    /// 存在しない項目は追加する。
    pub fn merge(&mut self, preference: &AudioCodecPreference) {
        for codec in &preference.codecs {
            if let Some(existing) = self.find_mut(codec.direction, codec.codec_type) {
                existing.implementation = codec.implementation.clone();
            } else {
                self.codecs.push(codec.clone());
            }
        }
    }
}

impl DisplayJson for AudioPreferenceCodec {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        let codec_type = audio_codec_type_to_json_str(self.codec_type)?;
        f.object(|f| {
            f.member("direction", self.direction.as_str())?;
            f.member("codec_type", codec_type)?;
            f.member("implementation", &self.implementation)
        })
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for AudioPreferenceCodec {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        let direction: CodecDirection = value.to_member("direction")?.required()?.try_into()?;
        let codec_type = parse_audio_codec_type(value.to_member("codec_type")?.required()?)?;
        let implementation: AudioCodecImplementation =
            value.to_member("implementation")?.required()?.try_into()?;
        Ok(Self {
            direction,
            codec_type,
            implementation,
        })
    }
}

impl DisplayJson for AudioCodecPreference {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| f.member("codecs", &self.codecs))
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for AudioCodecPreference {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            codecs: value.to_member("codecs")?.required()?.try_into()?,
        })
    }
}

/// [AudioCodecPreference] の妥当性を検証する。
///
/// - 同じ方向・コーデック種別の重複がないこと
/// - 各エントリの実装が `capabilities` に存在すること
/// - 各エントリの方向・コーデックが実装でサポートされていること
pub fn validate_audio_codec_preference(
    preference: &AudioCodecPreference,
    capabilities: &[Box<dyn AudioCodecCapability>],
) -> Result<()> {
    validate_capabilities(capabilities)?;

    for codec_type in AUDIO_CODEC_TYPES {
        for direction in [CodecDirection::Encoder, CodecDirection::Decoder] {
            let count = preference
                .codecs()
                .iter()
                .filter(|codec| codec.direction() == direction && codec.codec_type() == codec_type)
                .count();
            if count >= 2 {
                let codec_type_name =
                    audio_codec_type_to_json_str(codec_type).expect("known codec type");
                return Err(Error::InvalidAudioCodecPreference {
                    reason: format!("duplicate {codec_type_name} {}", direction.as_label()),
                });
            }
        }
    }

    for codec in preference.codecs() {
        validate_codec(codec, capabilities)?;
    }

    Ok(())
}

fn validate_capabilities(capabilities: &[Box<dyn AudioCodecCapability>]) -> Result<()> {
    let mut implementation_names = HashSet::new();
    for capability in capabilities {
        let implementation = capability.get_implementation();
        let implementation_name = implementation.name().to_string();
        if !implementation_names.insert(implementation_name) {
            return Err(Error::InvalidAudioCodecCapability {
                reason: format!(
                    "duplicate implementation in capabilities: implementation={}",
                    Json(&implementation)
                ),
            });
        }
    }
    Ok(())
}

fn validate_codec(
    codec: &AudioPreferenceCodec,
    capabilities: &[Box<dyn AudioCodecCapability>],
) -> Result<()> {
    let direction = codec.direction().as_label();
    let Some(capability) = find_audio_capability(capabilities, codec.implementation()) else {
        return Err(Error::InvalidAudioCodecPreference {
            reason: format!(
                "{direction} implementation not found: codec_preference={}",
                Json(codec)
            ),
        });
    };
    let encoder_supported = capability.is_supported(CodecDirection::Encoder, codec.codec_type());
    let decoder_supported = capability.is_supported(CodecDirection::Decoder, codec.codec_type());
    let (direction_supported, opposite_supported) = match codec.direction() {
        CodecDirection::Encoder => (encoder_supported, decoder_supported),
        CodecDirection::Decoder => (decoder_supported, encoder_supported),
    };

    if !direction_supported && !opposite_supported {
        return Err(Error::InvalidAudioCodecPreference {
            reason: format!("codec type not found: codec_preference={}", Json(codec)),
        });
    }

    if !direction_supported {
        return Err(Error::InvalidAudioCodecPreference {
            reason: format!(
                "{direction} not supported: codec_preference={}, codec_capability={}",
                Json(codec),
                codec_capability_summary(capability, codec.codec_type())
            ),
        });
    }

    Ok(())
}

fn codec_capability_summary(
    capability: &dyn AudioCodecCapability,
    codec_type: AudioCodecType,
) -> String {
    let codec_type_name = audio_codec_type_to_json_str(codec_type).unwrap_or("Unknown");
    format!(
        "{{\"implementation\":{},\"codec_type\":\"{}\",\"encoder_supported\":{},\"decoder_supported\":{}}}",
        Json(&capability.get_implementation()),
        codec_type_name,
        capability.is_supported(CodecDirection::Encoder, codec_type),
        capability.is_supported(CodecDirection::Decoder, codec_type),
    )
}

fn audio_codec_type_to_json_str(
    codec_type: AudioCodecType,
) -> std::result::Result<&'static str, std::fmt::Error> {
    codec_type.as_str().ok_or(std::fmt::Error)
}

fn parse_audio_codec_type(
    value: RawJsonValue<'_, '_>,
) -> std::result::Result<AudioCodecType, JsonParseError> {
    let codec_type_text: String = value.try_into()?;
    AudioCodecType::try_from(codec_type_text.as_str())
        .map_err(|_| value.invalid(format!("unsupported audio codec type: {codec_type_text}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::testing::TestAudioCodecCapability;

    fn default_preference_codec(
        direction: CodecDirection,
        codec_type: AudioCodecType,
        implementation: AudioCodecImplementation,
    ) -> AudioPreferenceCodec {
        AudioPreferenceCodec::new(direction, codec_type, implementation)
    }

    fn sample_capabilities() -> Vec<Box<dyn AudioCodecCapability>> {
        vec![Box::new(TestAudioCodecCapability::new(
            AudioCodecImplementation::new("internal", "WebRTC built-in"),
            vec![AudioCodecType::Opus],
            vec![AudioCodecType::Opus],
        ))]
    }

    fn sample_preference() -> AudioCodecPreference {
        AudioCodecPreference::new(vec![
            default_preference_codec(
                CodecDirection::Encoder,
                AudioCodecType::Opus,
                AudioCodecImplementation::new("internal", "WebRTC built-in"),
            ),
            default_preference_codec(
                CodecDirection::Decoder,
                AudioCodecType::Opus,
                AudioCodecImplementation::new("internal", "WebRTC built-in"),
            ),
        ])
    }

    #[test]
    fn create_preference_from_single_capability() {
        let capability = TestAudioCodecCapability::new(
            AudioCodecImplementation::new("internal", "WebRTC built-in"),
            vec![AudioCodecType::Opus],
            vec![AudioCodecType::Opus],
        );
        let preference = AudioCodecPreference::new_from_capability(&capability);
        assert!(
            preference
                .find(CodecDirection::Encoder, AudioCodecType::Opus)
                .is_some()
        );
        assert!(
            preference
                .find(CodecDirection::Decoder, AudioCodecType::Opus)
                .is_some()
        );
        assert!(
            preference
                .find(CodecDirection::Encoder, AudioCodecType::Isac)
                .is_none()
        );
    }

    fn assert_round_trip<T>(value: T)
    where
        T: Clone
            + std::fmt::Debug
            + PartialEq
            + DisplayJson
            + for<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>, Error = JsonParseError>,
    {
        let json_text = Json(&value).to_string();
        let parsed: Json<T> = json_text.parse().expect("JSON のパースに失敗しました");
        assert_eq!(parsed.0, value);
    }

    #[test]
    fn preference_codec_round_trip() {
        let codec = AudioPreferenceCodec::new(
            CodecDirection::Encoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("internal", "WebRTC built-in"),
        );
        assert_round_trip(codec);
    }

    #[test]
    fn preference_round_trip() {
        assert_round_trip(sample_preference());
    }

    #[test]
    fn validate_succeeds_with_supported_capabilities() {
        let preference = sample_preference();
        let capabilities = sample_capabilities();
        assert!(validate_audio_codec_preference(&preference, &capabilities).is_ok());
    }

    #[test]
    fn validate_fails_on_duplicate_codec_type_and_direction() {
        let preference = AudioCodecPreference::new(vec![
            default_preference_codec(
                CodecDirection::Encoder,
                AudioCodecType::Opus,
                AudioCodecImplementation::new("internal", "WebRTC built-in"),
            ),
            default_preference_codec(
                CodecDirection::Encoder,
                AudioCodecType::Opus,
                AudioCodecImplementation::new("other", "Other"),
            ),
        ]);
        let error = validate_audio_codec_preference(&preference, &sample_capabilities())
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidAudioCodecPreference { reason } => {
                assert_eq!(reason, "duplicate opus encoder");
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_fails_when_implementation_not_found() {
        let preference = AudioCodecPreference::new(vec![default_preference_codec(
            CodecDirection::Encoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("missing", "Missing"),
        )]);
        let error = validate_audio_codec_preference(&preference, &sample_capabilities())
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidAudioCodecPreference { reason } => {
                assert!(reason.contains("implementation not found"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_fails_on_duplicate_capability_implementation() {
        let preference = sample_preference();
        let capabilities: Vec<Box<dyn AudioCodecCapability>> = vec![
            Box::new(TestAudioCodecCapability::new(
                AudioCodecImplementation::new("internal", "WebRTC built-in"),
                vec![AudioCodecType::Opus],
                vec![AudioCodecType::Opus],
            )),
            Box::new(TestAudioCodecCapability::new(
                AudioCodecImplementation::new("internal", "another description"),
                vec![AudioCodecType::Opus],
                vec![AudioCodecType::Opus],
            )),
        ];
        let error = validate_audio_codec_preference(&preference, &capabilities)
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidAudioCodecCapability { reason } => {
                assert!(reason.contains("duplicate implementation in capabilities"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_fails_when_codec_type_not_found() {
        let preference = AudioCodecPreference::new(vec![default_preference_codec(
            CodecDirection::Encoder,
            AudioCodecType::Isac,
            AudioCodecImplementation::new("internal", "WebRTC built-in"),
        )]);
        let error = validate_audio_codec_preference(&preference, &sample_capabilities())
            .expect_err("失敗する必要があります");
        match error {
            Error::InvalidAudioCodecPreference { reason } => {
                assert!(reason.contains("codec type not found"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }

    #[test]
    fn validate_succeeds_when_supported_even_if_resolve_sdp_codec_spec_is_none() {
        let preference = AudioCodecPreference::new(vec![default_preference_codec(
            CodecDirection::Encoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("internal", "WebRTC built-in"),
        )]);
        let capabilities: Vec<Box<dyn AudioCodecCapability>> = vec![Box::new(
            TestAudioCodecCapability::new(
                AudioCodecImplementation::new("internal", "WebRTC built-in"),
                vec![AudioCodecType::Opus],
                Vec::new(),
            )
            .without_sdp_format_resolution(),
        )];
        assert!(
            validate_audio_codec_preference(&preference, &capabilities).is_ok(),
            "is_supported が true なら resolve が None でも検証は成功するはずです"
        );
    }

    #[test]
    fn get_or_add_has_implementation_and_merge_work() {
        let mut preference = AudioCodecPreference::default();
        let codec = preference.get_or_add(
            CodecDirection::Encoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("internal", "WebRTC built-in"),
        );
        codec.set_implementation(AudioCodecImplementation::new("internal", "WebRTC built-in"));
        assert!(
            preference
                .has_implementation(AudioCodecImplementation::new("internal", "WebRTC built-in"))
        );

        let merged = AudioCodecPreference::new(vec![default_preference_codec(
            CodecDirection::Decoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("other", "Other"),
        )]);
        preference.merge(&merged);
        let opus_decoder = preference
            .find(CodecDirection::Decoder, AudioCodecType::Opus)
            .expect("マージ後に opus デコーダーが存在する必要があります");
        assert_eq!(opus_decoder.implementation().name(), "other");
    }

    /// 片方向のみ対応の capability に対して、指定方向が非対応の場合に
    /// `{direction} not supported` で検証が失敗することを検証する。
    #[test]
    fn validate_fails_when_encoder_or_decoder_not_supported() {
        // エンコーダー非対応 (デコーダーは対応) の capability。
        // エンコーダー方向を指定すると "encoder not supported" になる。
        let encoder_only_capabilities: Vec<Box<dyn AudioCodecCapability>> =
            vec![Box::new(TestAudioCodecCapability::new(
                AudioCodecImplementation::new("internal", "WebRTC built-in"),
                Vec::new(),
                vec![AudioCodecType::Opus],
            ))];
        let encoder_preference = AudioCodecPreference::new(vec![default_preference_codec(
            CodecDirection::Encoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("internal", "WebRTC built-in"),
        )]);
        let error =
            validate_audio_codec_preference(&encoder_preference, &encoder_only_capabilities)
                .expect_err("失敗する必要があります");
        match error {
            Error::InvalidAudioCodecPreference { reason } => {
                assert!(reason.contains("encoder not supported"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }

        // デコーダー非対応 (エンコーダーは対応) の capability。
        // デコーダー方向を指定すると "decoder not supported" になる。
        let decoder_only_capabilities: Vec<Box<dyn AudioCodecCapability>> =
            vec![Box::new(TestAudioCodecCapability::new(
                AudioCodecImplementation::new("internal", "WebRTC built-in"),
                vec![AudioCodecType::Opus],
                Vec::new(),
            ))];
        let decoder_preference = AudioCodecPreference::new(vec![default_preference_codec(
            CodecDirection::Decoder,
            AudioCodecType::Opus,
            AudioCodecImplementation::new("internal", "WebRTC built-in"),
        )]);
        let error =
            validate_audio_codec_preference(&decoder_preference, &decoder_only_capabilities)
                .expect_err("失敗する必要があります");
        match error {
            Error::InvalidAudioCodecPreference { reason } => {
                assert!(reason.contains("decoder not supported"));
            }
            other => panic!("予期しないエラー: {other:?}"),
        }
    }
}
