//! 音声コーデックの実装情報と capability トレイト。
use shiguredo_webrtc::{
    AudioCodecSpec, AudioCodecType, AudioDecoder, AudioEncoder, EnvironmentRef, SdpAudioFormatRef,
};

use nojson::{DisplayJson, JsonFormatter, JsonParseError, RawJsonValue};

use crate::video_codec_capability::CodecDirection;

/// 音声コーデックの実装情報。
///
/// `name` は `AudioCodecPreference` との突き合わせに利用される識別子で、
/// 実装ごとに一意である必要がある。
#[derive(Debug, Clone, Eq)]
pub struct AudioCodecImplementation {
    name: String,
    description: String,
}

impl AudioCodecImplementation {
    /// 新しい `AudioCodecImplementation` を生成する。
    pub fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
        }
    }

    fn new_internal(name: String, description: String) -> Self {
        Self { name, description }
    }

    /// 実装名を返す。
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// 実装の説明文を返す。
    pub fn description(&self) -> &str {
        self.description.as_str()
    }
}

impl PartialEq for AudioCodecImplementation {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

/// `AudioCodecCapability` は、各 codec 実装ごとの差分を吸収するためのインターフェース。
///
/// 各エンコーダー/デコーダーの実装ごとに `AudioCodecCapability` を実装することで
/// Sora クライアントから利用可能になる。
pub trait AudioCodecCapability: Send {
    /// この capability を識別する実装情報を返す。
    ///
    /// 実装名は `AudioCodecPreference` との突き合わせに利用されるため、実装ごとに一意である必要がある。
    fn get_implementation(&self) -> AudioCodecImplementation;

    /// 指定したエンコーダー/デコーダーでサポートされているコーデック仕様のリストを返す。
    fn get_supported_codec_specs(&self, direction: CodecDirection) -> Vec<AudioCodecSpec>;

    /// 指定方向で `codec_type` が利用可能かどうかを返す。
    ///
    /// デフォルト実装では [Self::get_supported_codec_specs] のフォーマット名で判定する。
    fn is_supported(&self, direction: CodecDirection, codec_type: AudioCodecType) -> bool {
        let Some(codec_name) = codec_type.as_str() else {
            return false;
        };
        self.get_supported_codec_specs(direction)
            .iter()
            .any(|spec| spec.format().name().ok().as_deref() == Some(codec_name))
    }

    /// 要求 `format` に対して、実装が実際に利用するコーデック仕様を返す。
    ///
    /// デフォルト実装は [Self::get_supported_codec_specs()] に対する
    /// [shiguredo_webrtc::SdpAudioFormat::matches] を使う。
    fn resolve_sdp_codec_spec(
        &self,
        direction: CodecDirection,
        format: SdpAudioFormatRef<'_>,
    ) -> Option<AudioCodecSpec> {
        let request = format.to_owned();
        self.get_supported_codec_specs(direction)
            .into_iter()
            .find(|spec| spec.format().matches(request.as_ref()))
    }

    /// 指定したフォーマットでエンコーダーがサポートされている場合は AudioEncoder を返す。
    ///
    /// create_audio_encoder() は resolve_sdp_codec_spec() で解決されるフォーマット
    /// (get_supported_codec_specs() のいずれかと一致するフォーマット) で呼び出される
    /// ことが想定されている。
    /// get_supported_codec_specs() で返されないフォーマットで呼び出された場合の動作は
    /// 実装に依存するため、None が返されることは保証されない。
    ///
    /// `payload_type` はネゴシエーションで決まった音声ペイロードタイプであり、
    /// エンコーダーの RTP ペイロードタイプとして利用する必要がある。
    #[expect(unused_variables)]
    fn create_audio_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
        payload_type: i32,
    ) -> Option<AudioEncoder> {
        None
    }

    /// 指定したフォーマットでデコーダーがサポートされている場合は AudioDecoder を返す。
    ///
    /// create_audio_decoder() は resolve_sdp_codec_spec() で解決されるフォーマット
    /// (get_supported_codec_specs() のいずれかと一致するフォーマット) で呼び出される
    /// ことが想定されている。
    /// get_supported_codec_specs() で返されないフォーマットで呼び出された場合の動作は
    /// 実装に依存するため、None が返されることは保証されない。
    #[expect(unused_variables)]
    fn create_audio_decoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpAudioFormatRef<'_>,
    ) -> Option<AudioDecoder> {
        None
    }
}

impl DisplayJson for AudioCodecImplementation {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("name", &self.name)?;
            f.member("description", &self.description)
        })
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for AudioCodecImplementation {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        let name: String = value.to_member("name")?.required()?.try_into()?;
        let description: String = value.to_member("description")?.required()?.try_into()?;
        Ok(Self::new_internal(name, description))
    }
}

/// `capabilities` の中から指定した実装名の capability を探す。
pub(crate) fn find_audio_capability<'a>(
    capabilities: &'a [Box<dyn AudioCodecCapability>],
    implementation: &AudioCodecImplementation,
) -> Option<&'a dyn AudioCodecCapability> {
    let implementation_name = implementation.name();
    capabilities
        .iter()
        .map(|capability| capability.as_ref())
        .find(|capability| capability.get_implementation().name() == implementation_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nojson::Json;
    use shiguredo_webrtc::{AudioCodecType, SdpAudioFormat};

    use crate::testing::TestAudioCodecCapability;

    #[test]
    fn audio_codec_implementation_round_trip() {
        let value = AudioCodecImplementation::new("internal", "WebRTC built-in");
        let json_text = Json(&value).to_string();
        let parsed: Json<AudioCodecImplementation> =
            json_text.parse().expect("JSON のパースに失敗しました");
        assert_eq!(parsed.0, value);
    }

    #[test]
    fn audio_trait_works_with_trait_object() {
        let capability: Box<dyn AudioCodecCapability> = Box::new(TestAudioCodecCapability::new(
            AudioCodecImplementation::new("test", "Test Codec"),
            vec![AudioCodecType::Opus],
            vec![AudioCodecType::Opus],
        ));
        assert_eq!(capability.get_implementation().name(), "test");
        assert!(capability.is_supported(CodecDirection::Encoder, AudioCodecType::Opus));
        assert!(capability.is_supported(CodecDirection::Decoder, AudioCodecType::Opus));
        let opus = SdpAudioFormat::new("opus", 48000, 2);
        let env = shiguredo_webrtc::Environment::new();
        assert!(
            capability
                .create_audio_encoder(env.as_ref(), opus.as_ref(), 111)
                .is_some()
        );
        assert!(
            capability
                .create_audio_decoder(env.as_ref(), opus.as_ref())
                .is_some()
        );
    }
}
