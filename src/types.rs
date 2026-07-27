//! 公開型と接続設定用の型。
use nojson::{DisplayJson, JsonFormatter, RawJsonOwned};

use crate::error::{Error, Result};

/// シグナリング接続の方式を表す列挙型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalingType {
    /// WebSocket によるシグナリング。
    WebSocket,
    /// DataChannel によるシグナリング。
    DataChannel,
}

/// シグナリングメッセージの方向を表す列挙型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalingDirection {
    /// 送信方向。
    Sent,
    /// 受信方向。
    Received,
}

/// 接続ロールを表す列挙型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 送信のみ。
    SendOnly,
    /// 受信のみ。
    RecvOnly,
    /// 送受信可能。
    SendRecv,
}

impl Role {
    /// 文字列から [`Role`] をパースする。
    ///
    /// 有効な値は `"sendonly"`, `"recvonly"`, `"sendrecv"`。
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

    /// Sora に送信するロール文字列を返す。
    pub fn as_sora_role(self) -> &'static str {
        match self {
            Role::SendOnly => "sendonly",
            Role::RecvOnly => "recvonly",
            Role::SendRecv => "sendrecv",
        }
    }

    /// 送信を希望しているロールであれば `true` を返す。
    pub fn wants_send(self) -> bool {
        matches!(self, Role::SendOnly | Role::SendRecv)
    }

    /// 受信を希望しているロールであれば `true` を返す。
    pub fn wants_recv(self) -> bool {
        matches!(self, Role::RecvOnly | Role::SendRecv)
    }
}

/// プロキシの設定情報。
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ProxyInfo {
    /// プロキシサーバーの URL。
    pub url: String,
    /// プロキシ認証のユーザー名。
    pub username: Option<String>,
    /// プロキシ認証のパスワード。
    pub password: Option<String>,
    /// プロキシ接続時に使用する User-Agent ヘッダー。
    pub user_agent: Option<String>,
}

impl std::fmt::Debug for ProxyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let masked_url = mask_url_userinfo(&self.url);
        f.debug_struct("ProxyInfo")
            .field("url", &masked_url)
            .field("username", &self.username.as_ref().map(|_| "<redacted>"))
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("user_agent", &self.user_agent)
            .finish()
    }
}

fn mask_url_userinfo(url: &str) -> std::borrow::Cow<'_, str> {
    let Some(after_scheme) = url.find("://") else {
        return std::borrow::Cow::Borrowed(url);
    };
    let after_scheme = after_scheme + 3;
    let url_after_scheme = &url[after_scheme..];
    let Some(at_pos) = url_after_scheme.find('@') else {
        return std::borrow::Cow::Borrowed(url);
    };
    if let Some(slash_pos) = url_after_scheme.find('/')
        && at_pos > slash_pos
    {
        return std::borrow::Cow::Borrowed(url);
    }
    let masked = format!(
        "{}<redacted>@{}",
        &url[..after_scheme],
        &url[after_scheme + at_pos + 1..]
    );
    std::borrow::Cow::Owned(masked)
}

/// JSON 文字列を検証済みの形で保持する型。
///
/// `FromStr` により文字列からパース可能。
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

/// Opus 音声コーデックのパラメータ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioOpusParams {
    /// チャンネル数。
    pub channels: Option<u32>,
    /// 最大再生レート (Hz)。
    pub maxplaybackrate: Option<u32>,
    /// 最小パケット時間 (ms)。
    pub minptime: Option<u32>,
    /// パケット時間 (ms)。
    pub ptime: Option<u32>,
    /// ステレオ再生の設定。
    pub stereo: Option<bool>,
    /// SDP の sprop-stereo パラメータ。
    pub sprop_stereo: Option<bool>,
    /// 帯域内前方誤り訂正 (in-band FEC) を使用するか。
    pub useinbandfec: Option<bool>,
    /// DTX (Discontinuous Transmission) を使用するか。
    pub usedtx: Option<bool>,
}

impl DisplayJson for AudioOpusParams {
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

/// 映像コーデックの種類を表す列挙型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodecType {
    /// VP8 コーデック。
    Vp8,
    /// VP9 コーデック。
    Vp9,
    /// H.264 コーデック。
    H264,
    /// H.265 コーデック。
    H265,
    /// AV1 コーデック。
    Av1,
}

impl DisplayJson for VideoCodecType {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            VideoCodecType::Vp8 => f.value("VP8"),
            VideoCodecType::Vp9 => f.value("VP9"),
            VideoCodecType::H264 => f.value("H264"),
            VideoCodecType::H265 => f.value("H265"),
            VideoCodecType::Av1 => f.value("AV1"),
        }
    }
}

impl From<VideoCodecType> for shiguredo_webrtc::VideoCodecType {
    fn from(value: VideoCodecType) -> Self {
        match value {
            VideoCodecType::Vp8 => shiguredo_webrtc::VideoCodecType::Vp8,
            VideoCodecType::Vp9 => shiguredo_webrtc::VideoCodecType::Vp9,
            VideoCodecType::H264 => shiguredo_webrtc::VideoCodecType::H264,
            VideoCodecType::H265 => shiguredo_webrtc::VideoCodecType::H265,
            VideoCodecType::Av1 => shiguredo_webrtc::VideoCodecType::Av1,
        }
    }
}

impl TryFrom<shiguredo_webrtc::VideoCodecType> for VideoCodecType {
    type Error = String;

    fn try_from(value: shiguredo_webrtc::VideoCodecType) -> std::result::Result<Self, Self::Error> {
        match value {
            shiguredo_webrtc::VideoCodecType::Vp8 => Ok(VideoCodecType::Vp8),
            shiguredo_webrtc::VideoCodecType::Vp9 => Ok(VideoCodecType::Vp9),
            shiguredo_webrtc::VideoCodecType::H264 => Ok(VideoCodecType::H264),
            shiguredo_webrtc::VideoCodecType::H265 => Ok(VideoCodecType::H265),
            shiguredo_webrtc::VideoCodecType::Av1 => Ok(VideoCodecType::Av1),
            other => Err(format!(
                "変換できません: shiguredo_webrtc::VideoCodecType::{other:?} → sora_sdk::VideoCodecType"
            )),
        }
    }
}

/// 音声コーデックの種類を表す列挙型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodecType {
    /// Opus コーデック。
    Opus,
}

impl DisplayJson for AudioCodecType {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            AudioCodecType::Opus => f.value("OPUS"),
        }
    }
}

/// connect メッセージの `audio` フィールドに対応する音声設定。
///
/// [Audio::Bool] で単純な有効/無効を指定するか、
/// コーデック別のバリアントで詳細設定を指定できる。
/// role が [Role::SendRecv] または [Role::SendOnly] の場合は配信設定、
/// [Role::RecvOnly] の場合は受信設定として扱われる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audio {
    /// 音声の有効 (`true`) / 無効 (`false`)。
    ///
    /// `true` の場合は Opus で音声が配信または受信される。
    Bool(bool),
    /// Opus コーデックの詳細設定。
    Opus {
        /// ビットレート (kbps)。6〜510 の範囲。
        bit_rate: Option<u32>,
        /// Opus 固有のパラメータ。
        params: Option<AudioOpusParams>,
    },
}

impl Audio {
    /// Bool バリアントの [Audio] を生成する。
    pub fn new_bool(enabled: bool) -> Self {
        Self::Bool(enabled)
    }
    /// Opus バリアントの [Audio] を生成する。
    pub fn new_opus(bit_rate: Option<u32>, params: Option<AudioOpusParams>) -> Self {
        Self::Opus { bit_rate, params }
    }
}

impl DisplayJson for Audio {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Audio::Bool(b) => f.value(*b),
            Audio::Opus { bit_rate, params } => f.object(|f| {
                f.member("codec_type", AudioCodecType::Opus)?;
                if let Some(bit_rate) = bit_rate {
                    f.member("bit_rate", bit_rate)?;
                }
                if let Some(params) = params {
                    f.member("opus_params", params)?;
                }
                Ok(())
            }),
        }
    }
}

/// VP9 映像コーデックのパラメータ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoVP9Params {
    /// プロファイル ID (0..3)。
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

/// H.264 映像コーデックのパラメータ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoH264Params {
    /// プロファイルレベル ID (例: `"42e01f"`)。
    pub profile_level_id: Option<String>,
    /// B フレームの有効/無効。
    /// sora.conf で h264_b_frame = true を設定する必要があります
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

/// H.265 映像コーデックのパラメータ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoH265Params {
    /// レベル ID (例: `"120"`)。
    pub level_id: Option<String>,
    /// プロファイル ID (0..31)。
    pub profile_id: Option<u32>,
    /// ティアフラグ (0..1)。
    pub tier_flag: Option<u32>,
    /// 送信モード (`"SRST"`, `"MRST"`, `"MRMT"` のいずれか)。
    pub tx_mode: Option<String>,
    /// B フレームの有効/無効。
    /// sora.conf で h265_b_frame = true を設定する必要があります
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

/// AV1 映像コーデックのパラメータ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoAV1Params {
    /// プロファイル (0..2)。
    pub profile: Option<u32>,
    /// レベルインデックス (0..31)。
    pub level_idx: Option<u32>,
    /// ティア (0..1)。
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

/// リアルタイムメッセージング用の DataChannel 設定。
///
/// Sora サーバーとの接続時に `"type": "connect"` メッセージの
/// `data_channels` フィールドに含めて送信される。
/// ここで指定するラベルは必ず `#` で始める必要があり、
/// Sora が内部的に使用するシグナリング用ラベル
/// （`signaling` / `stats` / `push` / `notify` / `rpc`）は指定できない。
#[derive(Debug, Clone)]
pub struct ConnectDataChannel {
    /// DataChannel のラベル。
    ///
    /// 必ず `#` で始まる文字列を指定する。最大 32 文字（`#` を含む）。
    pub label: String,
    /// メッセージの方向。
    ///
    /// クライアントから見た方向を指定する。
    /// `"sendrecv"` で送受信、`"sendonly"` で送信のみ、
    /// `"recvonly"` で受信のみ。
    pub direction: String,
    /// 順序保証を行うかどうか。
    ///
    /// `true` の場合、SCTP がメッセージの送信順での配信を保証する。
    /// `false` の場合、到着が送信順から前後する可能性があるが、
    /// 順序保証に伴う HoL (Head-of-Line) ブロッキングが発生しない。
    pub ordered: Option<bool>,
    /// 最大再送時間（ミリ秒）。
    ///
    /// `max_retransmits` とは同時に指定できない。
    pub max_packet_life_time: Option<i32>,
    /// 最大再送回数。
    ///
    /// `max_packet_life_time` とは同時に指定できない。
    pub max_retransmits: Option<i32>,
    /// DataChannel のサブプロトコル。
    ///
    /// アプリケーション層のプロトコルを識別する文字列。
    pub protocol: Option<String>,
    /// メッセージを zlib で圧縮するかどうか。
    ///
    /// `true` の場合、送受信時に SDK 内部で
    /// compress_zlib / decompress_zlib による圧縮・展開が行われる。
    pub compress: Option<bool>,
    /// Sora サーバーがメッセージに付与するヘッダー。
    ///
    /// `{"type": "sender_connection_id"}` を指定すると、
    /// 受信メッセージの先頭に送信元の接続 ID（26 バイト）が付与される。
    /// direction が `"sendrecv"` または `"recvonly"` の場合に有効。
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

/// 転送フィルターのルール。
///
/// [ForwardingFilter] の `rules` フィールドで使われる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardingFilterRule {
    /// フィルタ対象のフィールド名。
    ///
    /// `"connection_id"` / `"client_id"` / `"kind"` のいずれかを指定する。
    pub field: String,
    /// 比較演算子。`"is_in"` または `"is_not_in"` を指定する。
    pub operator: String,
    /// 比較対象の値のリスト。
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

/// 転送フィルターの設定。
///
/// 接続時に Sora サーバーに送信し、条件に合致する
/// 他参加者の音声や映像の受信をブロックするために使う。
#[derive(Debug, Clone)]
pub struct ForwardingFilter {
    /// API での識別に使われるフィルターの名前。
    pub name: Option<String>,
    /// フィルターの優先度。
    pub priority: Option<i32>,
    /// 条件に合致した場合の動作。`"block"` または `"allow"`。
    pub action: Option<String>,
    /// 転送フィルターのルール。
    ///
    /// 内側の [Vec]\([ForwardingFilterRule]\) は AND 結合され、
    /// 外側の [Vec] は OR 結合される。
    pub rules: Vec<Vec<ForwardingFilterRule>>,
    /// API のバージョン文字列。
    pub version: Option<String>,
    /// 任意のメタデータ（JSON）。
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

/// connect メッセージの `video` フィールドに対応する映像設定。
///
/// [Video::Bool] で単純な有効/無効を指定するか、
/// コーデック別のバリアントで詳細設定を指定できる。
/// role が [Role::SendRecv] または [Role::SendOnly] の場合は配信設定、
/// [Role::RecvOnly] の場合は受信設定として扱われる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Video {
    /// 映像の有効 (`true`) / 無効 (`false`)。
    ///
    /// `true` の場合はデフォルトで映像が配信または受信される。
    Bool(bool),
    /// VP8 コーデックの詳細設定。
    Vp8 {
        /// ビットレート (kbps)。1〜50000 の範囲。
        bit_rate: Option<u32>,
    },
    /// VP9 コーデックの詳細設定。
    Vp9 {
        /// ビットレート (kbps)。1〜50000 の範囲。
        bit_rate: Option<u32>,
        /// VP9 固有のパラメータ。
        params: Option<VideoVP9Params>,
    },
    /// H.264 コーデックの詳細設定。
    H264 {
        /// ビットレート (kbps)。1〜50000 の範囲。
        bit_rate: Option<u32>,
        /// H.264 固有のパラメータ。
        params: Option<VideoH264Params>,
    },
    /// H.265 コーデックの詳細設定。
    H265 {
        /// ビットレート (kbps)。1〜50000 の範囲。
        bit_rate: Option<u32>,
        /// H.265 固有のパラメータ。
        params: Option<VideoH265Params>,
    },
    /// AV1 コーデックの詳細設定。
    Av1 {
        /// ビットレート (kbps)。1〜50000 の範囲。
        bit_rate: Option<u32>,
        /// AV1 固有のパラメータ。
        params: Option<VideoAV1Params>,
    },
}

impl Video {
    /// Bool バリアントの [Video] を生成する。
    pub fn new_bool(enabled: bool) -> Self {
        Self::Bool(enabled)
    }
    /// H.264 バリアントの [Video] を生成する。
    pub fn new_vp8(bit_rate: Option<u32>) -> Self {
        Self::Vp8 { bit_rate }
    }
    /// VP9 バリアントの [Video] を生成する。
    pub fn new_vp9(bit_rate: Option<u32>, params: Option<VideoVP9Params>) -> Self {
        Self::Vp9 { bit_rate, params }
    }
    /// AV1 バリアントの [Video] を生成する。
    pub fn new_av1(bit_rate: Option<u32>, params: Option<VideoAV1Params>) -> Self {
        Self::Av1 { bit_rate, params }
    }
    /// H.264 バリアントの [Video] を生成する。
    pub fn new_h264(bit_rate: Option<u32>, params: Option<VideoH264Params>) -> Self {
        Self::H264 { bit_rate, params }
    }
    /// H.265 バリアントの [Video] を生成する。
    pub fn new_h265(bit_rate: Option<u32>, params: Option<VideoH265Params>) -> Self {
        Self::H265 { bit_rate, params }
    }
}

impl DisplayJson for Video {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Video::Bool(b) => f.value(*b),
            Video::Vp8 { bit_rate } => f.object(|f| {
                f.member("codec_type", VideoCodecType::Vp8)?;
                if let Some(bit_rate) = bit_rate {
                    f.member("bit_rate", bit_rate)?;
                }
                Ok(())
            }),
            Video::Vp9 { bit_rate, params } => f.object(|f| {
                f.member("codec_type", VideoCodecType::Vp9)?;
                if let Some(bit_rate) = bit_rate {
                    f.member("bit_rate", bit_rate)?;
                }
                if let Some(params) = params {
                    f.member("vp9_params", params)?;
                }
                Ok(())
            }),
            Video::Av1 { bit_rate, params } => f.object(|f| {
                f.member("codec_type", VideoCodecType::Av1)?;
                if let Some(bit_rate) = bit_rate {
                    f.member("bit_rate", bit_rate)?;
                }
                if let Some(params) = params {
                    f.member("av1_params", params)?;
                }
                Ok(())
            }),
            Video::H264 { bit_rate, params } => f.object(|f| {
                f.member("codec_type", VideoCodecType::H264)?;
                if let Some(bit_rate) = bit_rate {
                    f.member("bit_rate", bit_rate)?;
                }
                if let Some(params) = params {
                    f.member("h264_params", params)?;
                }
                Ok(())
            }),
            Video::H265 { bit_rate, params } => f.object(|f| {
                f.member("codec_type", VideoCodecType::H265)?;
                if let Some(bit_rate) = bit_rate {
                    f.member("bit_rate", bit_rate)?;
                }
                if let Some(params) = params {
                    f.member("h265_params", params)?;
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
        let opus_params = AudioOpusParams {
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
