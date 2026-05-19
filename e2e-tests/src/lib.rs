use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_lc_rs::hmac;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nojson::JsonObjectFormatter;
use shiguredo_webrtc::{AudioTrack, VideoTrack};
use sora_sdk::{JsonString, Result, SoraConnectionContext};

use crate::stats::{
    RtcOutboundRtpStreamStats, RtcReceivedRtpStreamStatsTrait, RtcRtpStreamStatsTrait,
    RtcSentRtpStreamStatsTrait, RtcStatsTrait, WebRtcStat, WebRtcStatsReport,
};

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
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(&payload),
    );

    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let signature = hmac::sign(&key, signing_input.as_bytes());

    format!(
        "{}.{}",
        signing_input,
        URL_SAFE_NO_PAD.encode(signature.as_ref())
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

/// OpenH264 動的ライブラリのパスを取得する (設定されている場合)。
pub fn openh264_path() -> Option<String> {
    env::var("OPENH264_PATH").ok()
}

pub async fn wait_task_finished(task: tokio::task::JoinHandle<()>, name: &str) {
    let joined = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap_or_else(|_| panic!("{name} did not finish within timeout"));
    joined.unwrap_or_else(|err| panic!("{name} panicked: {err}"));
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

/// 統計情報から video RTP の codec が期待する mimeType か確認する。
pub fn verify_video_codec_mime_type(
    stats_json: &JsonString,
    stat_type: &str,
    expected_mime_type: &str,
) -> bool {
    use std::collections::HashSet;

    let stats = parse_stats_lossy(stats_json);
    let mut expected_codec_ids = HashSet::new();
    let mut video_codec_ids = Vec::new();

    for stat in &stats {
        match stat {
            WebRtcStat::Codec(codec)
                if codec.mime_type.eq_ignore_ascii_case(expected_mime_type) =>
            {
                expected_codec_ids.insert(codec.id());
            }
            WebRtcStat::InboundRtp(inbound) if stat_type == "inbound-rtp" => {
                if inbound.kind() == "video"
                    && let Some(codec_id) = inbound.codec_id()
                {
                    video_codec_ids.push(codec_id);
                }
            }
            WebRtcStat::OutboundRtp(outbound) if stat_type == "outbound-rtp" => {
                if outbound.kind() == "video"
                    && let Some(codec_id) = outbound.codec_id()
                {
                    video_codec_ids.push(codec_id);
                }
            }
            _ => {}
        }
    }

    if expected_codec_ids.is_empty() {
        return false;
    }
    video_codec_ids
        .iter()
        .any(|codec_id| expected_codec_ids.contains(codec_id))
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

pub mod fake_video_capturer;
pub mod stats;
pub mod test_connection;
pub use fake_video_capturer::{FakeVideoCapturer, FakeVideoCapturerConfig};
pub use test_connection::{SoraTestConnection, SoraTestConnectionBuilder, SoraTestEvent};
