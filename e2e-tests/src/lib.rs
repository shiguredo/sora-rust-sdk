use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::hmac;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nojson::JsonObjectFormatter;
use shiguredo_webrtc::{AudioTrack, VideoTrack};
use sora_sdk::{JsonString, Result, SoraClientContext};

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
    context: &SoraClientContext,
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

/// 統計情報から指定した type のエントリを検索し、指定したフィールドの値を合計して返す。
///
/// WebRTC 統計情報の JSON 配列をパースし、`stat_type` に一致する type を持つエントリを探して、
/// そのエントリの `field_name` フィールドの値を合算する。
pub fn sum_stats_field_for_type(stats_json: &JsonString, stat_type: &str, field_name: &str) -> u64 {
    use nojson::RawJson;

    let json_str = stats_json.to_string();
    let Ok(json) = RawJson::parse(&json_str) else {
        return 0;
    };
    let value = json.value();
    let Ok(array) = value.to_array() else {
        return 0;
    };

    let mut total = 0;
    for item in array {
        let Ok(type_member) = item.to_member("type") else {
            continue;
        };
        let Some(type_value) = type_member.optional() else {
            continue;
        };
        let type_str: std::result::Result<String, _> = type_value.try_into();
        let Ok(type_str) = type_str else {
            continue;
        };

        if type_str == stat_type {
            let Ok(field_member) = item.to_member(field_name) else {
                continue;
            };
            let Some(field_value) = field_member.optional() else {
                continue;
            };
            let value: std::result::Result<u64, _> = field_value.try_into();
            let Ok(value) = value else {
                continue;
            };
            total += value;
        }
    }
    total
}

/// 統計情報から指定した type のエントリを検索し、指定したフィールドの合計値が 0 より大きいか確認する。
pub fn verify_stats_field_positive(
    stats_json: &JsonString,
    stat_type: &str,
    field_name: &str,
) -> bool {
    sum_stats_field_for_type(stats_json, stat_type, field_name) > 0
}

/// 統計情報から video RTP の codec が期待する mimeType か確認する。
pub fn verify_video_codec_mime_type(
    stats_json: &JsonString,
    stat_type: &str,
    expected_mime_type: &str,
) -> bool {
    use nojson::RawJson;
    use std::collections::HashSet;

    let json_str = stats_json.to_string();
    let Ok(json) = RawJson::parse(&json_str) else {
        return false;
    };
    let value = json.value();
    let Ok(array) = value.to_array() else {
        return false;
    };

    let mut expected_codec_ids = HashSet::new();
    let mut video_codec_ids = Vec::new();

    for item in array {
        let Ok(type_member) = item.to_member("type") else {
            continue;
        };
        let Some(type_value) = type_member.optional() else {
            continue;
        };
        let type_str: std::result::Result<String, _> = type_value.try_into();
        let Ok(type_str) = type_str else {
            continue;
        };

        if type_str == "codec" {
            let mime_type = item
                .to_member("mimeType")
                .ok()
                .and_then(|m| m.optional())
                .and_then(|v| {
                    let value: std::result::Result<String, _> = v.try_into();
                    value.ok()
                });
            let Some(mime_type) = mime_type else {
                continue;
            };
            if !mime_type.eq_ignore_ascii_case(expected_mime_type) {
                continue;
            }
            let codec_id = item
                .to_member("id")
                .ok()
                .and_then(|m| m.optional())
                .and_then(|v| {
                    let value: std::result::Result<String, _> = v.try_into();
                    value.ok()
                });
            if let Some(codec_id) = codec_id {
                expected_codec_ids.insert(codec_id);
            }
            continue;
        }

        if type_str != stat_type {
            continue;
        }

        let kind = item
            .to_member("kind")
            .ok()
            .and_then(|m| m.optional())
            .and_then(|v| {
                let value: std::result::Result<String, _> = v.try_into();
                value.ok()
            });
        if kind.as_deref() != Some("video") {
            continue;
        }

        let codec_id = item
            .to_member("codecId")
            .ok()
            .and_then(|m| m.optional())
            .and_then(|v| {
                let value: std::result::Result<String, _> = v.try_into();
                value.ok()
            });
        if let Some(codec_id) = codec_id {
            video_codec_ids.push(codec_id);
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
    use nojson::RawJson;

    let json_str = stats_json.to_string();
    let Ok(json) = RawJson::parse(&json_str) else {
        return false;
    };
    let value = json.value();
    let Ok(array) = value.to_array() else {
        return false;
    };

    for item in array {
        let Ok(type_member) = item.to_member("type") else {
            continue;
        };
        let Some(type_value) = type_member.optional() else {
            continue;
        };
        let type_str: std::result::Result<String, _> = type_value.try_into();
        let Ok(type_str) = type_str else {
            continue;
        };

        if type_str == "data-channel" {
            let Ok(label_member) = item.to_member("label") else {
                continue;
            };
            let Some(label_value) = label_member.optional() else {
                continue;
            };
            let label_str: std::result::Result<String, _> = label_value.try_into();
            let Ok(label_str) = label_str else {
                continue;
            };
            if label_str == expected_label {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoOutboundRidStat {
    pub rid: String,
    pub bytes_sent: u64,
    pub packets_sent: u64,
}

/// 統計情報から video outbound-rtp の rid ごとの送信情報を抽出する。
pub fn collect_video_outbound_rid_stats(stats_json: &JsonString) -> Vec<VideoOutboundRidStat> {
    use nojson::RawJson;

    let json_str = stats_json.to_string();
    let Ok(json) = RawJson::parse(&json_str) else {
        return Vec::new();
    };
    let value = json.value();
    let Ok(array) = value.to_array() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for item in array {
        let Ok(type_member) = item.to_member("type") else {
            continue;
        };
        let Some(type_value) = type_member.optional() else {
            continue;
        };
        let stat_type: std::result::Result<String, _> = type_value.try_into();
        let Ok(stat_type) = stat_type else {
            continue;
        };
        if stat_type != "outbound-rtp" {
            continue;
        }

        let Ok(kind_member) = item.to_member("kind") else {
            continue;
        };
        let Some(kind_value) = kind_member.optional() else {
            continue;
        };
        let kind: std::result::Result<String, _> = kind_value.try_into();
        let Ok(kind) = kind else {
            continue;
        };
        if kind != "video" {
            continue;
        }

        let Ok(rid_member) = item.to_member("rid") else {
            continue;
        };
        let Some(rid_value) = rid_member.optional() else {
            continue;
        };
        let rid: std::result::Result<String, _> = rid_value.try_into();
        let Ok(rid) = rid else {
            continue;
        };

        let bytes_sent = item
            .to_member("bytesSent")
            .ok()
            .and_then(|m| m.optional())
            .and_then(|v| {
                let n: std::result::Result<u64, _> = v.try_into();
                n.ok()
            })
            .unwrap_or(0);

        let packets_sent = item
            .to_member("packetsSent")
            .ok()
            .and_then(|m| m.optional())
            .and_then(|v| {
                let n: std::result::Result<u64, _> = v.try_into();
                n.ok()
            })
            .unwrap_or(0);

        out.push(VideoOutboundRidStat {
            rid,
            bytes_sent,
            packets_sent,
        });
    }
    out
}

/// しきい値を超える simulcast layer 数を返す。
pub fn count_active_simulcast_layers(
    stats_json: &JsonString,
    min_bytes: u64,
    min_packets: u64,
) -> usize {
    collect_video_outbound_rid_stats(stats_json)
        .into_iter()
        .filter(|s| s.bytes_sent > min_bytes && s.packets_sent > min_packets)
        .count()
}

/// rid 集合に expected の要素が全て含まれるか確認する。
pub fn has_simulcast_rids(stats_json: &JsonString, expected: &[&str]) -> bool {
    use std::collections::BTreeSet;

    let actual: BTreeSet<String> = collect_video_outbound_rid_stats(stats_json)
        .into_iter()
        .map(|s| s.rid)
        .collect();
    let expected: BTreeSet<String> = expected.iter().map(|rid| (*rid).to_string()).collect();
    expected.is_subset(&actual)
}

pub mod fake_video_capturer;
pub use fake_video_capturer::{FakeVideoCapturer, FakeVideoCapturerConfig};
