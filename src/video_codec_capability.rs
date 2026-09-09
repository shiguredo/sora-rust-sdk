//! ビデオコーデックの実装情報と capability トレイト。
use shiguredo_webrtc::{
    EnvironmentRef, SdpVideoFormat, SdpVideoFormatRef, VideoCodecType, VideoDecoder, VideoEncoder,
    fuzzy_match_sdp_video_format,
};

use nojson::{DisplayJson, JsonFormatter, JsonParseError, RawJsonValue};

/// ビデオコーデックの実装情報。
///
/// `name` は `VideoCodecPreference` との突き合わせに利用される識別子で、
/// 実装ごとに一意である必要がある。
#[derive(Debug, Clone, Eq)]
pub struct VideoCodecImplementation {
    name: String,
    description: String,
}

impl VideoCodecImplementation {
    /// 新しい `VideoCodecImplementation` を生成する。
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

impl PartialEq for VideoCodecImplementation {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

// コーデックの方向 (エンコード/デコード) は共有モジュール `codec_direction` が提供する。
use crate::codec_direction::CodecDirection;

/// `VideoCodecCapability` は、各 codec 実装ごとの差分を吸収するためのインターフェース。
///
/// 各エンコーダー/デコーダーの実装ごとに `VideoCodecCapability` を実装することで
/// Sora クライアントから利用可能になる。
pub trait VideoCodecCapability: Send {
    /// この capability を識別する実装情報を返す。
    ///
    /// 実装名は `VideoCodecPreference` との突き合わせに利用されるため、実装ごとに一意である必要がある。
    fn get_implementation(&self) -> VideoCodecImplementation;

    /// 指定したエンコーダー/デコーダーでサポートされている SDP フォーマットのリストを返す。
    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat>;

    /// 指定方向で `codec_type` が利用可能かどうかを返す。
    ///
    /// デフォルト実装では `resolve_sdp_format()` による解決可否で判定する。
    fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
        let Some(codec_name) = codec_type.as_str() else {
            return false;
        };
        let requested = SdpVideoFormat::new(codec_name);
        self.resolve_sdp_format(direction, requested.as_ref())
            .is_some()
    }

    /// 要求 `format` に対して、実装が実際に利用する具体的な SDP フォーマットを解決する。
    ///
    /// デフォルト実装は `get_supported_formats()` に対する
    /// `fuzzy_match_sdp_video_format()` を使う。
    fn resolve_sdp_format(
        &self,
        direction: CodecDirection,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<SdpVideoFormat> {
        fuzzy_match_sdp_video_format(&self.get_supported_formats(direction), format)
    }

    /// 指定したフォーマットでエンコーダーがサポートされている場合は VideoEncoder を返す。
    ///
    /// create_video_encoder() は get_supported_formats() で返されるフォーマットのいずれかとマッチするフォーマットで呼び出されることが想定されている。
    /// get_supported_formats() で返されないフォーマットで呼び出された場合の動作は実装に依存するため、None が返されることは保証されない。
    #[expect(unused_variables)]
    fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        None
    }

    /// 指定したフォーマットでデコーダーがサポートされている場合は VideoDecoder を返す。
    ///
    /// create_video_decoder() は get_supported_formats() で返されるフォーマットのいずれかとマッチするフォーマットで呼び出されることが想定されている。
    /// get_supported_formats() で返されないフォーマットで呼び出された場合の動作は実装に依存するため、None が返されることは保証されない。
    #[expect(unused_variables)]
    fn create_video_decoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        None
    }
}

impl DisplayJson for VideoCodecImplementation {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("name", &self.name)?;
            f.member("description", &self.description)
        })
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for VideoCodecImplementation {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        let name: String = value.to_member("name")?.required()?.try_into()?;
        let description: String = value.to_member("description")?.required()?.try_into()?;
        Ok(Self::new_internal(name, description))
    }
}

/// `capabilities` の中から指定した実装名の capability を探す。
pub(crate) fn find_capability<'a>(
    capabilities: &'a [Box<dyn VideoCodecCapability>],
    implementation: &VideoCodecImplementation,
) -> Option<&'a dyn VideoCodecCapability> {
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
    use shiguredo_webrtc::VideoCodecType;

    use crate::testing::TestVideoCodecCapability;

    #[test]
    fn video_codec_implementation_round_trip() {
        let value = VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC");
        let json_text = Json(&value).to_string();
        let parsed: Json<VideoCodecImplementation> =
            json_text.parse().expect("JSON のパースに失敗しました");
        assert_eq!(parsed.0, value);
    }

    #[test]
    fn trait_works_with_trait_object() {
        let capability: Box<dyn VideoCodecCapability> = Box::new(TestVideoCodecCapability::new(
            VideoCodecImplementation::new("test", "Test Codec"),
            vec![VideoCodecType::H264],
            vec![VideoCodecType::H264],
        ));
        assert_eq!(capability.get_implementation().name(), "test");
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));
        let h264 = SdpVideoFormat::new("H264");
        let env = shiguredo_webrtc::Environment::new();
        assert!(
            capability
                .create_video_encoder(env.as_ref(), h264.as_ref())
                .is_some()
        );
        assert!(
            capability
                .create_video_decoder(env.as_ref(), h264.as_ref())
                .is_some()
        );
    }
}
