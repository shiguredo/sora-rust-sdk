//! エラー定義と Result 型。
use std::io;

use nojson::JsonParseError;
use shiguredo_http11::{Error as Http11Error, auth::AuthError, uri::UriError};
use tokio::sync::{mpsc, oneshot};

use crate::client::SoraClientCommand;

#[derive(Debug)]
pub enum Error {
    InvalidRole {
        value: String,
    },
    HostEmpty,
    HostInvalidFormat,
    UriParse(UriError),
    UrlMissingScheme,
    UrlUnsupportedScheme {
        scheme: String,
    },
    UrlUserinfoNotSupported,
    UrlFragmentNotAllowed,
    UrlMissingHost,
    ProxyUrlUnsupportedScheme {
        scheme: String,
    },
    ProxyUrlUserinfoNotSupported,
    ProxyUrlFragmentNotAllowed,
    ProxyUrlMissingHost,
    ProxyUrlPathNotAllowed {
        path: String,
    },
    ProxyUrlQueryNotAllowed,
    ProxyConnectDecode(Http11Error),
    ProxyConnectResponseMissing,
    ProxyConnectStatusNotSuccessful {
        status_code: u16,
        reason_phrase: String,
    },
    ProxyAuth(AuthError),
    DnsResolve {
        host: String,
        source: io::Error,
    },
    NoResolvedAddress {
        host: String,
        port: u16,
    },
    TcpConnectTimeout {
        host: String,
        port: u16,
    },
    TcpConnect {
        host: String,
        port: u16,
        source: io::Error,
    },
    TlsConfig(rustls::Error),
    InvalidServerName(rustls::pki_types::InvalidDnsNameError),
    TlsConnectTimeout {
        host: String,
    },
    TlsConnect {
        host: String,
        source: io::Error,
    },
    Websocket(shiguredo_websocket::Error),
    Io(io::Error),
    JsonParse(JsonParseError),
    Webrtc(shiguredo_webrtc::Error),
    PeerConnectionMissing,
    SetRemoteDescriptionTimeout,
    SetRemoteDescriptionResponseMissing,
    SetRemoteDescriptionFailed {
        reason: String,
    },
    AnswerTimeout,
    AnswerResponseMissing,
    AnswerFailed {
        reason: String,
    },
    SetLocalDescriptionTimeout,
    SetLocalDescriptionResponseMissing,
    SetLocalDescriptionFailed {
        reason: String,
    },
    SimulcastVideoSenderMissing,
    SimulcastSetParametersFailed {
        source: shiguredo_webrtc::Error,
    },
    DataChannelMissing {
        label: String,
    },
    DataChannelSendFailed,
    Utf8DecodeFailed(std::string::FromUtf8Error),
    CandidateNotSupported,
    UnsupportedMessageType {
        message_type: String,
    },
    CommandSendFailed {
        source: mpsc::error::SendError<SoraClientCommand>,
        command: &'static str,
    },
    CommandResponseMissing {
        source: oneshot::error::RecvError,
        command: &'static str,
    },
    InvalidVideoCodecCapability {
        reason: String,
    },
    InvalidVideoCodecPreference {
        reason: String,
    },
    RpcTimeout,
    SignalingUrlsEmpty,
    AllSignalingUrlsFailed {
        errors: Vec<(String, String)>,
    },
    TurnTlsCaCert {
        message: String,
    },
    ClientCertParse,
    ClientKeyParse,
    CaCertParse,
    ClientCertKeyIncomplete,
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
            Error::Utf8DecodeFailed(err) => {
                write!(f, "UTF-8 デコードに失敗しました: {err}")
            }
            Error::CandidateNotSupported => f.write_str("candidate は未対応です"),
            Error::UnsupportedMessageType { message_type } => {
                write!(f, "未対応のメッセージを受信しました: {message_type}")
            }
            Error::CommandSendFailed { command, source } => {
                write!(f, "コマンドの送信に失敗しました: {command}: {source}")
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
            Error::RpcTimeout => f.write_str("RPC レスポンスがタイムアウトしました"),
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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::UriParse(err) => Some(err),
            Error::ProxyConnectDecode(err) => Some(err),
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
            Error::SimulcastSetParametersFailed { source } => Some(source),
            Error::Utf8DecodeFailed(err) => Some(err),
            Error::CommandSendFailed { source, .. } => Some(source),
            Error::CommandResponseMissing { source, .. } => Some(source),
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

impl From<Http11Error> for Error {
    fn from(err: Http11Error) -> Self {
        Error::ProxyConnectDecode(err)
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

impl From<shiguredo_websocket::Error> for Error {
    fn from(err: shiguredo_websocket::Error) -> Self {
        Error::Websocket(err)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
