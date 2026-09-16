use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_lc_rs::hmac;
use base64ct::{Base64UrlUnpadded, Encoding};
use nojson::{JsonObjectFormatter, RawJson};
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::ConfigVerifierExt;
use shiguredo_http11::{Request, ResponseDecoder, uri::Uri};
use shiguredo_webrtc::{AudioTrack, VideoTrack};
use sora_sdk::{JsonString, Result, SoraConnectionContext};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::stats::{
    RtcOutboundRtpStreamStats, RtcReceivedRtpStreamStatsTrait, RtcRtpStreamStatsTrait,
    RtcSentRtpStreamStatsTrait, RtcStatsTrait, WebRtcStat, WebRtcStatsReport,
};

/// CI 環境 (GitHub Actions) で実行されているかどうかを返す。
pub fn is_running_on_ci() -> bool {
    env::var("CI").is_ok_and(|v| v == "true")
        || env::var("GITHUB_ACTIONS").is_ok_and(|v| v == "true")
}

/// .env ファイルを読み込んで環境変数に設定する。
///
/// 既存の環境変数は上書きしない。
/// e2e-tests ディレクトリ内の .env を読み込む。
pub fn load_env() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let env_path = format!("{}/.env", manifest_dir);
    load_env_from(env_path);
}

/// 指定したパスから .env ファイルを読み込む。
pub fn load_env_from<P: AsRef<Path>>(path: P) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };

    for line in content.lines() {
        let line = line.trim();

        // 空行とコメントをスキップ
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // KEY=VALUE 形式をパース
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        let value = value.trim();

        // クォートを除去
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);

        // 既存の環境変数は上書きしない
        if env::var(key).is_err() {
            unsafe {
                env::set_var(key, value);
            }
        }
    }
}

/// シグナリング URL のリストを取得する。
///
/// 環境変数 `TEST_SIGNALING_URLS` をカンマ区切りで分割して返す。
pub fn signaling_urls() -> Result<Vec<String>> {
    let value = env::var("TEST_SIGNALING_URLS")
        .map_err(|_| io::Error::other("TEST_SIGNALING_URLS が設定されていません"))?;
    Ok(value.split(',').map(|s| s.trim().to_string()).collect())
}

/// チャネル ID を生成する。
pub fn generate_channel_id() -> String {
    let prefix = env::var("TEST_CHANNEL_ID_PREFIX").unwrap_or_else(|_| "e2e-test".to_string());
    let suffix = env::var("TEST_CHANNEL_ID_SUFFIX").unwrap_or_default();
    let random = shiguredo_webrtc::random_bytes(8);
    let hex: String = random.iter().map(|b| format!("{:02x}", b)).collect();
    format!("{}{}{}", prefix, hex, suffix)
}

/// シークレットキーを取得する (設定されている場合)。
pub fn secret_key() -> Option<String> {
    env::var("TEST_SECRET_KEY").ok()
}

/// access_token (JWT) を生成する。
///
/// `channel_id` と `exp` を含むペイロードに、`extra_claims` クロージャで
/// 追加の claims を書き込み、HMAC-SHA256 で署名する。
///
/// ```ignore
/// generate_access_token(&channel_id, &secret, |f| {
///     f.member("cluster_affinity", true)
/// });
/// ```
pub fn generate_access_token<F>(channel_id: &str, secret: &str, extra_claims: F) -> String
where
    F: Fn(&mut JsonObjectFormatter<'_, '_, '_>) -> std::fmt::Result,
{
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("システム時刻の取得に失敗しました")
        .as_secs();
    let exp = now + 300;

    let payload = nojson::object(|f| {
        f.member("channel_id", channel_id)?;
        f.member("exp", exp)?;
        extra_claims(f)
    })
    .to_string();

    let header = r#"{"alg":"HS256","typ":"JWT"}"#;

    let signing_input = format!(
        "{}.{}",
        Base64UrlUnpadded::encode_string(header.as_bytes()),
        Base64UrlUnpadded::encode_string(payload.as_bytes()),
    );

    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let signature = hmac::sign(&key, signing_input.as_bytes());

    format!(
        "{}.{}",
        signing_input,
        Base64UrlUnpadded::encode_string(signature.as_ref())
    )
}

/// access_token を含む metadata JSON 文字列を生成する。
pub fn build_metadata_with_access_token(access_token: &str) -> JsonString {
    use nojson::{DisplayJson, Json, JsonFormatter};

    struct Metadata<'a> {
        access_token: &'a str,
    }

    impl DisplayJson for Metadata<'_> {
        fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
            f.object(|f| f.member("access_token", self.access_token))
        }
    }

    let metadata = Json(Metadata { access_token }).to_string();
    metadata
        .parse()
        .expect("metadata JSON の生成に失敗しました")
}

/// FakeVideoCapturer を使って送信用のトラックを作成する。
pub fn build_sender_tracks(
    context: &SoraConnectionContext,
    capturer: &mut FakeVideoCapturer,
) -> Result<(VideoTrack, AudioTrack)> {
    if !capturer.start() {
        return Err(io::Error::other("FakeVideoCapturer の開始に失敗しました").into());
    }
    let video_track = context.create_video_track(&capturer.video_source())?;
    let audio_source = context.create_audio_source()?;
    let audio_track = context.create_audio_track(&audio_source)?;
    Ok((video_track, audio_track))
}

/// API URL を取得する (設定されている場合)。
pub fn api_url() -> Option<String> {
    env::var("TEST_API_URL").ok()
}

/// DisconnectChannel API の応答種別。
///
/// Sora ドキュメント「クラスター機能」の HTTP API のリダイレクト機能に基づき、
/// 307 (Temporary Redirect) の応答を追従するために 2xx とリダイレクトを区別する。
enum DisconnectChannelResponse {
    /// 応答が 3xx で Location ヘッダーを含む場合のリダイレクト先
    Redirect { location: String },
    /// 2xx の応答で、response body の `channel_id` が要求値と一致した場合
    Success,
}

/// DisconnectChannel API を実行して、指定したチャネルの接続をすべて切断する。
///
/// Sora ドキュメント「API」のコネクション API 仕様に基づき、`api_url` に対して
/// 実 HTTP 接続で `POST` リクエストを送信する。
/// 外部の `curl` プロセス、モック、スタブは使わない。
///
/// - request header は `x-sora-target: Sora_20151104.DisconnectChannel`
/// - request header は `Content-Type: application/json`
/// - request body は `{"channel_id":"<channel_id>"}`
///
/// `http://` は Tokio の実 `TcpStream`、`https://` は `rustls` と
/// `rustls-platform-verifier` による実 TLS stream を使う。
/// connect、request write、response header / body read の全体へ共通の 5 秒
/// timeout を適用する。
/// HTTP response が 2xx でない場合、response の decode に失敗した場合、または
/// response body の `channel_id` が要求値と一致しない場合は `Err` を返す。
///
/// クラスター機能により、指定したチャネル ID を他ノードが担当している場合は
/// 307 (Temporary Redirect) の HTTP 応答でリダイレクトされる。この場合は
/// Location ヘッダーを追従して再要求する。
/// 無限ループを防ぐため、リダイレクトは最大 10 回まで追従する。
pub async fn disconnect_channel(
    api_url: &str,
    channel_id: &str,
) -> std::result::Result<(), io::Error> {
    let mut url = api_url.to_string();
    for _ in 0..10 {
        match send_disconnect_channel_request_to_url(&url, channel_id).await? {
            DisconnectChannelResponse::Success => return Ok(()),
            DisconnectChannelResponse::Redirect { location } => {
                url = resolve_redirect_url(&url, &location)?;
            }
        }
    }
    Err(io::Error::other(
        "DisconnectChannel API のリダイレクトが多すぎます",
    ))
}

/// リダイレクトの Location ヘッダーを現在の URL に対して解決する。
///
/// Location が絶対 URL (`http://` または `https://`) の場合はそのまま使い、
/// `/` から始まる相対パスの場合は現在の URL の scheme、host、port に基づいて
/// 解決する。
/// それ以外の相対 URL は解決できないため `Err` を返す。
fn resolve_redirect_url(
    current_url: &str,
    location: &str,
) -> std::result::Result<String, io::Error> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }
    let base = Uri::parse(current_url)
        .map_err(|e| io::Error::other(format!("リダイレクト元 URL の解析に失敗しました: {e}")))?;
    let scheme = base
        .scheme()
        .ok_or_else(|| io::Error::other("リダイレクト元 URL に scheme がありません"))?;
    let host = base
        .host()
        .ok_or_else(|| io::Error::other("リダイレクト元 URL に host がありません"))?;
    let authority = match base.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let location = location.strip_prefix('/').unwrap_or(location);
    Ok(format!("{scheme}://{authority}/{location}"))
}

/// 単一 URL への DisconnectChannel API リクエストを送信して応答を検証する。
///
/// connect、request write、response header / body read の全体へ共通の 5 秒
/// timeout を適用する。
async fn send_disconnect_channel_request_to_url(
    api_url: &str,
    channel_id: &str,
) -> std::result::Result<DisconnectChannelResponse, io::Error> {
    let uri = Uri::parse(api_url)
        .map_err(|e| io::Error::other(format!("TEST_API_URL の解析に失敗しました: {e}")))?;
    let scheme = uri
        .scheme()
        .ok_or_else(|| io::Error::other("TEST_API_URL に scheme がありません"))?;
    let tls = if scheme.eq_ignore_ascii_case("http") {
        false
    } else if scheme.eq_ignore_ascii_case("https") {
        true
    } else {
        return Err(io::Error::other(format!(
            "TEST_API_URL の scheme は http または https のみ対応しています: {scheme}"
        )));
    };
    let host = uri
        .host()
        .ok_or_else(|| io::Error::other("TEST_API_URL に host がありません"))?;
    let port = uri.port().unwrap_or(if tls { 443 } else { 80 });

    // 接続用のホストは IPv6 のブラケットを外す
    let connect_host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);

    // request body は nojson で生成する
    let body = nojson::object(|f| f.member("channel_id", channel_id)).to_string();

    // request target は URL の path + query を保持し、Host は URL に明示された
    // host と port を保持する
    let request = Request::new("POST", uri.origin_form())
        .map_err(encode_err)?
        .header("Host", host_header(host, uri.port()))
        .map_err(encode_err)?
        .header("x-sora-target", "Sora_20151104.DisconnectChannel")
        .map_err(encode_err)?
        .header("Content-Type", "application/json")
        .map_err(encode_err)?
        .body(body)
        .encode()
        .map_err(encode_err)?;

    tokio::time::timeout(Duration::from_secs(5), async {
        let tcp_stream = TcpStream::connect((connect_host, port)).await?;
        if tls {
            let config = ClientConfig::with_platform_verifier()
                .map_err(|e| io::Error::other(format!("TLS 設定の作成に失敗しました: {e}")))?;
            let server_name = ServerName::try_from(connect_host.to_string())
                .map_err(|e| io::Error::other(format!("ServerName の生成に失敗しました: {e}")))?;
            let connector = TlsConnector::from(Arc::new(config));
            let tls_stream = connector
                .connect(server_name, tcp_stream)
                .await
                .map_err(|e| io::Error::other(format!("TLS 接続に失敗しました: {e}")))?;
            send_disconnect_channel_request(tls_stream, &request, channel_id).await
        } else {
            send_disconnect_channel_request(tcp_stream, &request, channel_id).await
        }
    })
    .await
    .map_err(|_| io::Error::other("DisconnectChannel API がタイムアウトしました"))?
}

/// `shiguredo_http11` のエンコードエラーを `io::Error` に変換する。
fn encode_err(e: shiguredo_http11::EncodeError) -> io::Error {
    io::Error::other(format!("HTTP リクエストのエンコードに失敗しました: {e}"))
}

/// Host ヘッダーの値を構築する。
///
/// URL にポートが明示されている場合は `host:port`、明示されていない場合は
/// `host` のみを返す。
fn host_header(host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

/// DisconnectChannel API のリクエストを送信してレスポンスを検証する。
///
/// 2xx でない場合、response の decode に失敗した場合、または response body の
/// `channel_id` が要求値と一致しない場合は `Err` を返す。
/// 3xx で Location ヘッダーを含む場合は `DisconnectChannelResponse::Redirect` を返す。
async fn send_disconnect_channel_request<S>(
    mut stream: S,
    request: &[u8],
    channel_id: &str,
) -> std::result::Result<DisconnectChannelResponse, io::Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    stream.write_all(request).await?;

    let mut decoder = ResponseDecoder::new();
    decoder.set_request_method("POST");
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(io::Error::other(
                "API レスポンス受信前に接続が閉じられました",
            ));
        }
        decoder.feed(&buf[..n]).map_err(|e| {
            io::Error::other(format!("API レスポンスの decode に失敗しました: {e}"))
        })?;
        if let Some(response) = decoder
            .decode()
            .map_err(|e| io::Error::other(format!("API レスポンスの decode に失敗しました: {e}")))?
        {
            let status = response.status_code();
            // Sora ドキュメント「クラスター機能」の HTTP API のリダイレクト機能に基づき、
            // 指定したチャネル ID を他ノードが担当している場合に 307 (Temporary Redirect) で
            // そのノードへ誘導される。Location ヘッダーを追従して再要求する。
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(location) = response.get_header("Location")
            {
                return Ok(DisconnectChannelResponse::Redirect {
                    location: location.to_string(),
                });
            }
            if !(200..300).contains(&status) {
                return Err(io::Error::other(format!(
                    "DisconnectChannel API が失敗しました: status={status}"
                )));
            }
            let body = response
                .body_bytes()
                .ok_or_else(|| io::Error::other("API レスポンスに body がありません"))?;
            let body_str = std::str::from_utf8(body).map_err(|e| {
                io::Error::other(format!("API レスポンス body が UTF-8 ではありません: {e}"))
            })?;
            let json = RawJson::parse(body_str).map_err(|e| {
                io::Error::other(format!(
                    "API レスポンス body の JSON パースに失敗しました: {e}"
                ))
            })?;
            let actual_channel_id: String = json
                .value()
                .to_member("channel_id")
                .and_then(|v| v.required())
                .and_then(|v| v.try_into())
                .map_err(|e| {
                    io::Error::other(format!(
                        "API レスポンス body の channel_id が取得できませんでした: {e}"
                    ))
                })?;
            if actual_channel_id != channel_id {
                return Err(io::Error::other(format!(
                    "API レスポンスの channel_id が要求値と一致しません: expected={channel_id} actual={actual_channel_id}"
                )));
            }
            return Ok(DisconnectChannelResponse::Success);
        }
    }
}

/// OpenH264 動的ライブラリのパスを取得する (設定されている場合)。
pub fn openh264_path() -> Option<String> {
    env::var("OPENH264_PATH").ok()
}

fn parse_stats_lossy(stats_json: &JsonString) -> Vec<WebRtcStat> {
    use nojson::RawJson;

    if let Ok(report) = WebRtcStatsReport::parse(stats_json) {
        return report.stats;
    }

    let json_str = stats_json.to_string();
    let Ok(json) = RawJson::parse(&json_str) else {
        return Vec::new();
    };
    let Ok(array) = json.value().to_array() else {
        return Vec::new();
    };
    array
        .filter_map(|item| WebRtcStat::try_from(item).ok())
        .collect()
}

fn has_stat_type<T: RtcStatsTrait>(stat: &T, expected: &str) -> bool {
    stat.stats_type().as_str() == expected
}

fn stat_kind(stat: &WebRtcStat) -> Option<String> {
    match stat {
        WebRtcStat::InboundRtp(v) => Some(v.kind()),
        WebRtcStat::OutboundRtp(v) => Some(v.kind()),
        WebRtcStat::RemoteInboundRtp(v) => Some(v.kind()),
        WebRtcStat::RemoteOutboundRtp(v) => Some(v.kind()),
        _ => None,
    }
}

fn numeric_field_from_received<T: RtcReceivedRtpStreamStatsTrait>(
    stat: &T,
    field_name: &str,
) -> Option<u64> {
    match field_name {
        "packetsReceived" => stat.packets_received(),
        "packetsLost" => stat
            .packets_lost()
            .and_then(|value| u64::try_from(value).ok()),
        _ => None,
    }
}

fn numeric_field_from_sent<T: RtcSentRtpStreamStatsTrait>(
    stat: &T,
    field_name: &str,
) -> Option<u64> {
    match field_name {
        "packetsSent" => stat.packets_sent(),
        "bytesSent" => stat.bytes_sent(),
        _ => None,
    }
}

fn numeric_field_value(stat: &WebRtcStat, field_name: &str) -> Option<u64> {
    match stat {
        WebRtcStat::InboundRtp(inbound) => {
            numeric_field_from_received(inbound, field_name).or(match field_name {
                "bytesReceived" => inbound.bytes_received,
                "framesReceived" => inbound.frames_received,
                "framesDecoded" => inbound.frames_decoded,
                "keyFramesDecoded" => inbound.key_frames_decoded,
                _ => None,
            })
        }
        WebRtcStat::OutboundRtp(outbound) => {
            numeric_field_from_sent(outbound, field_name).or(match field_name {
                "headerBytesSent" => outbound.header_bytes_sent,
                "retransmittedPacketsSent" => outbound.retransmitted_packets_sent,
                "retransmittedBytesSent" => outbound.retransmitted_bytes_sent,
                _ => None,
            })
        }
        WebRtcStat::RemoteInboundRtp(remote_inbound) => {
            numeric_field_from_received(remote_inbound, field_name).or(match field_name {
                "roundTripTimeMeasurements" => remote_inbound.round_trip_time_measurements,
                _ => None,
            })
        }
        WebRtcStat::RemoteOutboundRtp(remote_outbound) => {
            numeric_field_from_sent(remote_outbound, field_name).or(match field_name {
                "reportsSent" => remote_outbound.reports_sent,
                "roundTripTimeMeasurements" => remote_outbound.round_trip_time_measurements,
                _ => None,
            })
        }
        WebRtcStat::DataChannel(data_channel) => match field_name {
            "messagesSent" => data_channel.messages_sent,
            "bytesSent" => data_channel.bytes_sent,
            "messagesReceived" => data_channel.messages_received,
            "bytesReceived" => data_channel.bytes_received,
            _ => None,
        },
        WebRtcStat::Transport(transport) => match field_name {
            "packetsSent" => transport.packets_sent,
            "packetsReceived" => transport.packets_received,
            "bytesSent" => transport.bytes_sent,
            "bytesReceived" => transport.bytes_received,
            _ => None,
        },
        WebRtcStat::CandidatePair(candidate_pair) => match field_name {
            "packetsSent" => candidate_pair.packets_sent,
            "packetsReceived" => candidate_pair.packets_received,
            "bytesSent" => candidate_pair.bytes_sent,
            "bytesReceived" => candidate_pair.bytes_received,
            _ => None,
        },
        WebRtcStat::PeerConnection(peer_connection) => match field_name {
            "dataChannelsOpened" => peer_connection.data_channels_opened,
            "dataChannelsClosed" => peer_connection.data_channels_closed,
            _ => None,
        },
        WebRtcStat::Codec(_)
        | WebRtcStat::LocalCandidate(_)
        | WebRtcStat::RemoteCandidate(_)
        | WebRtcStat::Certificate(_)
        | WebRtcStat::Other(_) => None,
    }
}

fn sum_stats_field_for_type_internal(
    stats_json: &JsonString,
    stat_type: &str,
    field_name: &str,
    kind: Option<&str>,
) -> u64 {
    parse_stats_lossy(stats_json)
        .into_iter()
        .filter(|stat| has_stat_type(stat, stat_type))
        .filter(|stat| {
            kind.map(|expected| stat_kind(stat).as_deref() == Some(expected))
                .unwrap_or(true)
        })
        .filter_map(|stat| numeric_field_value(&stat, field_name))
        .sum()
}

/// 統計情報から指定した type のエントリを検索し、指定したフィールドの値を合計して返す。
///
/// WebRTC 統計情報の JSON 配列をパースし、`stat_type` に一致する type を持つエントリを探して、
/// そのエントリの `field_name` フィールドの値を合算する。
pub fn sum_stats_field_for_type(stats_json: &JsonString, stat_type: &str, field_name: &str) -> u64 {
    sum_stats_field_for_type_internal(stats_json, stat_type, field_name, None)
}

/// 統計情報から指定した type と kind=video のエントリを検索し、指定したフィールドの値を合計して返す。
pub fn sum_video_stats_field_for_type(
    stats_json: &JsonString,
    stat_type: &str,
    field_name: &str,
) -> u64 {
    sum_stats_field_for_type_internal(stats_json, stat_type, field_name, Some("video"))
}

/// 統計情報から指定した type と kind=audio のエントリを検索し、指定したフィールドの値を合計して返す。
pub fn sum_audio_stats_field_for_type(
    stats_json: &JsonString,
    stat_type: &str,
    field_name: &str,
) -> u64 {
    sum_stats_field_for_type_internal(stats_json, stat_type, field_name, Some("audio"))
}

/// 統計情報から指定した type のエントリを検索し、指定したフィールドの合計値が 0 より大きいか確認する。
pub fn verify_stats_field_positive(
    stats_json: &JsonString,
    stat_type: &str,
    field_name: &str,
) -> bool {
    sum_stats_field_for_type(stats_json, stat_type, field_name) > 0
}

/// 統計情報から指定した type と kind=video のエントリを検索し、指定したフィールドの合計値が 0 より大きいか確認する。
pub fn verify_video_stats_field_positive(
    stats_json: &JsonString,
    stat_type: &str,
    field_name: &str,
) -> bool {
    sum_video_stats_field_for_type(stats_json, stat_type, field_name) > 0
}

/// 統計情報から指定した type と kind=audio のエントリを検索し、指定したフィールドの合計値が 0 より大きいか確認する。
pub fn verify_audio_stats_field_positive(
    stats_json: &JsonString,
    stat_type: &str,
    field_name: &str,
) -> bool {
    sum_audio_stats_field_for_type(stats_json, stat_type, field_name) > 0
}

/// 統計情報から指定した kind の RTP の codec が期待する mimeType か確認する。
fn verify_codec_mime_type_internal(
    stats_json: &JsonString,
    stat_type: &str,
    expected_mime_type: &str,
    kind: &str,
) -> bool {
    use std::collections::HashSet;

    let stats = parse_stats_lossy(stats_json);
    let mut expected_codec_ids = HashSet::new();
    let mut rtp_codec_ids = Vec::new();

    for stat in &stats {
        match stat {
            WebRtcStat::Codec(codec)
                if codec.mime_type.eq_ignore_ascii_case(expected_mime_type) =>
            {
                expected_codec_ids.insert(codec.id());
            }
            WebRtcStat::InboundRtp(inbound) if stat_type == "inbound-rtp" => {
                if inbound.kind() == kind
                    && let Some(codec_id) = inbound.codec_id()
                {
                    rtp_codec_ids.push(codec_id);
                }
            }
            WebRtcStat::OutboundRtp(outbound) if stat_type == "outbound-rtp" => {
                if outbound.kind() == kind
                    && let Some(codec_id) = outbound.codec_id()
                {
                    rtp_codec_ids.push(codec_id);
                }
            }
            _ => {}
        }
    }

    if expected_codec_ids.is_empty() {
        return false;
    }
    rtp_codec_ids
        .iter()
        .any(|codec_id| expected_codec_ids.contains(codec_id))
}

/// 統計情報から video RTP の codec が期待する mimeType か確認する。
pub fn verify_video_codec_mime_type(
    stats_json: &JsonString,
    stat_type: &str,
    expected_mime_type: &str,
) -> bool {
    verify_codec_mime_type_internal(stats_json, stat_type, expected_mime_type, "video")
}

/// 統計情報から audio RTP の codec が期待する mimeType か確認する。
pub fn verify_audio_codec_mime_type(
    stats_json: &JsonString,
    stat_type: &str,
    expected_mime_type: &str,
) -> bool {
    verify_codec_mime_type_internal(stats_json, stat_type, expected_mime_type, "audio")
}

/// 統計情報から data-channel タイプのエントリを検索し、指定した label が存在するか確認する。
///
/// WebRTC 統計情報の JSON 配列をパースし、`data-channel` タイプを持つエントリを探して、
/// そのエントリの `label` フィールドが `expected_label` と一致するかを確認する。
pub fn verify_data_channel_label(stats_json: &JsonString, expected_label: &str) -> bool {
    parse_stats_lossy(stats_json).iter().any(|stat| {
        if let WebRtcStat::DataChannel(data_channel) = stat {
            return data_channel.label.as_deref() == Some(expected_label);
        }
        false
    })
}

/// 統計情報から video outbound-rtp の rid ごとの送信情報を抽出する。
pub fn collect_video_outbound_rid_stats(stats_json: &JsonString) -> Vec<RtcOutboundRtpStreamStats> {
    parse_stats_lossy(stats_json)
        .into_iter()
        .filter_map(|stat| {
            let WebRtcStat::OutboundRtp(outbound) = stat else {
                return None;
            };
            if outbound.kind() != "video" || outbound.rid.is_none() {
                return None;
            }
            Some(outbound)
        })
        .collect()
}

/// しきい値を超える simulcast layer 数を返す。
pub fn count_active_simulcast_layers(
    stats_json: &JsonString,
    min_bytes: u64,
    min_packets: u64,
) -> usize {
    collect_video_outbound_rid_stats(stats_json)
        .into_iter()
        .filter(|s| {
            s.bytes_sent().unwrap_or(0) > min_bytes && s.packets_sent().unwrap_or(0) > min_packets
        })
        .count()
}

/// rid 集合に expected の要素が全て含まれるか確認する。
pub fn has_simulcast_rids(stats_json: &JsonString, expected: &[&str]) -> bool {
    use std::collections::BTreeSet;

    let actual: BTreeSet<String> = collect_video_outbound_rid_stats(stats_json)
        .into_iter()
        .filter_map(|s| s.rid)
        .collect();
    let expected: BTreeSet<String> = expected.iter().map(|rid| (*rid).to_string()).collect();
    expected.is_subset(&actual)
}

pub mod fake_audio_device_module;
pub mod fake_video_capturer;
pub mod stats;
pub mod test_connection;
pub use fake_audio_device_module::{FakeAudioDeviceModule, FakeAudioDeviceModuleConfig};
pub use fake_video_capturer::{FakeVideoCapturer, FakeVideoCapturerConfig};

pub use test_connection::{
    SoraTestConnection, SoraTestConnectionBuilder, SoraTestEvent,
    build_recvonly_data_channel_signaling_connection,
};
