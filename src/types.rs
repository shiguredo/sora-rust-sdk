//! 公開型と接続設定用の型。
use nojson::{DisplayJson, JsonFormatter, RawJsonOwned};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalingType {
    WebSocket,
    DataChannel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalingDirection {
    Sent,
    Received,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    SendOnly,
    RecvOnly,
    SendRecv,
}

impl Role {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "sendonly" => Ok(Self::SendOnly),
            "recvonly" => Ok(Self::RecvOnly),
            "sendrecv" => Ok(Self::SendRecv),
            _ => Err(Error::InvalidRole {
                value: value.to_string(),
            }),
        }
    }

    pub fn as_sora_role(self) -> &'static str {
        match self {
            Role::SendOnly => "sendonly",
            Role::RecvOnly => "recvonly",
            Role::SendRecv => "sendrecv",
        }
    }

    pub fn wants_send(self) -> bool {
        matches!(self, Role::SendOnly | Role::SendRecv)
    }

    pub fn wants_recv(self) -> bool {
        matches!(self, Role::RecvOnly | Role::SendRecv)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxyInfo {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JsonString {
    raw: RawJsonOwned,
}

impl std::str::FromStr for JsonString {
    type Err = Error;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        RawJsonOwned::parse(value)
            .map(Self::from)
            .map_err(Error::from)
    }
}

impl std::fmt::Display for JsonString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw.to_string())
    }
}

impl From<RawJsonOwned> for JsonString {
    fn from(raw: RawJsonOwned) -> Self {
        Self { raw }
    }
}

impl DisplayJson for JsonString {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        DisplayJson::fmt(&self.raw, f)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpusParams {
    pub channels: Option<u32>,
    pub maxplaybackrate: Option<u32>,
    pub minptime: Option<u32>,
    pub ptime: Option<u32>,
    pub stereo: Option<bool>,
    pub sprop_stereo: Option<bool>,
    pub useinbandfec: Option<bool>,
    pub usedtx: Option<bool>,
}

impl DisplayJson for OpusParams {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            if let Some(channels) = self.channels {
                f.member("channels", channels)?;
            }
            if let Some(maxplaybackrate) = self.maxplaybackrate {
                f.member("maxplaybackrate", maxplaybackrate)?;
            }
            if let Some(minptime) = self.minptime {
                f.member("minptime", minptime)?;
            }
            if let Some(ptime) = self.ptime {
                f.member("ptime", ptime)?;
            }
            if let Some(stereo) = self.stereo {
                f.member("stereo", stereo)?;
            }
            if let Some(sprop_stereo) = self.sprop_stereo {
                f.member("sprop_stereo", sprop_stereo)?;
            }
            if let Some(useinbandfec) = self.useinbandfec {
                f.member("useinbandfec", useinbandfec)?;
            }
            if let Some(usedtx) = self.usedtx {
                f.member("usedtx", usedtx)?;
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodecType {
    VP8,
    VP9,
    H264,
    H265,
    AV1,
}

impl DisplayJson for VideoCodecType {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            VideoCodecType::VP8 => f.value("VP8"),
            VideoCodecType::VP9 => f.value("VP9"),
            VideoCodecType::H264 => f.value("H264"),
            VideoCodecType::H265 => f.value("H265"),
            VideoCodecType::AV1 => f.value("AV1"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodecType {
    OPUS,
}

impl DisplayJson for AudioCodecType {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            AudioCodecType::OPUS => f.value("OPUS"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audio {
    Bool(bool),
    Audio {
        codec_type: Option<AudioCodecType>,
        bit_rate: Option<u32>,
        opus_params: Option<OpusParams>,
    },
}

impl Audio {
    pub fn new_bool(enabled: bool) -> Self {
        Self::Bool(enabled)
    }
    pub fn new_opus(bit_rate: Option<u32>, opus_params: Option<OpusParams>) -> Self {
        Self::Audio {
            codec_type: Some(AudioCodecType::OPUS),
            bit_rate,
            opus_params,
        }
    }
}

impl DisplayJson for Audio {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Audio::Bool(b) => f.value(*b),
            Audio::Audio {
                codec_type,
                bit_rate,
                opus_params,
            } => f.object(|f| {
                if let Some(codec_type) = codec_type {
                    f.member("codec_type", codec_type)?;
                }
                if let Some(bit_rate) = bit_rate {
                    f.member("bit_rate", bit_rate)?;
                }
                if let Some(opus_params) = opus_params {
                    f.member("opus_params", opus_params)?;
                }
                Ok(())
            }),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoVP9Params {
    // 0..3
    pub profile_id: Option<u32>,
}

impl DisplayJson for VideoVP9Params {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            if let Some(profile_id) = self.profile_id {
                f.member("profile_id", profile_id)?;
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoH264Params {
    pub profile_level_id: Option<String>,
    // sora.conf で h264_b_frame = true を設定する必要があります
    pub b_frame: Option<bool>,
}

impl DisplayJson for VideoH264Params {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            if let Some(profile_level_id) = self.profile_level_id.as_ref() {
                f.member("profile_level_id", profile_level_id)?;
            }
            if let Some(b_frame) = self.b_frame {
                f.member("b_frame", b_frame)?;
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoH265Params {
    pub level_id: Option<String>,
    // 0..31
    pub profile_id: Option<u32>,
    // 0..1
    pub tier_flag: Option<u32>,
    // "SRST" | "MRST" | "MRMT"
    pub tx_mode: Option<String>,
    // sora.conf で h265_b_frame = true を設定する必要があります
    pub b_frame: Option<bool>,
}

impl DisplayJson for VideoH265Params {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            if let Some(level_id) = self.level_id.as_ref() {
                f.member("level_id", level_id)?;
            }
            if let Some(profile_id) = self.profile_id {
                f.member("profile_id", profile_id)?;
            }
            if let Some(tier_flag) = self.tier_flag {
                f.member("tier_flag", tier_flag)?;
            }
            if let Some(tx_mode) = self.tx_mode.as_ref() {
                f.member("tx_mode", tx_mode)?;
            }
            if let Some(b_frame) = self.b_frame {
                f.member("b_frame", b_frame)?;
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoAV1Params {
    // 0..2
    pub profile: Option<u32>,
    // 0..31
    pub level_idx: Option<u32>,
    // 0..1
    pub tier: Option<u32>,
}

impl DisplayJson for VideoAV1Params {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            if let Some(profile) = self.profile {
                f.member("profile", profile)?;
            }
            if let Some(level_idx) = self.level_idx {
                f.member("level_idx", level_idx)?;
            }
            if let Some(tier) = self.tier {
                f.member("tier", tier)?;
            }
            Ok(())
        })
    }
}

// -------------------------
// ConnectDataChannel
// -------------------------

#[derive(Debug, Clone)]
pub struct ConnectDataChannel {
    pub label: String,
    pub direction: String,
    pub ordered: Option<bool>,
    pub max_packet_life_time: Option<i32>,
    pub max_retransmits: Option<i32>,
    pub protocol: Option<String>,
    pub compress: Option<bool>,
    pub header: Option<Vec<JsonString>>,
}

impl DisplayJson for ConnectDataChannel {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("label", &self.label)?;
            f.member("direction", &self.direction)?;
            if let Some(ordered) = self.ordered {
                f.member("ordered", ordered)?;
            }
            if let Some(max_packet_life_time) = self.max_packet_life_time {
                f.member("max_packet_life_time", max_packet_life_time)?;
            }
            if let Some(max_retransmits) = self.max_retransmits {
                f.member("max_retransmits", max_retransmits)?;
            }
            if let Some(protocol) = &self.protocol {
                f.member("protocol", protocol)?;
            }
            if let Some(compress) = self.compress {
                f.member("compress", compress)?;
            }
            if let Some(header) = &self.header {
                f.member("header", header)?;
            }
            Ok(())
        })
    }
}

// -------------------------
// ForwardingFilter
// -------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardingFilterRule {
    pub field: String,
    pub operator: String,
    pub values: Vec<String>,
}

impl DisplayJson for ForwardingFilterRule {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("field", &self.field)?;
            f.member("operator", &self.operator)?;
            f.member("values", &self.values)
        })
    }
}

#[derive(Debug, Clone)]
pub struct ForwardingFilter {
    pub name: Option<String>,
    pub priority: Option<i32>,
    pub action: Option<String>,
    pub rules: Vec<Vec<ForwardingFilterRule>>,
    pub version: Option<String>,
    pub metadata: Option<JsonString>,
}

impl DisplayJson for ForwardingFilter {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            if let Some(name) = &self.name {
                f.member("name", name)?;
            }
            if let Some(priority) = self.priority {
                f.member("priority", priority)?;
            }
            if let Some(action) = &self.action {
                f.member("action", action)?;
            }
            f.member("rules", &self.rules)?;
            if let Some(version) = &self.version {
                f.member("version", version)?;
            }
            if let Some(metadata) = &self.metadata {
                f.member("metadata", metadata)?;
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Video {
    Bool(bool),
    Video {
        codec_type: Option<VideoCodecType>,
        bit_rate: Option<u32>,
        vp9_params: Option<VideoVP9Params>,
        av1_params: Option<VideoAV1Params>,
        h264_params: Option<VideoH264Params>,
        h265_params: Option<VideoH265Params>,
    },
}

impl Video {
    pub fn new_bool(enabled: bool) -> Self {
        Self::Bool(enabled)
    }
    pub fn new_vp8(bit_rate: Option<u32>) -> Self {
        Self::Video {
            codec_type: Some(VideoCodecType::VP8),
            bit_rate,
            vp9_params: None,
            av1_params: None,
            h264_params: None,
            h265_params: None,
        }
    }
    pub fn new_vp9(bit_rate: Option<u32>, vp9_params: Option<VideoVP9Params>) -> Self {
        Self::Video {
            codec_type: Some(VideoCodecType::VP9),
            bit_rate,
            vp9_params,
            av1_params: None,
            h264_params: None,
            h265_params: None,
        }
    }
    pub fn new_av1(bit_rate: Option<u32>, av1_params: Option<VideoAV1Params>) -> Self {
        Self::Video {
            codec_type: Some(VideoCodecType::AV1),
            bit_rate,
            vp9_params: None,
            av1_params,
            h264_params: None,
            h265_params: None,
        }
    }
    pub fn new_h264(bit_rate: Option<u32>, h264_params: Option<VideoH264Params>) -> Self {
        Self::Video {
            codec_type: Some(VideoCodecType::H264),
            bit_rate,
            vp9_params: None,
            av1_params: None,
            h264_params,
            h265_params: None,
        }
    }
    pub fn new_h265(bit_rate: Option<u32>, h265_params: Option<VideoH265Params>) -> Self {
        Self::Video {
            codec_type: Some(VideoCodecType::H265),
            bit_rate,
            vp9_params: None,
            av1_params: None,
            h264_params: None,
            h265_params,
        }
    }
}

impl DisplayJson for Video {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Video::Bool(b) => f.value(*b),
            Video::Video {
                codec_type,
                bit_rate,
                vp9_params,
                av1_params,
                h264_params,
                h265_params,
            } => f.object(|f| {
                if let Some(codec_type) = codec_type {
                    f.member("codec_type", codec_type)?;
                }
                if let Some(bit_rate) = bit_rate {
                    f.member("bit_rate", bit_rate)?;
                }
                if let Some(vp9_params) = vp9_params {
                    f.member("vp9_params", vp9_params)?;
                }
                if let Some(av1_params) = av1_params {
                    f.member("av1_params", av1_params)?;
                }
                if let Some(h264_params) = h264_params {
                    f.member("h264_params", h264_params)?;
                }
                if let Some(h265_params) = h265_params {
                    f.member("h265_params", h265_params)?;
                }
                Ok(())
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nojson::Json;

    #[test]
    fn audio_new_opus_serializes_opus_params() {
        let opus_params = OpusParams {
            channels: Some(2),
            maxplaybackrate: Some(48_000),
            minptime: Some(10),
            ptime: Some(20),
            stereo: Some(true),
            sprop_stereo: Some(true),
            useinbandfec: Some(true),
            usedtx: Some(false),
        };
        let audio = Audio::new_opus(Some(64_000), Some(opus_params));
        let json = Json(&audio).to_string();

        assert_eq!(
            json,
            r#"{"codec_type":"OPUS","bit_rate":64000,"opus_params":{"channels":2,"maxplaybackrate":48000,"minptime":10,"ptime":20,"stereo":true,"sprop_stereo":true,"useinbandfec":true,"usedtx":false}}"#
        );
    }

    #[test]
    fn video_new_vp9_serializes_vp9_params() {
        let vp9_params = VideoVP9Params {
            profile_id: Some(2),
        };
        let video = Video::new_vp9(Some(512_000), Some(vp9_params));
        let json = Json(&video).to_string();

        assert_eq!(
            json,
            r#"{"codec_type":"VP9","bit_rate":512000,"vp9_params":{"profile_id":2}}"#
        );
    }

    #[test]
    fn video_new_av1_serializes_av1_params() {
        let av1_params = VideoAV1Params {
            profile: Some(1),
            level_idx: Some(10),
            tier: Some(1),
        };
        let video = Video::new_av1(Some(768_000), Some(av1_params));
        let json = Json(&video).to_string();

        assert_eq!(
            json,
            r#"{"codec_type":"AV1","bit_rate":768000,"av1_params":{"profile":1,"level_idx":10,"tier":1}}"#
        );
    }

    #[test]
    fn video_new_h264_serializes_h264_params() {
        let h264_params = VideoH264Params {
            profile_level_id: Some("42e01f".to_string()),
            b_frame: Some(true),
        };
        let video = Video::new_h264(Some(1_000_000), Some(h264_params));
        let json = Json(&video).to_string();

        assert_eq!(
            json,
            r#"{"codec_type":"H264","bit_rate":1000000,"h264_params":{"profile_level_id":"42e01f","b_frame":true}}"#
        );
    }

    #[test]
    fn video_new_h265_serializes_h265_params() {
        let h265_params = VideoH265Params {
            level_id: Some("120".to_string()),
            profile_id: Some(1),
            tier_flag: Some(0),
            tx_mode: Some("MRST".to_string()),
            b_frame: Some(false),
        };
        let video = Video::new_h265(Some(1_200_000), Some(h265_params));
        let json = Json(&video).to_string();

        assert_eq!(
            json,
            r#"{"codec_type":"H265","bit_rate":1200000,"h265_params":{"level_id":"120","profile_id":1,"tier_flag":0,"tx_mode":"MRST","b_frame":false}}"#
        );
    }
}
