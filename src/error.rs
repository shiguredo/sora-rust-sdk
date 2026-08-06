//! エラー定義と Result 型。
use std::io;

use nojson::JsonParseError;
use shiguredo_http11::{EncodeError, auth::AuthError, uri::UriError};
use tokio::sync::oneshot;

use crate::video_codecs::mp4::Mp4Error;

/// SDK のエラー型。
#[derive(Debug)]
pub enum Error {
    /// `--role` に不正な値が指定された。
    InvalidRole {
        /// 指定された role 文字列。
        value: String,
    },
    /// ホストが空。
    HostEmpty,
    /// ホストの指定形式が不正。
    HostInvalidFormat,
    /// URL の解析に失敗した。内部エラーとして [`UriError`] を保持する。
    UriParse(UriError),
    /// URL にスキームがない。
    UrlMissingScheme,
    /// URL のスキームが未対応（`ws://` または `wss://` のみ対応）。
    UrlUnsupportedScheme {
        /// 指定されたスキーム文字列。
        scheme: String,
    },
    /// URL に userinfo が含まれている（未対応）。
    UrlUserinfoNotSupported,
    /// URL にフラグメントが含まれている（許可されない）。
    UrlFragmentNotAllowed,
    /// URL にホストがない。
    UrlMissingHost,
    /// プロキシ URL のスキームが未対応（`http://` のみ対応）。
    ProxyUrlUnsupportedScheme {
        /// 指定されたスキーム文字列。
        scheme: String,
    },
    /// プロキシ URL に userinfo が含まれている（未対応）。
    ProxyUrlUserinfoNotSupported,
    /// プロキシ URL にフラグメントが含まれている（許可されない）。
    ProxyUrlFragmentNotAllowed,
    /// プロキシ URL にホストがない。
    ProxyUrlMissingHost,
    /// プロキシ URL にパスが含まれている（`/` のみ許可）。
    ProxyUrlPathNotAllowed {
        /// 指定されたパス文字列。
        path: String,
    },
    /// プロキシ URL にクエリが含まれている（許可されない）。
    ProxyUrlQueryNotAllowed,
    /// プロキシ CONNECT レスポンスの解析に失敗した。内部エラーとして [`shiguredo_http11::Error`] を保持する。
    ProxyConnectDecode(shiguredo_http11::Error),
    /// プロキシ CONNECT リクエストの生成に失敗した。内部エラーとして [`EncodeError`] を保持する。
    ProxyConnectEncode(EncodeError),
    /// プロキシ CONNECT の応答を受信できなかった。
    ProxyConnectResponseMissing,
    /// プロキシ CONNECT が失敗ステータスを返した。
    ProxyConnectStatusNotSuccessful {
        /// 受信した HTTP ステータスコード。
        status_code: u16,
        /// 受信した HTTP reason phrase。
        reason_phrase: String,
    },
    /// プロキシ CONNECT 応答後に TLS 開始前の不正な余剰データを受信した。
    ProxyConnectUnexpectedTrailingData,
    /// プロキシ認証情報の生成に失敗した。内部エラーとして [`AuthError`] を保持する。
    ProxyAuth(AuthError),
    /// DNS 解決に失敗した。
    DnsResolve {
        /// 解決を試みたホスト名。
        host: String,
        /// 発生した IO エラー。
        source: io::Error,
    },
    /// 指定されたホストとポートに解決されたアドレスが見つからない。
    NoResolvedAddress {
        /// 解決を試みたホスト名。
        host: String,
        /// 解決を試みたポート番号。
        port: u16,
    },
    /// TCP 接続がタイムアウトした。
    TcpConnectTimeout {
        /// 接続を試みたホスト名。
        host: String,
        /// 接続を試みたポート番号。
        port: u16,
    },
    /// TCP 接続に失敗した。
    TcpConnect {
        /// 接続を試みたホスト名。
        host: String,
        /// 接続を試みたポート番号。
        port: u16,
        /// 発生した IO エラー。
        source: io::Error,
    },
    /// TLS 設定に失敗した。内部エラーとして [`rustls::Error`] を保持する。
    TlsConfig(rustls::Error),
    /// TLS の ServerName が不正。内部エラーとして [`rustls::pki_types::InvalidDnsNameError`] を保持する。
    InvalidServerName(rustls::pki_types::InvalidDnsNameError),
    /// TLS 接続がタイムアウトした。
    TlsConnectTimeout {
        /// 接続を試みたホスト名。
        host: String,
    },
    /// TLS 接続に失敗した。
    TlsConnect {
        /// 接続を試みたホスト名。
        host: String,
        /// 発生した IO エラー。
        source: io::Error,
    },
    /// WebSocket のエラー。内部エラーとして [`shiguredo_websocket::Error`] を保持する。
    Websocket(shiguredo_websocket::Error),
    /// IO エラー。内部エラーとして [`std::io::Error`] を保持する。
    Io(io::Error),
    /// JSON のパースに失敗した。内部エラーとして [`JsonParseError`] を保持する。
    JsonParse(JsonParseError),
    /// WebRTC のエラー。内部エラーとして [`shiguredo_webrtc::Error`] を保持する。
    Webrtc(shiguredo_webrtc::Error),
    /// [`PeerConnection`](shiguredo_webrtc::PeerConnection) が存在しない。
    PeerConnectionMissing,
    /// SetRemoteDescription がタイムアウトした。
    SetRemoteDescriptionTimeout,
    /// SetRemoteDescription の応答を受信できなかった。
    SetRemoteDescriptionResponseMissing,
    /// SetRemoteDescription が失敗した。
    SetRemoteDescriptionFailed {
        /// 失敗理由。
        reason: String,
    },
    /// Answer の生成がタイムアウトした。
    AnswerTimeout,
    /// Answer の応答を受信できなかった。
    AnswerResponseMissing,
    /// Answer の生成に失敗した。
    AnswerFailed {
        /// 失敗理由。
        reason: String,
    },
    /// SetLocalDescription がタイムアウトした。
    SetLocalDescriptionTimeout,
    /// SetLocalDescription の応答を受信できなかった。
    SetLocalDescriptionResponseMissing,
    /// SetLocalDescription が失敗した。
    SetLocalDescriptionFailed {
        /// 失敗理由。
        reason: String,
    },
    /// simulcast に対応する video sender が存在しない。
    SimulcastVideoSenderMissing,
    /// simulcast の SetParameters が失敗した。内部エラーとして [`shiguredo_webrtc::Error`] を保持する。
    SimulcastSetParametersFailed {
        /// 発生した WebRTC エラー。
        source: shiguredo_webrtc::Error,
    },
    /// 指定されたラベルの DataChannel が存在しない。
    DataChannelMissing {
        /// 見つからなかった DataChannel のラベル名。
        label: String,
    },
    /// DataChannel への送信に失敗した。
    DataChannelSendFailed,
    /// リダイレクトが発生し、実行中の RPC レスポンス待機が中断された。
    Redirected,
    /// シグナリング時に指定されていないラベルで DataChannel にメッセージを送信しようとした。
    InvalidDataChannelLabel {
        /// 指定されたラベル名。
        label: String,
    },
    /// UTF-8 デコードに失敗した。内部エラーとして [`std::string::FromUtf8Error`] を保持する。
    Utf8DecodeFailed(std::string::FromUtf8Error),
    /// candidate は未対応。
    CandidateNotSupported,
    /// 未対応のメッセージタイプを受信した。
    UnsupportedMessageType {
        /// 受信した未対応のメッセージ種別。
        message_type: String,
    },
    /// コマンドの送信に失敗した。
    CommandSendFailed {
        /// 失敗理由。
        reason: String,
        /// 失敗したコマンド名。
        command: &'static str,
    },
    /// コマンドの応答を受信できなかった。内部エラーとして [`tokio::sync::oneshot::error::RecvError`] を保持する。
    CommandResponseMissing {
        /// oneshot 受信エラー。
        source: oneshot::error::RecvError,
        /// 失敗したコマンド名。
        command: &'static str,
    },
    /// ビデオコーデックの capability 指定が不正。
    InvalidVideoCodecCapability {
        /// 失敗理由。
        reason: String,
    },
    /// ビデオコーデックの preference 指定が不正。
    InvalidVideoCodecPreference {
        /// 失敗理由。
        reason: String,
    },
    /// libcamera からのエラーメッセージ（`feature = "libcamera"` 時のみ有効）。
    #[cfg(feature = "libcamera")]
    LibcameraMessage {
        /// エラーメッセージ。
        message: String,
    },
    /// libcamera のエラー（`feature = "libcamera"` 時のみ有効）。内部エラーとして [`shiguredo_libcamera::Error`] を保持する。
    #[cfg(feature = "libcamera")]
    Libcamera(shiguredo_libcamera::Error),
    /// OpenH264 のエラー（`feature = "openh264"` 時のみ有効）。内部エラーとして [`shiguredo_openh264::Error`] を保持する。
    #[cfg(feature = "openh264")]
    Openh264(shiguredo_openh264::Error),
    /// AMF のエラー（`feature = "amf"` 時のみ有効）。内部エラーとして [`shiguredo_amf::Error`] を保持する。
    #[cfg(feature = "amf")]
    Amf {
        /// 発生した AMF エラー。
        source: shiguredo_amf::Error,
    },
    /// AMF からのエラーメッセージ（`feature = "amf"` 時のみ有効）。
    #[cfg(feature = "amf")]
    AmfMessage {
        /// エラーメッセージ。
        reason: String,
    },
    /// VPL のエラー（`feature = "vpl"` 時のみ有効）。内部エラーとして [`shiguredo_vpl::Error`] を保持する。
    #[cfg(feature = "vpl")]
    Vpl {
        /// 発生した VPL エラー。
        source: shiguredo_vpl::Error,
    },
    /// VPL からのエラーメッセージ（`feature = "vpl"` 時のみ有効）。
    #[cfg(feature = "vpl")]
    VplMessage {
        /// エラーメッセージ。
        reason: String,
    },
    /// RPC の応答がタイムアウトした。
    RpcTimeout,
    /// RPC の応答が JSON-RPC 2.0 の要件を満たしていない。
    RpcProtocolViolation {
        /// SDK が生成した Request ID と相関できた id。
        /// 相関できない場合は `None`。
        id: Option<u64>,
    },
    /// シグナリング URL が空。
    SignalingUrlsEmpty,
    /// すべてのシグナリング URL への接続が失敗した。
    AllSignalingUrlsFailed {
        /// 各シグナリング URL とそのエラーメッセージのリスト。
        errors: Vec<(String, String)>,
    },
    /// TURN-TLS CA 証明書が不正。
    TurnTlsCaCert {
        /// エラーメッセージ。
        message: String,
    },
    /// クライアント証明書の PEM パースに失敗した。
    ClientCertParse,
    /// クライアント秘密鍵の PEM パースに失敗した。
    ClientKeyParse,
    /// CA 証明書の PEM パースに失敗した。
    CaCertParse,
    /// `client_cert` と `client_key` の指定が不完全（両方を指定する必要がある）。
    ClientCertKeyIncomplete,
    /// NVCodec のエラー（`feature = "nvcodec"` 時のみ有効）。内部エラーとして [`shiguredo_nvcodec::Error`] を保持する。
    #[cfg(feature = "nvcodec")]
    NvCodec {
        /// 発生した NvCodec エラー。
        source: shiguredo_nvcodec::Error,
    },
    /// NVCodec からのエラーメッセージ（`feature = "nvcodec"` 時のみ有効）。
    #[cfg(feature = "nvcodec")]
    NvCodecMessage {
        /// エラーメッセージ。
        reason: String,
    },
    /// V4L2 のエラー（`feature = "v4l2"` 時のみ有効）。内部エラーとして [`shiguredo_v4l2::v4l2_m2m::Error`] を保持する。
    #[cfg(feature = "v4l2")]
    V4l2 {
        /// 発生した V4L2 エラー。
        source: shiguredo_v4l2::v4l2_m2m::Error,
    },
    /// V4L2 からのエラーメッセージ（`feature = "v4l2"` 時のみ有効）。
    #[cfg(feature = "v4l2")]
    V4l2Message {
        /// エラーメッセージ。
        reason: String,
    },
    /// システム時刻が不正（UNIX エポック以前など）。内部エラーとして [`std::time::SystemTimeError`] を保持する。
    InvalidSystemTime {
        /// 発生した時刻エラー。
        source: std::time::SystemTimeError,
    },
    /// MP4 ファイルの処理に失敗した。内部エラーとして [`Mp4Error`] を保持する。
    Mp4 {
        /// 発生した MP4 エラー。
        source: Mp4Error,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidRole { value } => write!(
                f,
                "--role は sendonly, recvonly, sendrecv のみ対応です: {value}"
            ),
            Error::HostEmpty => f.write_str("ホストが空です"),
            Error::HostInvalidFormat => f.write_str("ホストの指定が不正です"),
            Error::UriParse(err) => write!(f, "URL の解析に失敗しました: {err}"),
            Error::UrlMissingScheme => f.write_str("URL にスキームがありません"),
            Error::UrlUnsupportedScheme { scheme } => write!(
                f,
                "URL のスキームは ws:// または wss:// のみ対応しています: {scheme}"
            ),
            Error::UrlUserinfoNotSupported => f.write_str("URL の userinfo は未対応です"),
            Error::UrlFragmentNotAllowed => f.write_str("URL のフラグメントは指定できません"),
            Error::UrlMissingHost => f.write_str("URL にホストがありません"),
            Error::ProxyUrlUnsupportedScheme { scheme } => write!(
                f,
                "Proxy URL のスキームは http:// のみ対応しています: {scheme}"
            ),
            Error::ProxyUrlUserinfoNotSupported => {
                f.write_str("Proxy URL の userinfo は未対応です")
            }
            Error::ProxyUrlFragmentNotAllowed => {
                f.write_str("Proxy URL のフラグメントは指定できません")
            }
            Error::ProxyUrlMissingHost => f.write_str("Proxy URL にホストがありません"),
            Error::ProxyUrlPathNotAllowed { path } => {
                write!(f, "Proxy URL のパスは / のみ指定できます: {path}")
            }
            Error::ProxyUrlQueryNotAllowed => f.write_str("Proxy URL のクエリは指定できません"),
            Error::ProxyConnectDecode(err) => {
                write!(f, "Proxy CONNECT レスポンスの解析に失敗しました: {err}")
            }
            Error::ProxyConnectEncode(err) => {
                write!(f, "Proxy CONNECT リクエストの生成に失敗しました: {err}")
            }
            Error::ProxyConnectResponseMissing => {
                f.write_str("Proxy CONNECT レスポンス受信前に接続が閉じられました")
            }
            Error::ProxyConnectStatusNotSuccessful {
                status_code,
                reason_phrase,
            } => write!(
                f,
                "Proxy CONNECT が失敗しました: status={status_code} reason={reason_phrase}"
            ),
            Error::ProxyConnectUnexpectedTrailingData => {
                f.write_str("Proxy CONNECT 応答後に不正な余剰データを受信しました")
            }
            Error::ProxyAuth(err) => write!(f, "Proxy 認証ヘッダーの生成に失敗しました: {err}"),
            Error::DnsResolve { host, source } => {
                write!(f, "DNS 解決に失敗しました: {host}: {source}")
            }
            Error::NoResolvedAddress { host, port } => {
                write!(f, "接続先のアドレスが見つかりません: {host}:{port}")
            }
            Error::TcpConnectTimeout { host, port } => {
                write!(f, "TCP 接続がタイムアウトしました: {host}:{port}")
            }
            Error::TcpConnect { host, port, source } => {
                write!(f, "TCP 接続に失敗しました: {host}:{port}: {source}")
            }
            Error::TlsConfig(err) => {
                write!(f, "証明書検証の設定に失敗しました: {err}")
            }
            Error::InvalidServerName(err) => {
                write!(f, "ServerName の生成に失敗しました: {err}")
            }
            Error::TlsConnectTimeout { host } => {
                write!(f, "TLS 接続がタイムアウトしました: {host}")
            }
            Error::TlsConnect { host, source } => {
                write!(f, "TLS 接続に失敗しました: {host}: {source}")
            }
            Error::Websocket(err) => write!(f, "WebSocket エラー: {err}"),
            Error::Io(err) => write!(f, "IO エラー: {err}"),
            Error::JsonParse(err) => write!(f, "JSON エラー: {err}"),
            Error::Webrtc(err) => write!(f, "WebRTC エラー: {err}"),
            Error::PeerConnectionMissing => f.write_str("PeerConnection がありません"),
            Error::SetRemoteDescriptionTimeout => {
                f.write_str("SetRemoteDescription がタイムアウトしました")
            }
            Error::SetRemoteDescriptionResponseMissing => {
                f.write_str("SetRemoteDescription の応答を受信できませんでした")
            }
            Error::SetRemoteDescriptionFailed { reason } => {
                write!(f, "SetRemoteDescription が失敗しました: {reason}")
            }
            Error::AnswerTimeout => f.write_str("Answer の生成がタイムアウトしました"),
            Error::AnswerResponseMissing => f.write_str("Answer の応答を受信できませんでした"),
            Error::AnswerFailed { reason } => write!(f, "Answer の生成に失敗しました: {reason}"),
            Error::SetLocalDescriptionTimeout => {
                f.write_str("SetLocalDescription がタイムアウトしました")
            }
            Error::SetLocalDescriptionResponseMissing => {
                f.write_str("SetLocalDescription の応答を受信できませんでした")
            }
            Error::SetLocalDescriptionFailed { reason } => {
                write!(f, "SetLocalDescription が失敗しました: {reason}")
            }
            Error::SimulcastVideoSenderMissing => {
                f.write_str("simulcast の適用対象となる video sender がありません")
            }
            Error::SimulcastSetParametersFailed { source } => {
                write!(f, "simulcast の SetParameters が失敗しました: {source}")
            }
            Error::DataChannelMissing { label } => {
                write!(f, "DataChannel がありません: {label}")
            }
            Error::DataChannelSendFailed => f.write_str("DataChannel への送信に失敗しました"),
            Error::Redirected => f.write_str("リダイレクトが発生したため中断されました"),
            Error::InvalidDataChannelLabel { label } => {
                write!(f, "シグナリング時に指定されていないラベルです: {label}")
            }
            Error::Utf8DecodeFailed(err) => {
                write!(f, "UTF-8 デコードに失敗しました: {err}")
            }
            Error::CandidateNotSupported => f.write_str("candidate は未対応です"),
            Error::UnsupportedMessageType { message_type } => {
                write!(f, "未対応のメッセージを受信しました: {message_type}")
            }
            Error::CommandSendFailed { command, reason } => {
                write!(f, "コマンドの送信に失敗しました: {command}: {reason}")
            }
            Error::CommandResponseMissing { command, source } => {
                write!(
                    f,
                    "コマンドの応答を受信できませんでした: {command}: {source}"
                )
            }
            Error::InvalidVideoCodecCapability { reason } => {
                write!(f, "VideoCodecCapability が不正です: {reason}")
            }
            Error::InvalidVideoCodecPreference { reason } => {
                write!(f, "VideoCodecPreference が不正です: {reason}")
            }
            #[cfg(feature = "libcamera")]
            Error::LibcameraMessage { message } => write!(f, "libcamera error: {message}"),
            #[cfg(feature = "libcamera")]
            Error::Libcamera(err) => write!(f, "libcamera error: {err}"),
            #[cfg(feature = "openh264")]
            Error::Openh264(err) => write!(f, "OpenH264 error: {err}"),
            #[cfg(feature = "amf")]
            Error::Amf { source } => write!(f, "AMF error: {source}"),
            #[cfg(feature = "amf")]
            Error::AmfMessage { reason } => write!(f, "AMF error: {reason}"),
            #[cfg(feature = "vpl")]
            Error::Vpl { source } => write!(f, "VPL error: {source}"),
            #[cfg(feature = "vpl")]
            Error::VplMessage { reason } => write!(f, "VPL error: {reason}"),
            Error::RpcTimeout => f.write_str("RPC レスポンスがタイムアウトしました"),
            Error::RpcProtocolViolation { id } => match id {
                Some(id) => {
                    write!(
                        f,
                        "RPC レスポンスが JSON-RPC 2.0 の要件を満たしていません: id={id}"
                    )
                }
                None => f.write_str("RPC レスポンスが JSON-RPC 2.0 の要件を満たしていません"),
            },
            Error::SignalingUrlsEmpty => f.write_str("シグナリング URL が指定されていません"),
            Error::AllSignalingUrlsFailed { errors } => {
                write!(f, "すべてのシグナリング URL への接続に失敗しました:")?;
                for (url, err) in errors {
                    write!(f, "\n  {url}: {err}")?;
                }
                Ok(())
            }
            Error::TurnTlsCaCert { message } => {
                write!(f, "TURN-TLS CA 証明書の解析に失敗しました: {message}")
            }
            Error::ClientCertParse => f.write_str("クライアント証明書の PEM パースに失敗しました"),
            Error::ClientKeyParse => f.write_str("クライアント秘密鍵の PEM パースに失敗しました"),
            Error::CaCertParse => f.write_str("CA 証明書の PEM パースに失敗しました"),
            Error::ClientCertKeyIncomplete => {
                f.write_str("client_cert と client_key は両方を指定する必要があります")
            }
            #[cfg(feature = "nvcodec")]
            Error::NvCodec { source } => write!(f, "NVCodec error: {source}"),
            #[cfg(feature = "nvcodec")]
            Error::NvCodecMessage { reason } => write!(f, "NVCodec error: {reason}"),
            #[cfg(feature = "v4l2")]
            Error::V4l2 { source } => write!(f, "V4L2 error: {source}"),
            #[cfg(feature = "v4l2")]
            Error::V4l2Message { reason } => write!(f, "V4L2 error: {reason}"),
            Error::InvalidSystemTime { source } => {
                write!(
                    f,
                    "システム時刻が UNIX エポック (1970-01-01) より前です: {source}"
                )
            }
            Error::Mp4 { source } => write!(f, "MP4 エラー: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::UriParse(err) => Some(err),
            Error::ProxyConnectDecode(err) => Some(err),
            Error::ProxyConnectEncode(err) => Some(err),
            Error::ProxyAuth(err) => Some(err),
            Error::DnsResolve { source, .. } => Some(source),
            Error::TcpConnect { source, .. } => Some(source),
            Error::TlsConfig(err) => Some(err),
            Error::InvalidServerName(err) => Some(err),
            Error::TlsConnect { source, .. } => Some(source),
            Error::Websocket(err) => Some(err),
            Error::Io(err) => Some(err),
            Error::JsonParse(err) => Some(err),
            Error::Webrtc(err) => Some(err),
            #[cfg(feature = "libcamera")]
            Error::Libcamera(err) => Some(err),
            #[cfg(feature = "openh264")]
            Error::Openh264(err) => Some(err),
            #[cfg(feature = "amf")]
            Error::Amf { source } => Some(source),
            #[cfg(feature = "vpl")]
            Error::Vpl { source } => Some(source),
            Error::SimulcastSetParametersFailed { source } => Some(source),
            Error::Utf8DecodeFailed(err) => Some(err),
            Error::CommandResponseMissing { source, .. } => Some(source),
            #[cfg(feature = "v4l2")]
            Error::V4l2 { source } => Some(source),
            Error::InvalidSystemTime { source } => Some(source),
            Error::Mp4 { source } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(err: std::string::FromUtf8Error) -> Self {
        Error::Utf8DecodeFailed(err)
    }
}

impl From<UriError> for Error {
    fn from(err: UriError) -> Self {
        Error::UriParse(err)
    }
}

impl From<shiguredo_http11::Error> for Error {
    fn from(err: shiguredo_http11::Error) -> Self {
        Error::ProxyConnectDecode(err)
    }
}

impl From<EncodeError> for Error {
    fn from(err: EncodeError) -> Self {
        Error::ProxyConnectEncode(err)
    }
}

impl From<AuthError> for Error {
    fn from(err: AuthError) -> Self {
        Error::ProxyAuth(err)
    }
}

impl From<rustls::Error> for Error {
    fn from(err: rustls::Error) -> Self {
        Error::TlsConfig(err)
    }
}

impl From<rustls::pki_types::InvalidDnsNameError> for Error {
    fn from(err: rustls::pki_types::InvalidDnsNameError) -> Self {
        Error::InvalidServerName(err)
    }
}

impl From<JsonParseError> for Error {
    fn from(err: JsonParseError) -> Self {
        Error::JsonParse(err)
    }
}

impl From<shiguredo_webrtc::Error> for Error {
    fn from(err: shiguredo_webrtc::Error) -> Self {
        Error::Webrtc(err)
    }
}

#[cfg(feature = "libcamera")]
impl From<shiguredo_libcamera::Error> for Error {
    fn from(err: shiguredo_libcamera::Error) -> Self {
        Error::Libcamera(err)
    }
}

#[cfg(feature = "openh264")]
impl From<shiguredo_openh264::Error> for Error {
    fn from(err: shiguredo_openh264::Error) -> Self {
        Error::Openh264(err)
    }
}

#[cfg(feature = "amf")]
impl From<shiguredo_amf::Error> for Error {
    fn from(err: shiguredo_amf::Error) -> Self {
        Error::Amf { source: err }
    }
}

#[cfg(feature = "vpl")]
impl From<shiguredo_vpl::Error> for Error {
    fn from(err: shiguredo_vpl::Error) -> Self {
        Error::Vpl { source: err }
    }
}

impl From<shiguredo_websocket::Error> for Error {
    fn from(err: shiguredo_websocket::Error) -> Self {
        Error::Websocket(err)
    }
}

#[cfg(feature = "nvcodec")]
impl From<shiguredo_nvcodec::Error> for Error {
    fn from(err: shiguredo_nvcodec::Error) -> Self {
        Error::NvCodec { source: err }
    }
}

#[cfg(feature = "v4l2")]
impl From<shiguredo_v4l2::v4l2_m2m::Error> for Error {
    fn from(err: shiguredo_v4l2::v4l2_m2m::Error) -> Self {
        Error::V4l2 { source: err }
    }
}

impl From<std::time::SystemTimeError> for Error {
    fn from(err: std::time::SystemTimeError) -> Self {
        Error::InvalidSystemTime { source: err }
    }
}

impl From<Mp4Error> for Error {
    fn from(err: Mp4Error) -> Self {
        Error::Mp4 { source: err }
    }
}

/// SDK のエラー型 [`Error`] をエラーパラメータに持つ `std::result::Result` の 1 引数エイリアス。
///
/// ```
/// use sora_sdk::Result;
///
/// fn example() -> Result<()> {
///     Ok(())
/// }
/// ```
pub type Result<T> = std::result::Result<T, Error>;
