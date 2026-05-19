use nojson::{JsonParseError, RawJson, RawJsonValue};
use sora_sdk::JsonString;

type JsonResult<T> = std::result::Result<T, JsonParseError>;

pub trait RtcStatsTrait {
    fn timestamp(&self) -> f64;
    fn stats_type(&self) -> RtcStatsType;
    fn id(&self) -> String;
}

pub trait RtcRtpStreamStatsTrait: RtcStatsTrait {
    fn ssrc(&self) -> u64;
    fn kind(&self) -> String;
    fn transport_id(&self) -> Option<String>;
    fn codec_id(&self) -> Option<String>;
}

pub trait RtcReceivedRtpStreamStatsTrait: RtcRtpStreamStatsTrait {
    fn packets_received(&self) -> Option<u64>;
    fn packets_lost(&self) -> Option<i64>;
    fn jitter(&self) -> Option<f64>;
}

pub trait RtcSentRtpStreamStatsTrait: RtcRtpStreamStatsTrait {
    fn packets_sent(&self) -> Option<u64>;
    fn bytes_sent(&self) -> Option<u64>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtcStatsType {
    Codec,
    InboundRtp,
    OutboundRtp,
    RemoteInboundRtp,
    RemoteOutboundRtp,
    MediaSource,
    MediaPlayout,
    PeerConnection,
    DataChannel,
    Transport,
    CandidatePair,
    LocalCandidate,
    RemoteCandidate,
    Certificate,
    Unknown(String),
}

impl RtcStatsType {
    fn from_str(value: &str) -> Self {
        match value {
            "codec" => Self::Codec,
            "inbound-rtp" => Self::InboundRtp,
            "outbound-rtp" => Self::OutboundRtp,
            "remote-inbound-rtp" => Self::RemoteInboundRtp,
            "remote-outbound-rtp" => Self::RemoteOutboundRtp,
            "media-source" => Self::MediaSource,
            "media-playout" => Self::MediaPlayout,
            "peer-connection" => Self::PeerConnection,
            "data-channel" => Self::DataChannel,
            "transport" => Self::Transport,
            "candidate-pair" => Self::CandidatePair,
            "local-candidate" => Self::LocalCandidate,
            "remote-candidate" => Self::RemoteCandidate,
            "certificate" => Self::Certificate,
            _ => Self::Unknown(value.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Codec => "codec",
            Self::InboundRtp => "inbound-rtp",
            Self::OutboundRtp => "outbound-rtp",
            Self::RemoteInboundRtp => "remote-inbound-rtp",
            Self::RemoteOutboundRtp => "remote-outbound-rtp",
            Self::MediaSource => "media-source",
            Self::MediaPlayout => "media-playout",
            Self::PeerConnection => "peer-connection",
            Self::DataChannel => "data-channel",
            Self::Transport => "transport",
            Self::CandidatePair => "candidate-pair",
            Self::LocalCandidate => "local-candidate",
            Self::RemoteCandidate => "remote-candidate",
            Self::Certificate => "certificate",
            Self::Unknown(value) => value,
        }
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcStatsType {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        let value: String = value.try_into()?;
        Ok(Self::from_str(&value))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcStats {
    pub timestamp: f64,
    pub stats_type: RtcStatsType,
    pub id: String,
}

impl RtcStatsTrait for RtcStats {
    fn timestamp(&self) -> f64 {
        self.timestamp
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats_type.clone()
    }

    fn id(&self) -> String {
        self.id.clone()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        let timestamp = required_f64(value, "timestamp")?;
        let stats_type = RtcStatsType::try_from(value.to_member("type")?.required()?)?;
        let id = required_string(value, "id")?;
        Ok(Self {
            timestamp,
            stats_type,
            id,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcRtpStreamStats {
    pub stats: RtcStats,
    pub ssrc: u64,
    pub kind: String,
    pub transport_id: Option<String>,
    pub codec_id: Option<String>,
}

impl RtcStatsTrait for RtcRtpStreamStats {
    fn timestamp(&self) -> f64 {
        self.stats.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats.stats_type()
    }

    fn id(&self) -> String {
        self.stats.id()
    }
}

impl RtcRtpStreamStatsTrait for RtcRtpStreamStats {
    fn ssrc(&self) -> u64 {
        self.ssrc
    }

    fn kind(&self) -> String {
        self.kind.clone()
    }

    fn transport_id(&self) -> Option<String> {
        self.transport_id.clone()
    }

    fn codec_id(&self) -> Option<String> {
        self.codec_id.clone()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcRtpStreamStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        let stats = value.try_into()?;
        let ssrc = required_u64(value, "ssrc")?;
        let kind = required_string(value, "kind")?;
        let transport_id = optional_string(value, "transportId")?;
        let codec_id = optional_string(value, "codecId")?;
        Ok(Self {
            stats,
            ssrc,
            kind,
            transport_id,
            codec_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcReceivedRtpStreamStats {
    pub rtp_stream: RtcRtpStreamStats,
    pub packets_received: Option<u64>,
    pub packets_lost: Option<i64>,
    pub jitter: Option<f64>,
}

impl RtcStatsTrait for RtcReceivedRtpStreamStats {
    fn timestamp(&self) -> f64 {
        self.rtp_stream.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.rtp_stream.stats_type()
    }

    fn id(&self) -> String {
        self.rtp_stream.id()
    }
}

impl RtcRtpStreamStatsTrait for RtcReceivedRtpStreamStats {
    fn ssrc(&self) -> u64 {
        self.rtp_stream.ssrc()
    }

    fn kind(&self) -> String {
        self.rtp_stream.kind()
    }

    fn transport_id(&self) -> Option<String> {
        self.rtp_stream.transport_id()
    }

    fn codec_id(&self) -> Option<String> {
        self.rtp_stream.codec_id()
    }
}

impl RtcReceivedRtpStreamStatsTrait for RtcReceivedRtpStreamStats {
    fn packets_received(&self) -> Option<u64> {
        self.packets_received
    }

    fn packets_lost(&self) -> Option<i64> {
        self.packets_lost
    }

    fn jitter(&self) -> Option<f64> {
        self.jitter
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcReceivedRtpStreamStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            rtp_stream: value.try_into()?,
            packets_received: optional_u64(value, "packetsReceived")?,
            packets_lost: optional_i64(value, "packetsLost")?,
            jitter: optional_f64(value, "jitter")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcSentRtpStreamStats {
    pub rtp_stream: RtcRtpStreamStats,
    pub packets_sent: Option<u64>,
    pub bytes_sent: Option<u64>,
}

impl RtcStatsTrait for RtcSentRtpStreamStats {
    fn timestamp(&self) -> f64 {
        self.rtp_stream.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.rtp_stream.stats_type()
    }

    fn id(&self) -> String {
        self.rtp_stream.id()
    }
}

impl RtcRtpStreamStatsTrait for RtcSentRtpStreamStats {
    fn ssrc(&self) -> u64 {
        self.rtp_stream.ssrc()
    }

    fn kind(&self) -> String {
        self.rtp_stream.kind()
    }

    fn transport_id(&self) -> Option<String> {
        self.rtp_stream.transport_id()
    }

    fn codec_id(&self) -> Option<String> {
        self.rtp_stream.codec_id()
    }
}

impl RtcSentRtpStreamStatsTrait for RtcSentRtpStreamStats {
    fn packets_sent(&self) -> Option<u64> {
        self.packets_sent
    }

    fn bytes_sent(&self) -> Option<u64> {
        self.bytes_sent
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcSentRtpStreamStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            rtp_stream: value.try_into()?,
            packets_sent: optional_u64(value, "packetsSent")?,
            bytes_sent: optional_u64(value, "bytesSent")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcCodecStats {
    pub stats: RtcStats,
    pub payload_type: u64,
    pub transport_id: String,
    pub mime_type: String,
    pub clock_rate: Option<u64>,
    pub channels: Option<u64>,
    pub sdp_fmtp_line: Option<String>,
}

impl RtcStatsTrait for RtcCodecStats {
    fn timestamp(&self) -> f64 {
        self.stats.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats.stats_type()
    }

    fn id(&self) -> String {
        self.stats.id()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcCodecStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            stats: value.try_into()?,
            payload_type: required_u64(value, "payloadType")?,
            transport_id: required_string(value, "transportId")?,
            mime_type: required_string(value, "mimeType")?,
            clock_rate: optional_u64(value, "clockRate")?,
            channels: optional_u64(value, "channels")?,
            sdp_fmtp_line: optional_string(value, "sdpFmtpLine")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcInboundRtpStreamStats {
    pub received: RtcReceivedRtpStreamStats,
    pub track_identifier: String,
    pub mid: Option<String>,
    pub remote_id: Option<String>,
    pub bytes_received: Option<u64>,
    pub packets_discarded: Option<u64>,
    pub frames_received: Option<u64>,
    pub frames_decoded: Option<u64>,
    pub key_frames_decoded: Option<u64>,
    pub decoder_implementation: Option<String>,
}

impl RtcStatsTrait for RtcInboundRtpStreamStats {
    fn timestamp(&self) -> f64 {
        self.received.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.received.stats_type()
    }

    fn id(&self) -> String {
        self.received.id()
    }
}

impl RtcRtpStreamStatsTrait for RtcInboundRtpStreamStats {
    fn ssrc(&self) -> u64 {
        self.received.ssrc()
    }

    fn kind(&self) -> String {
        self.received.kind()
    }

    fn transport_id(&self) -> Option<String> {
        self.received.transport_id()
    }

    fn codec_id(&self) -> Option<String> {
        self.received.codec_id()
    }
}

impl RtcReceivedRtpStreamStatsTrait for RtcInboundRtpStreamStats {
    fn packets_received(&self) -> Option<u64> {
        self.received.packets_received()
    }

    fn packets_lost(&self) -> Option<i64> {
        self.received.packets_lost()
    }

    fn jitter(&self) -> Option<f64> {
        self.received.jitter()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcInboundRtpStreamStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            received: value.try_into()?,
            track_identifier: required_string(value, "trackIdentifier")?,
            mid: optional_string(value, "mid")?,
            remote_id: optional_string(value, "remoteId")?,
            bytes_received: optional_u64(value, "bytesReceived")?,
            packets_discarded: optional_u64(value, "packetsDiscarded")?,
            frames_received: optional_u64(value, "framesReceived")?,
            frames_decoded: optional_u64(value, "framesDecoded")?,
            key_frames_decoded: optional_u64(value, "keyFramesDecoded")?,
            decoder_implementation: optional_string(value, "decoderImplementation")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcOutboundRtpStreamStats {
    pub sent: RtcSentRtpStreamStats,
    pub mid: Option<String>,
    pub media_source_id: Option<String>,
    pub remote_id: Option<String>,
    pub rid: Option<String>,
    pub header_bytes_sent: Option<u64>,
    pub retransmitted_packets_sent: Option<u64>,
    pub retransmitted_bytes_sent: Option<u64>,
    pub encoder_implementation: Option<String>,
    pub active: Option<bool>,
    pub scalability_mode: Option<String>,
}

impl RtcStatsTrait for RtcOutboundRtpStreamStats {
    fn timestamp(&self) -> f64 {
        self.sent.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.sent.stats_type()
    }

    fn id(&self) -> String {
        self.sent.id()
    }
}

impl RtcRtpStreamStatsTrait for RtcOutboundRtpStreamStats {
    fn ssrc(&self) -> u64 {
        self.sent.ssrc()
    }

    fn kind(&self) -> String {
        self.sent.kind()
    }

    fn transport_id(&self) -> Option<String> {
        self.sent.transport_id()
    }

    fn codec_id(&self) -> Option<String> {
        self.sent.codec_id()
    }
}

impl RtcSentRtpStreamStatsTrait for RtcOutboundRtpStreamStats {
    fn packets_sent(&self) -> Option<u64> {
        self.sent.packets_sent()
    }

    fn bytes_sent(&self) -> Option<u64> {
        self.sent.bytes_sent()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcOutboundRtpStreamStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            sent: value.try_into()?,
            mid: optional_string(value, "mid")?,
            media_source_id: optional_string(value, "mediaSourceId")?,
            remote_id: optional_string(value, "remoteId")?,
            rid: optional_string(value, "rid")?,
            header_bytes_sent: optional_u64(value, "headerBytesSent")?,
            retransmitted_packets_sent: optional_u64(value, "retransmittedPacketsSent")?,
            retransmitted_bytes_sent: optional_u64(value, "retransmittedBytesSent")?,
            encoder_implementation: optional_string(value, "encoderImplementation")?,
            active: optional_bool(value, "active")?,
            scalability_mode: optional_string(value, "scalabilityMode")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcRemoteInboundRtpStreamStats {
    pub received: RtcReceivedRtpStreamStats,
    pub local_id: Option<String>,
    pub round_trip_time: Option<f64>,
    pub total_round_trip_time: Option<f64>,
    pub fraction_lost: Option<f64>,
    pub round_trip_time_measurements: Option<u64>,
}

impl RtcStatsTrait for RtcRemoteInboundRtpStreamStats {
    fn timestamp(&self) -> f64 {
        self.received.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.received.stats_type()
    }

    fn id(&self) -> String {
        self.received.id()
    }
}

impl RtcRtpStreamStatsTrait for RtcRemoteInboundRtpStreamStats {
    fn ssrc(&self) -> u64 {
        self.received.ssrc()
    }

    fn kind(&self) -> String {
        self.received.kind()
    }

    fn transport_id(&self) -> Option<String> {
        self.received.transport_id()
    }

    fn codec_id(&self) -> Option<String> {
        self.received.codec_id()
    }
}

impl RtcReceivedRtpStreamStatsTrait for RtcRemoteInboundRtpStreamStats {
    fn packets_received(&self) -> Option<u64> {
        self.received.packets_received()
    }

    fn packets_lost(&self) -> Option<i64> {
        self.received.packets_lost()
    }

    fn jitter(&self) -> Option<f64> {
        self.received.jitter()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcRemoteInboundRtpStreamStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            received: value.try_into()?,
            local_id: optional_string(value, "localId")?,
            round_trip_time: optional_f64(value, "roundTripTime")?,
            total_round_trip_time: optional_f64(value, "totalRoundTripTime")?,
            fraction_lost: optional_f64(value, "fractionLost")?,
            round_trip_time_measurements: optional_u64(value, "roundTripTimeMeasurements")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcRemoteOutboundRtpStreamStats {
    pub sent: RtcSentRtpStreamStats,
    pub local_id: Option<String>,
    pub remote_timestamp: Option<f64>,
    pub reports_sent: Option<u64>,
    pub round_trip_time: Option<f64>,
    pub total_round_trip_time: Option<f64>,
    pub round_trip_time_measurements: Option<u64>,
}

impl RtcStatsTrait for RtcRemoteOutboundRtpStreamStats {
    fn timestamp(&self) -> f64 {
        self.sent.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.sent.stats_type()
    }

    fn id(&self) -> String {
        self.sent.id()
    }
}

impl RtcRtpStreamStatsTrait for RtcRemoteOutboundRtpStreamStats {
    fn ssrc(&self) -> u64 {
        self.sent.ssrc()
    }

    fn kind(&self) -> String {
        self.sent.kind()
    }

    fn transport_id(&self) -> Option<String> {
        self.sent.transport_id()
    }

    fn codec_id(&self) -> Option<String> {
        self.sent.codec_id()
    }
}

impl RtcSentRtpStreamStatsTrait for RtcRemoteOutboundRtpStreamStats {
    fn packets_sent(&self) -> Option<u64> {
        self.sent.packets_sent()
    }

    fn bytes_sent(&self) -> Option<u64> {
        self.sent.bytes_sent()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcRemoteOutboundRtpStreamStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            sent: value.try_into()?,
            local_id: optional_string(value, "localId")?,
            remote_timestamp: optional_f64(value, "remoteTimestamp")?,
            reports_sent: optional_u64(value, "reportsSent")?,
            round_trip_time: optional_f64(value, "roundTripTime")?,
            total_round_trip_time: optional_f64(value, "totalRoundTripTime")?,
            round_trip_time_measurements: optional_u64(value, "roundTripTimeMeasurements")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcPeerConnectionStats {
    pub stats: RtcStats,
    pub data_channels_opened: Option<u64>,
    pub data_channels_closed: Option<u64>,
}

impl RtcStatsTrait for RtcPeerConnectionStats {
    fn timestamp(&self) -> f64 {
        self.stats.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats.stats_type()
    }

    fn id(&self) -> String {
        self.stats.id()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcPeerConnectionStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            stats: value.try_into()?,
            data_channels_opened: optional_u64(value, "dataChannelsOpened")?,
            data_channels_closed: optional_u64(value, "dataChannelsClosed")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcDataChannelStats {
    pub stats: RtcStats,
    pub label: Option<String>,
    pub protocol: Option<String>,
    pub data_channel_identifier: Option<u64>,
    pub state: String,
    pub messages_sent: Option<u64>,
    pub bytes_sent: Option<u64>,
    pub messages_received: Option<u64>,
    pub bytes_received: Option<u64>,
}

impl RtcStatsTrait for RtcDataChannelStats {
    fn timestamp(&self) -> f64 {
        self.stats.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats.stats_type()
    }

    fn id(&self) -> String {
        self.stats.id()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcDataChannelStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            stats: value.try_into()?,
            label: optional_string(value, "label")?,
            protocol: optional_string(value, "protocol")?,
            data_channel_identifier: optional_u64(value, "dataChannelIdentifier")?,
            state: required_string(value, "state")?,
            messages_sent: optional_u64(value, "messagesSent")?,
            bytes_sent: optional_u64(value, "bytesSent")?,
            messages_received: optional_u64(value, "messagesReceived")?,
            bytes_received: optional_u64(value, "bytesReceived")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcTransportStats {
    pub stats: RtcStats,
    pub packets_sent: Option<u64>,
    pub packets_received: Option<u64>,
    pub bytes_sent: Option<u64>,
    pub bytes_received: Option<u64>,
    pub ice_role: Option<String>,
    pub ice_local_username_fragment: Option<String>,
    pub dtls_state: String,
    pub ice_state: Option<String>,
    pub selected_candidate_pair_id: Option<String>,
    pub local_certificate_id: Option<String>,
    pub remote_certificate_id: Option<String>,
}

impl RtcStatsTrait for RtcTransportStats {
    fn timestamp(&self) -> f64 {
        self.stats.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats.stats_type()
    }

    fn id(&self) -> String {
        self.stats.id()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcTransportStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            stats: value.try_into()?,
            packets_sent: optional_u64(value, "packetsSent")?,
            packets_received: optional_u64(value, "packetsReceived")?,
            bytes_sent: optional_u64(value, "bytesSent")?,
            bytes_received: optional_u64(value, "bytesReceived")?,
            ice_role: optional_string(value, "iceRole")?,
            ice_local_username_fragment: optional_string(value, "iceLocalUsernameFragment")?,
            dtls_state: required_string(value, "dtlsState")?,
            ice_state: optional_string(value, "iceState")?,
            selected_candidate_pair_id: optional_string(value, "selectedCandidatePairId")?,
            local_certificate_id: optional_string(value, "localCertificateId")?,
            remote_certificate_id: optional_string(value, "remoteCertificateId")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcIceCandidatePairStats {
    pub stats: RtcStats,
    pub transport_id: String,
    pub local_candidate_id: String,
    pub remote_candidate_id: String,
    pub state: String,
    pub nominated: Option<bool>,
    pub packets_sent: Option<u64>,
    pub packets_received: Option<u64>,
    pub bytes_sent: Option<u64>,
    pub bytes_received: Option<u64>,
    pub current_round_trip_time: Option<f64>,
    pub total_round_trip_time: Option<f64>,
    pub available_outgoing_bitrate: Option<f64>,
    pub available_incoming_bitrate: Option<f64>,
}

impl RtcStatsTrait for RtcIceCandidatePairStats {
    fn timestamp(&self) -> f64 {
        self.stats.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats.stats_type()
    }

    fn id(&self) -> String {
        self.stats.id()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcIceCandidatePairStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            stats: value.try_into()?,
            transport_id: required_string(value, "transportId")?,
            local_candidate_id: required_string(value, "localCandidateId")?,
            remote_candidate_id: required_string(value, "remoteCandidateId")?,
            state: required_string(value, "state")?,
            nominated: optional_bool(value, "nominated")?,
            packets_sent: optional_u64(value, "packetsSent")?,
            packets_received: optional_u64(value, "packetsReceived")?,
            bytes_sent: optional_u64(value, "bytesSent")?,
            bytes_received: optional_u64(value, "bytesReceived")?,
            current_round_trip_time: optional_f64(value, "currentRoundTripTime")?,
            total_round_trip_time: optional_f64(value, "totalRoundTripTime")?,
            available_outgoing_bitrate: optional_f64(value, "availableOutgoingBitrate")?,
            available_incoming_bitrate: optional_f64(value, "availableIncomingBitrate")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcIceCandidateStats {
    pub stats: RtcStats,
    pub transport_id: String,
    pub address: Option<String>,
    pub port: Option<i64>,
    pub protocol: Option<String>,
    pub candidate_type: String,
    pub priority: Option<i64>,
    pub url: Option<String>,
    pub relay_protocol: Option<String>,
    pub foundation: Option<String>,
    pub related_address: Option<String>,
    pub related_port: Option<i64>,
    pub username_fragment: Option<String>,
    pub tcp_type: Option<String>,
}

impl RtcStatsTrait for RtcIceCandidateStats {
    fn timestamp(&self) -> f64 {
        self.stats.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats.stats_type()
    }

    fn id(&self) -> String {
        self.stats.id()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcIceCandidateStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            stats: value.try_into()?,
            transport_id: required_string(value, "transportId")?,
            address: optional_string(value, "address")?,
            port: optional_i64(value, "port")?,
            protocol: optional_string(value, "protocol")?,
            candidate_type: required_string(value, "candidateType")?,
            priority: optional_i64(value, "priority")?,
            url: optional_string(value, "url")?,
            relay_protocol: optional_string(value, "relayProtocol")?,
            foundation: optional_string(value, "foundation")?,
            related_address: optional_string(value, "relatedAddress")?,
            related_port: optional_i64(value, "relatedPort")?,
            username_fragment: optional_string(value, "usernameFragment")?,
            tcp_type: optional_string(value, "tcpType")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtcCertificateStats {
    pub stats: RtcStats,
    pub fingerprint: String,
    pub fingerprint_algorithm: String,
    pub base64_certificate: String,
    pub issuer_certificate_id: Option<String>,
}

impl RtcStatsTrait for RtcCertificateStats {
    fn timestamp(&self) -> f64 {
        self.stats.timestamp()
    }

    fn stats_type(&self) -> RtcStatsType {
        self.stats.stats_type()
    }

    fn id(&self) -> String {
        self.stats.id()
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for RtcCertificateStats {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        Ok(Self {
            stats: value.try_into()?,
            fingerprint: required_string(value, "fingerprint")?,
            fingerprint_algorithm: required_string(value, "fingerprintAlgorithm")?,
            base64_certificate: required_string(value, "base64Certificate")?,
            issuer_certificate_id: optional_string(value, "issuerCertificateId")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebRtcStat {
    Codec(RtcCodecStats),
    InboundRtp(RtcInboundRtpStreamStats),
    OutboundRtp(RtcOutboundRtpStreamStats),
    RemoteInboundRtp(RtcRemoteInboundRtpStreamStats),
    RemoteOutboundRtp(RtcRemoteOutboundRtpStreamStats),
    PeerConnection(RtcPeerConnectionStats),
    DataChannel(RtcDataChannelStats),
    Transport(RtcTransportStats),
    CandidatePair(RtcIceCandidatePairStats),
    LocalCandidate(RtcIceCandidateStats),
    RemoteCandidate(RtcIceCandidateStats),
    Certificate(RtcCertificateStats),
    Other(RtcStats),
}

impl RtcStatsTrait for WebRtcStat {
    fn timestamp(&self) -> f64 {
        match self {
            Self::Codec(v) => v.timestamp(),
            Self::InboundRtp(v) => v.timestamp(),
            Self::OutboundRtp(v) => v.timestamp(),
            Self::RemoteInboundRtp(v) => v.timestamp(),
            Self::RemoteOutboundRtp(v) => v.timestamp(),
            Self::PeerConnection(v) => v.timestamp(),
            Self::DataChannel(v) => v.timestamp(),
            Self::Transport(v) => v.timestamp(),
            Self::CandidatePair(v) => v.timestamp(),
            Self::LocalCandidate(v) => v.timestamp(),
            Self::RemoteCandidate(v) => v.timestamp(),
            Self::Certificate(v) => v.timestamp(),
            Self::Other(v) => v.timestamp(),
        }
    }

    fn stats_type(&self) -> RtcStatsType {
        match self {
            Self::Codec(v) => v.stats_type(),
            Self::InboundRtp(v) => v.stats_type(),
            Self::OutboundRtp(v) => v.stats_type(),
            Self::RemoteInboundRtp(v) => v.stats_type(),
            Self::RemoteOutboundRtp(v) => v.stats_type(),
            Self::PeerConnection(v) => v.stats_type(),
            Self::DataChannel(v) => v.stats_type(),
            Self::Transport(v) => v.stats_type(),
            Self::CandidatePair(v) => v.stats_type(),
            Self::LocalCandidate(v) => v.stats_type(),
            Self::RemoteCandidate(v) => v.stats_type(),
            Self::Certificate(v) => v.stats_type(),
            Self::Other(v) => v.stats_type(),
        }
    }

    fn id(&self) -> String {
        match self {
            Self::Codec(v) => v.id(),
            Self::InboundRtp(v) => v.id(),
            Self::OutboundRtp(v) => v.id(),
            Self::RemoteInboundRtp(v) => v.id(),
            Self::RemoteOutboundRtp(v) => v.id(),
            Self::PeerConnection(v) => v.id(),
            Self::DataChannel(v) => v.id(),
            Self::Transport(v) => v.id(),
            Self::CandidatePair(v) => v.id(),
            Self::LocalCandidate(v) => v.id(),
            Self::RemoteCandidate(v) => v.id(),
            Self::Certificate(v) => v.id(),
            Self::Other(v) => v.id(),
        }
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for WebRtcStat {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        let base = RtcStats::try_from(value)?;
        match base.stats_type {
            RtcStatsType::Codec => Ok(Self::Codec(value.try_into()?)),
            RtcStatsType::InboundRtp => Ok(Self::InboundRtp(value.try_into()?)),
            RtcStatsType::OutboundRtp => Ok(Self::OutboundRtp(value.try_into()?)),
            RtcStatsType::RemoteInboundRtp => Ok(Self::RemoteInboundRtp(value.try_into()?)),
            RtcStatsType::RemoteOutboundRtp => Ok(Self::RemoteOutboundRtp(value.try_into()?)),
            RtcStatsType::PeerConnection => Ok(Self::PeerConnection(value.try_into()?)),
            RtcStatsType::DataChannel => Ok(Self::DataChannel(value.try_into()?)),
            RtcStatsType::Transport => Ok(Self::Transport(value.try_into()?)),
            RtcStatsType::CandidatePair => Ok(Self::CandidatePair(value.try_into()?)),
            RtcStatsType::LocalCandidate => Ok(Self::LocalCandidate(value.try_into()?)),
            RtcStatsType::RemoteCandidate => Ok(Self::RemoteCandidate(value.try_into()?)),
            RtcStatsType::Certificate => Ok(Self::Certificate(value.try_into()?)),
            _ => Ok(Self::Other(base)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebRtcStatsReport {
    pub stats: Vec<WebRtcStat>,
}

impl WebRtcStatsReport {
    pub fn parse(stats_json: &JsonString) -> JsonResult<Self> {
        let json_str = stats_json.to_string();
        let json = RawJson::parse(&json_str)?;
        let array = json.value().to_array()?;
        let mut stats = Vec::new();
        for item in array {
            stats.push(item.try_into()?);
        }
        Ok(Self { stats })
    }
}

fn required_string(value: RawJsonValue<'_, '_>, member_name: &str) -> JsonResult<String> {
    value.to_member(member_name)?.required()?.try_into()
}

fn required_u64(value: RawJsonValue<'_, '_>, member_name: &str) -> JsonResult<u64> {
    value.to_member(member_name)?.required()?.try_into()
}

fn required_f64(value: RawJsonValue<'_, '_>, member_name: &str) -> JsonResult<f64> {
    value.to_member(member_name)?.required()?.try_into()
}

fn optional_string(value: RawJsonValue<'_, '_>, member_name: &str) -> JsonResult<Option<String>> {
    value
        .to_member(member_name)?
        .optional()
        .map(|v| v.try_into())
        .transpose()
}

fn optional_u64(value: RawJsonValue<'_, '_>, member_name: &str) -> JsonResult<Option<u64>> {
    value
        .to_member(member_name)?
        .optional()
        .map(|v| v.try_into())
        .transpose()
}

fn optional_i64(value: RawJsonValue<'_, '_>, member_name: &str) -> JsonResult<Option<i64>> {
    value
        .to_member(member_name)?
        .optional()
        .map(|v| v.try_into())
        .transpose()
}

fn optional_f64(value: RawJsonValue<'_, '_>, member_name: &str) -> JsonResult<Option<f64>> {
    value
        .to_member(member_name)?
        .optional()
        .map(|v| v.try_into())
        .transpose()
}

fn optional_bool(value: RawJsonValue<'_, '_>, member_name: &str) -> JsonResult<Option<bool>> {
    value
        .to_member(member_name)?
        .optional()
        .map(|v| v.try_into())
        .transpose()
}
