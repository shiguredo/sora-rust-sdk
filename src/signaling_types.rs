//! シグナリング用の型定義と JSON 変換。
use nojson::{DisplayJson, JsonFormatter, JsonParseError, RawJson, RawJsonValue};

use crate::error::{Error, Result};
use crate::types::{Audio, ConnectDataChannel, ForwardingFilter, JsonString, Role, Video};

#[derive(Debug, Clone)]
pub(crate) struct IceServerConfig {
    pub(crate) urls: Vec<String>,
    pub(crate) username: Option<String>,
    pub(crate) credential: Option<String>,
}

// -------------------------
// DataChannel 設定
// -------------------------

#[derive(Debug, Clone)]
pub(crate) struct DataChannelConfig {
    pub(crate) label: String,
    pub(crate) compress: bool,
    #[expect(dead_code)]
    pub(crate) direction: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SimulcastScaleResolutionDownToConfig {
    pub(crate) max_width: i32,
    pub(crate) max_height: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SimulcastEncodingConfig {
    pub(crate) rid: String,
    pub(crate) max_bitrate: Option<i32>,
    pub(crate) min_bitrate: Option<i32>,
    pub(crate) scale_resolution_down_by: Option<f64>,
    pub(crate) max_framerate: Option<f64>,
    pub(crate) active: Option<bool>,
    pub(crate) adaptive_ptime: Option<bool>,
    pub(crate) scalability_mode: Option<String>,
    pub(crate) scale_resolution_down_to: Option<SimulcastScaleResolutionDownToConfig>,
}

#[derive(Debug, Clone)]
pub(crate) enum IncomingMessageData {
    Offer {
        sdp: String,
        ice_servers: Vec<IceServerConfig>,
        data_channels: Vec<DataChannelConfig>,
        simulcast: bool,
        encodings: Vec<SimulcastEncodingConfig>,
    },
    ReOffer {
        sdp: String,
    },
    Ping {
        stats: Option<bool>,
    },
    ReqStats {},
    Notify {},
    Push {},
    Switched {
        ignore_disconnect_websocket: bool,
    },
    Redirect {
        location: String,
    },
    Close {
        code: u16,
        reason: String,
    },
}

pub(crate) struct IncomingMessage {
    pub(crate) message: String,
    pub(crate) data: IncomingMessageData,
}

impl IncomingMessage {
    pub(crate) fn parse(text: &str) -> Result<IncomingMessage> {
        let json = RawJson::parse(text)?;
        let value = json.value();
        let data = IncomingMessageData::try_from(value)?;
        Ok(IncomingMessage {
            message: text.to_string(),
            data,
        })
    }

    /// `config.iceServers` をパースする。
    ///
    /// `config` / `iceServers` が存在しない場合は空リストを返す。
    pub fn parse_ice_servers(value: RawJsonValue) -> Result<Vec<IceServerConfig>> {
        Self::parse_optional(value, "config", |config| {
            Self::parse_optional(config, "iceServers", |ice_servers| {
                ice_servers.to_array()?.map(|v| v.try_into()).collect()
            })
        })
    }

    pub fn parse_data_channels(value: RawJsonValue) -> Result<Vec<DataChannelConfig>> {
        Self::parse_optional(value, "data_channels", |v| {
            v.to_array()?.map(|v| v.try_into()).collect()
        })
    }

    pub fn parse_simulcast_encodings(value: RawJsonValue) -> Result<Vec<SimulcastEncodingConfig>> {
        Self::parse_optional(value, "encodings", |v| {
            v.to_array()?.map(|v| v.try_into()).collect()
        })
    }

    /// オプショナルなメンバーを取得して `parse` でパースする。
    ///
    /// `member` が存在しない場合は空リストを返す。
    fn parse_optional<'text, 'raw, T, F>(
        value: RawJsonValue<'text, 'raw>,
        member: &str,
        parse: F,
    ) -> Result<Vec<T>>
    where
        F: FnOnce(RawJsonValue<'text, 'raw>) -> Result<Vec<T>>,
    {
        value
            .to_member(member)?
            .optional()
            .map(parse)
            .unwrap_or(Ok(vec![]))
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for IncomingMessageData {
    type Error = Error;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> Result<Self> {
        let message_type_value = value.to_member("type")?.required()?;
        let message_type_text: String = message_type_value.try_into()?;
        match message_type_text.as_str() {
            "offer" => {
                let sdp = value.to_member("sdp")?.required()?.try_into()?;
                let ice_servers = IncomingMessage::parse_ice_servers(value)?;
                let data_channels = IncomingMessage::parse_data_channels(value)?;
                let simulcast = value
                    .to_member("simulcast")?
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()?
                    .unwrap_or(false);
                let encodings = IncomingMessage::parse_simulcast_encodings(value)?;
                Ok(Self::Offer {
                    sdp,
                    ice_servers,
                    data_channels,
                    simulcast,
                    encodings,
                })
            }
            "re-offer" => {
                let sdp = value.to_member("sdp")?.required()?.try_into()?;
                Ok(Self::ReOffer { sdp })
            }
            "ping" => {
                let stats = value
                    .to_member("stats")?
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()?;
                Ok(Self::Ping { stats })
            }
            "req-stats" => Ok(Self::ReqStats {}),
            "notify" => Ok(Self::Notify {}),
            "push" => Ok(Self::Push {}),
            "switched" => {
                let ignore_disconnect_websocket = value
                    .to_member("ignore_disconnect_websocket")?
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()?
                    .unwrap_or(false);
                Ok(Self::Switched {
                    ignore_disconnect_websocket,
                })
            }
            "redirect" => {
                let location = value.to_member("location")?.required()?.try_into()?;
                Ok(Self::Redirect { location })
            }
            "close" => {
                // Sora ドキュメント「シグナリングの型定義」の SignalingCloseMessage に基づき、
                // code と reason は必須として検証する。
                let code = value.to_member("code")?.required()?.try_into()?;
                let reason = value.to_member("reason")?.required()?.try_into()?;
                Ok(Self::Close { code, reason })
            }
            "candidate" => Err(Error::CandidateNotSupported),
            other => Err(Error::UnsupportedMessageType {
                message_type: other.to_string(),
            }),
        }
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for IceServerConfig {
    type Error = Error;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> Result<Self> {
        let urls = value
            .to_member("urls")?
            .required()?
            .to_array()?
            .map(|v| v.try_into())
            .collect::<std::result::Result<Vec<String>, JsonParseError>>()?;
        let username: Option<String> = value
            .to_member("username")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        let credential: Option<String> = value
            .to_member("credential")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;

        Ok(Self {
            urls,
            username,
            credential,
        })
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for DataChannelConfig {
    type Error = Error;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> Result<Self> {
        let label = value.to_member("label")?.required()?.try_into()?;
        let compress = value.to_member("compress")?.required()?.try_into()?;
        let direction = value.to_member("direction")?.required()?.try_into()?;

        Ok(Self {
            label,
            compress,
            direction,
        })
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for SimulcastScaleResolutionDownToConfig {
    type Error = Error;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> Result<Self> {
        let max_width = value.to_member("maxWidth")?.required()?.try_into()?;
        let max_height = value.to_member("maxHeight")?.required()?.try_into()?;
        Ok(Self {
            max_width,
            max_height,
        })
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for SimulcastEncodingConfig {
    type Error = Error;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> Result<Self> {
        let rid = value.to_member("rid")?.required()?.try_into()?;
        let max_bitrate = value
            .to_member("maxBitrate")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        let min_bitrate = value
            .to_member("minBitrate")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        let scale_resolution_down_by = value
            .to_member("scaleResolutionDownBy")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        let max_framerate = value
            .to_member("maxFramerate")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        let active = value
            .to_member("active")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        let adaptive_ptime = value
            .to_member("adaptivePtime")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        let scalability_mode = value
            .to_member("scalabilityMode")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        let scale_resolution_down_to = value
            .to_member("scaleResolutionDownTo")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;
        Ok(Self {
            rid,
            max_bitrate,
            min_bitrate,
            scale_resolution_down_by,
            max_framerate,
            active,
            adaptive_ptime,
            scalability_mode,
            scale_resolution_down_to,
        })
    }
}

#[expect(clippy::large_enum_variant)]
pub(crate) enum OutgoingMessage {
    Connect {
        channel_id: String,
        role: Role,
        client_id: Option<String>,
        bundle_id: Option<String>,
        redirect: bool,
        sora_client: String,
        libwebrtc: String,
        environment: String,
        metadata: Option<JsonString>,
        data_channel_signaling: Option<bool>,
        ignore_disconnect_websocket: Option<bool>,
        simulcast: Option<bool>,
        simulcast_request_rid: Option<String>,
        spotlight: Option<bool>,
        spotlight_focus_rid: Option<String>,
        spotlight_unfocus_rid: Option<String>,
        signaling_notify_metadata: Option<JsonString>,
        data_channels: Option<Vec<ConnectDataChannel>>,
        forwarding_filters: Option<Vec<ForwardingFilter>>,
        audio: Option<Audio>,
        video: Option<Video>,
    },
    Answer {
        sdp: String,
    },
    ReAnswer {
        sdp: String,
    },
    Pong {
        stats: Option<JsonString>,
    },
    Stats {
        reports: JsonString,
    },
    Candidate {
        candidate: String,
    },
    Disconnect,
}

impl DisplayJson for OutgoingMessage {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            OutgoingMessage::Connect {
                channel_id,
                role,
                client_id,
                bundle_id,
                redirect,
                sora_client,
                libwebrtc,
                environment,
                metadata,
                data_channel_signaling,
                ignore_disconnect_websocket,
                simulcast,
                simulcast_request_rid,
                spotlight,
                spotlight_focus_rid,
                spotlight_unfocus_rid,
                signaling_notify_metadata,
                data_channels,
                forwarding_filters,
                audio,
                video,
            } => f.object(|f| {
                f.member("type", "connect")?;
                f.member("channel_id", channel_id)?;
                if let Some(client_id) = client_id {
                    f.member("client_id", client_id)?;
                }
                if let Some(bundle_id) = bundle_id {
                    f.member("bundle_id", bundle_id)?;
                }
                if *redirect {
                    f.member("redirect", true)?;
                }
                f.member("role", role.as_sora_role())?;
                f.member("sora_client", sora_client)?;
                f.member("libwebrtc", libwebrtc)?;
                f.member("environment", environment)?;
                if let Some(metadata) = metadata {
                    f.member("metadata", metadata)?;
                }
                if let Some(data_channel_signaling) = data_channel_signaling {
                    f.member("data_channel_signaling", data_channel_signaling)?;
                }
                if let Some(ignore_disconnect_websocket) = ignore_disconnect_websocket {
                    f.member("ignore_disconnect_websocket", ignore_disconnect_websocket)?;
                }
                if let Some(simulcast) = simulcast {
                    f.member("simulcast", simulcast)?;
                }
                if let Some(simulcast_request_rid) = simulcast_request_rid {
                    f.member("simulcast_request_rid", simulcast_request_rid)?;
                }
                if let Some(spotlight) = spotlight {
                    f.member("spotlight", spotlight)?;
                }
                if let Some(spotlight_focus_rid) = spotlight_focus_rid {
                    f.member("spotlight_focus_rid", spotlight_focus_rid)?;
                }
                if let Some(spotlight_unfocus_rid) = spotlight_unfocus_rid {
                    f.member("spotlight_unfocus_rid", spotlight_unfocus_rid)?;
                }
                if let Some(signaling_notify_metadata) = signaling_notify_metadata {
                    f.member("signaling_notify_metadata", signaling_notify_metadata)?;
                }
                if let Some(data_channels) = data_channels {
                    f.member("data_channels", data_channels)?;
                }
                if let Some(forwarding_filters) = forwarding_filters {
                    f.member("forwarding_filters", forwarding_filters)?;
                }
                if let Some(audio) = audio {
                    f.member("audio", audio)?;
                }
                if let Some(video) = video {
                    f.member("video", video)?;
                }
                Ok(())
            }),
            OutgoingMessage::Answer { sdp } => f.object(|f| {
                f.member("type", "answer")?;
                f.member("sdp", sdp)
            }),
            OutgoingMessage::ReAnswer { sdp } => f.object(|f| {
                f.member("type", "re-answer")?;
                f.member("sdp", sdp)
            }),
            OutgoingMessage::Pong { stats } => f.object(|f| {
                f.member("type", "pong")?;
                if let Some(stats) = stats {
                    f.member("stats", stats)?;
                }
                Ok(())
            }),
            OutgoingMessage::Stats { reports } => f.object(|f| {
                f.member("type", "stats")?;
                f.member("reports", reports)
            }),
            OutgoingMessage::Candidate { candidate } => f.object(|f| {
                f.member("type", "candidate")?;
                f.member("candidate", candidate)
            }),
            OutgoingMessage::Disconnect => f.object(|f| {
                f.member("type", "disconnect")?;
                f.member("reason", "NO-ERROR")
            }),
        }
    }
}

impl OutgoingMessage {
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new_connect(
        channel_id: &str,
        role: Role,
        client_id: Option<String>,
        bundle_id: Option<String>,
        redirect: bool,
        sora_client: String,
        libwebrtc: String,
        environment: String,
        metadata: Option<JsonString>,
        data_channel_signaling: Option<bool>,
        ignore_disconnect_websocket: Option<bool>,
        simulcast: Option<bool>,
        simulcast_request_rid: Option<String>,
        spotlight: Option<bool>,
        spotlight_focus_rid: Option<String>,
        spotlight_unfocus_rid: Option<String>,
        signaling_notify_metadata: Option<JsonString>,
        data_channels: Option<Vec<ConnectDataChannel>>,
        forwarding_filters: Option<Vec<ForwardingFilter>>,
        audio: Option<Audio>,
        video: Option<Video>,
    ) -> Self {
        Self::Connect {
            channel_id: channel_id.to_string(),
            role,
            client_id,
            bundle_id,
            redirect,
            sora_client,
            libwebrtc,
            environment,
            metadata,
            data_channel_signaling,
            ignore_disconnect_websocket,
            simulcast,
            simulcast_request_rid,
            spotlight,
            spotlight_focus_rid,
            spotlight_unfocus_rid,
            signaling_notify_metadata,
            data_channels,
            forwarding_filters,
            audio,
            video,
        }
    }
    pub(crate) fn new_answer(sdp: &str) -> Self {
        Self::Answer {
            sdp: sdp.to_string(),
        }
    }
    pub(crate) fn new_reanswer(sdp: &str) -> Self {
        Self::ReAnswer {
            sdp: sdp.to_string(),
        }
    }
    pub(crate) fn new_pong(stats: Option<JsonString>) -> Self {
        Self::Pong { stats }
    }
    pub(crate) fn new_stats(reports: JsonString) -> Self {
        Self::Stats { reports }
    }
    pub(crate) fn new_candidate(candidate: &str) -> Self {
        Self::Candidate {
            candidate: candidate.to_string(),
        }
    }
    pub(crate) fn new_disconnect() -> Self {
        Self::Disconnect
    }
}

#[cfg(test)]
mod tests {
    use nojson::Json;

    use super::*;

    /// Close メッセージの parse が成功して code と reason を保持することを確認する。
    fn assert_close_parsed(text: &str, code: u16, reason: &str) {
        let message =
            IncomingMessage::parse(text).expect("Close メッセージの parse に失敗しました");
        match message.data {
            IncomingMessageData::Close {
                code: actual_code,
                reason: actual_reason,
            } => {
                assert_eq!(actual_code, code);
                assert_eq!(actual_reason, reason);
            }
            _ => panic!("Close 以外のメッセージとして parse されました: {text}"),
        }
    }

    /// Close メッセージの parse が失敗することを確認する。
    fn assert_close_parse_failed(text: &str) {
        let result = IncomingMessage::parse(text);
        assert!(
            result.is_err(),
            "不正な Close メッセージが parse されてしまいました: {text}"
        );
    }

    #[test]
    fn close_accepts_code_1000_with_reason() {
        assert_close_parsed(
            r#"{"type":"close","code":1000,"reason":"DISCONNECTED-API"}"#,
            1000,
            "DISCONNECTED-API",
        );
    }

    #[test]
    fn close_accepts_code_4490_with_reason() {
        assert_close_parsed(
            r#"{"type":"close","code":4490,"reason":"INTERNAL-ERROR"}"#,
            4490,
            "INTERNAL-ERROR",
        );
    }

    #[test]
    fn close_rejects_missing_code() {
        assert_close_parse_failed(r#"{"type":"close","reason":"DISCONNECTED-API"}"#);
    }

    #[test]
    fn close_rejects_missing_reason() {
        assert_close_parse_failed(r#"{"type":"close","code":1000}"#);
    }

    #[test]
    fn close_rejects_string_code() {
        assert_close_parse_failed(r#"{"type":"close","code":"1000","reason":"DISCONNECTED-API"}"#);
    }

    #[test]
    fn close_rejects_float_code() {
        assert_close_parse_failed(r#"{"type":"close","code":1000.5,"reason":"DISCONNECTED-API"}"#);
    }

    #[test]
    fn close_rejects_negative_code() {
        assert_close_parse_failed(r#"{"type":"close","code":-1,"reason":"DISCONNECTED-API"}"#);
    }

    #[test]
    fn close_rejects_overflowing_code() {
        assert_close_parse_failed(r#"{"type":"close","code":65536,"reason":"DISCONNECTED-API"}"#);
    }

    #[test]
    fn close_rejects_non_string_reason() {
        assert_close_parse_failed(r#"{"type":"close","code":1000,"reason":12345}"#);
    }

    /// Disconnect メッセージが `{"type":"disconnect","reason":"NO-ERROR"}` にシリアライズされることを確認する。
    #[test]
    fn disconnect_serializes_to_no_error() {
        let text = Json(OutgoingMessage::new_disconnect()).to_string();
        assert_eq!(text, r#"{"type":"disconnect","reason":"NO-ERROR"}"#);
    }

    #[test]
    fn offer_accepts_missing_ice_servers() {
        // offer でも iceServers が無い場合は空リストとして受理する。
        let message = IncomingMessage::parse(r#"{"type":"offer","sdp":"sdp","config":{}}"#)
            .expect("iceServers が無い offer はパースできるべきです");
        match message.data {
            IncomingMessageData::Offer { ice_servers, .. } => {
                assert!(
                    ice_servers.is_empty(),
                    "iceServers は空リストになるべきです"
                );
            }
            _ => panic!("Offer としてパースされるべきです"),
        }
    }

    #[test]
    fn offer_parses_ice_servers() {
        // offer の config.iceServers が正しくパースされることを確認する。
        let text = r#"{"type":"offer","sdp":"sdp","config":{"iceServers":[{"urls":["stun:example.com:3478"]}]}}"#;
        let message =
            IncomingMessage::parse(text).expect("iceServers 付き offer のパースに失敗しました");
        match message.data {
            IncomingMessageData::Offer { ice_servers, .. } => {
                assert_eq!(ice_servers.len(), 1, "iceServers の件数が期待と異なります");
                assert_eq!(
                    ice_servers[0].urls,
                    vec!["stun:example.com:3478"],
                    "iceServers の URL が期待と異なります"
                );
            }
            _ => panic!("Offer としてパースされるべきです"),
        }
    }
}
