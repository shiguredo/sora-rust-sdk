//! AV1 トラックの SDK 固有ポリシー層。
//!
//! OBU / Sequence Header / Frame Header の汎用解析は `shiguredo_mp4::bitstream::av1` に
//! 寄せ、本モジュールでは AV1CodecConfigurationRecord の抽出情報、track 検証、passthrough
//! payload の再構成、AV1 SDP parameter の解決という SDK 固有のロジックを提供する。
//! track 検証の失敗は `crate::video_codecs::mp4::Mp4Error::InvalidAv1Track(String)` に
//! メッセージで報告する。

use shiguredo_mp4::bitstream::av1::{
    Av1FrameHeaderPrefix, Av1FrameType, Av1ObuParseContext, Av1ObuType, Av1SequenceHeader,
    parse_frame_header_prefix, parse_obus, parse_sequence_header,
};
use shiguredo_webrtc::{SdpVideoFormat, SdpVideoFormatRef, VideoCodecType};

use crate::video_codecs::mp4::Mp4Error;

/// `Mp4Error` をエラー型に持つ [`std::result::Result`]。
pub(super) type Result<T> = std::result::Result<T, Mp4Error>;

/// AV1CodecConfigurationRecord (`av1C`) の抽出情報。
///
/// `shiguredo_mp4::Av1cBox` の bitfield を比較しやすい素朴な型へ展開したもの。
/// sample entry 一貫性検証と Sequence Header との field 比較に使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Av1TrackConfig {
    /// seq_profile (3 bit): 0..=2。
    pub(super) seq_profile: u8,
    /// seq_level_idx[0] (5 bit): 0..=31。
    pub(super) seq_level_idx_0: u8,
    /// seq_tier[0] (1 bit): 0 または 1。
    pub(super) seq_tier_0: u8,
    /// high_bitdepth (1 bit)。
    pub(super) high_bitdepth: bool,
    /// twelve_bit (1 bit): seq_profile == 2 かつ high_bitdepth のときのみ意味を持つ。
    pub(super) twelve_bit: bool,
    /// monochrome (1 bit)。
    pub(super) monochrome: bool,
    /// chroma_subsampling_x (1 bit): 0 または 1。
    pub(super) chroma_subsampling_x: u8,
    /// chroma_subsampling_y (1 bit): 0 または 1。
    pub(super) chroma_subsampling_y: u8,
    /// chroma_sample_position (2 bit): 0..=2。予約値 3 は mp4-rs の
    /// `parse_sequence_header` が拒否するため、ここには現れない。
    pub(super) chroma_sample_position: u8,
    /// initial_presentation_delay_minus_one (4 bit): 存在するときのみ Some。
    pub(super) initial_presentation_delay_minus_one: Option<u8>,
    /// configOBUs 全 byte（AV1CodecConfigurationRecord の末尾に格納される OBU 列）。
    pub(super) config_obus: Vec<u8>,
}

/// `Av1FrameType` をエラーメッセージに埋め込むための短い名前に変換する。
pub(super) fn av1_frame_type_name(t: Av1FrameType) -> &'static str {
    match t {
        Av1FrameType::Key => "key_frame",
        Av1FrameType::Inter => "inter_frame",
        Av1FrameType::IntraOnly => "intra_only_frame",
        Av1FrameType::Switch => "switch_frame",
    }
}

/// `Av1ObuType` をエラーメッセージに埋め込むための短い名前に変換する。
pub(super) fn av1_obu_type_name(t: Av1ObuType) -> &'static str {
    match t {
        Av1ObuType::Reserved(_) => "reserved",
        Av1ObuType::SequenceHeader => "sequence_header",
        Av1ObuType::TemporalDelimiter => "temporal_delimiter",
        Av1ObuType::FrameHeader => "frame_header",
        Av1ObuType::TileGroup => "tile_group",
        Av1ObuType::Metadata => "metadata",
        Av1ObuType::Frame => "frame",
        Av1ObuType::RedundantFrameHeader => "redundant_frame_header",
        Av1ObuType::TileList => "tile_list",
        Av1ObuType::Padding => "padding",
    }
}

/// Sequence Header と AV1CodecConfigurationRecord の対応 field が一致するか判定する。
///
/// AV1 spec Section 5.5.1 / AV1 Codec ISO Media File Format Binding v1.3.0 Section 2.3.4 に基づき、
/// 対応関係が定義されている 9 field を比較する。
fn sequence_header_matches_av1c(sh: &Av1SequenceHeader, av1c: &Av1TrackConfig) -> bool {
    sh.seq_profile == av1c.seq_profile
        && sh.seq_level_idx_0 == av1c.seq_level_idx_0
        && sh.seq_tier_0 == av1c.seq_tier_0
        && sh.high_bitdepth == av1c.high_bitdepth
        && sh.twelve_bit == av1c.twelve_bit
        && sh.monochrome == av1c.monochrome
        && sh.chroma_subsampling_x == av1c.chroma_subsampling_x
        && sh.chroma_subsampling_y == av1c.chroma_subsampling_y
        && sh.chroma_sample_position == av1c.chroma_sample_position
}

/// Sequence Header が本 SDK のポリシー（単一 operating point かつ `operating_point_idc[0] == 0`）
/// を満たすか判定する。
///
/// 固定 packetizer は operating point の選択・除去を行わないため、複数 operating point や
/// 非 0 の `operating_point_idc[0]` を持つ bitstream は、av1C が直接保持する operating point 0
/// の値で SDP 能力を過小広告してしまう。
/// この判定は本 SDK のポリシーであり、mp4-rs の汎用 parser には含めない
/// （mp4-rs 側は複数 operating point を汎用解析して field を公開するだけ）。
pub(super) fn has_supported_operating_points(seq: &Av1SequenceHeader) -> bool {
    seq.operating_points_cnt_minus_1 == 0 && seq.operating_point_idc_0 == 0
}

/// AV1 track の bitstream / configOBUs 一貫性を検証する。
///
/// `is_keyframes[i]` は sample index i が sync sample かどうか。
/// sample bytes は `read_sample` closure で毎回取得する。
/// 呼び出し側で `Mp4VideoTrackInfo::codec_type == Av1` かつ `av1_config` が `Some` の場合のみ
/// 呼ぶ前提。
///
/// 検証項目:
/// - configOBUs の OBU 列と Sequence Header (存在時) の bit-level 解析
/// - configOBUs の Sequence Header が最大 1 個で、存在する場合は先頭 OBU である
/// - configOBUs の Sequence Header field が AV1CodecConfigurationRecord と一致
/// - sync sample が 1 件以上存在し、先頭 sample が sync sample である
/// - 各 sync sample が Frame Header / Frame OBU を含み、その最初のものが
///   `show_existing_frame == 0` かつ `frame_type == KEY_FRAME` かつ `show_frame == 1`
/// - 各 sync sample が Sequence Header OBU を含み、最初の Frame Header/Frame より前に現れる
/// - 全 sample 内の Sequence Header field が AV1CodecConfigurationRecord と一致
/// - 全 Sequence Header が単一 operating point かつ `operating_point_idc[0] == 0`
/// - 同一 coded video sequence 内の Sequence Header payload が byte-for-byte 一致
///   (configOBUs に SH があれば全 sample が config の payload と一致、
///   なければ各 sync sample の最初の SH を per-CVS 基準にする)
/// - RTP 送信対象の最初の OBU (configOBUs || sample data の Temporal Delimiter / Padding を
///   飛ばした先頭) が Sequence Header である
///
/// 失敗はすべて [`Mp4Error::InvalidAv1Track`] に文脈入りのメッセージで報告する。
pub(super) fn validate_av1_track(
    is_keyframes: &[bool],
    av1_config: &Av1TrackConfig,
    mut read_sample: impl FnMut(usize) -> Result<Vec<u8>>,
) -> Result<()> {
    // 1. configOBUs を parse し、Sequence Header (存在時) の payload / fields を確定する。
    let config_obus =
        parse_obus(&av1_config.config_obus, Av1ObuParseContext::ConfigObus).map_err(|e| {
            Mp4Error::InvalidAv1Track(format!("configOBUs の OBU 解析に失敗しました: {e:?}"))
        })?;

    // configOBUs の Sequence Header は最大 1 個で、存在する場合は先頭 OBU でなければならない。
    // mp4-rs の parse_obus はこの制約を検証しないため、返却された OBU 列から SDK 側で確認する。
    // 先頭以外の SH は「先頭でない」を、先頭の SH がもう 1 個ある場合は「複数」を優先して報告する。
    let config_sh_indices: Vec<usize> = config_obus
        .iter()
        .enumerate()
        .filter(|(_, obu)| obu.obu_type == Av1ObuType::SequenceHeader)
        .map(|(i, _)| i)
        .collect();
    match config_sh_indices.as_slice() {
        [] => {}
        [first] => {
            if *first != 0 {
                return Err(Mp4Error::InvalidAv1Track(format!(
                    "configOBUs の Sequence Header OBU が先頭ではありません: sh_index={first}"
                )));
            }
        }
        [first, ..] => {
            if *first != 0 {
                return Err(Mp4Error::InvalidAv1Track(format!(
                    "configOBUs の Sequence Header OBU が先頭ではありません: sh_index={first}"
                )));
            }
            return Err(Mp4Error::InvalidAv1Track(
                "configOBUs に Sequence Header OBU が複数存在します".to_string(),
            ));
        }
    }

    // configOBUs の Sequence Header (存在時) の payload / fields を確定する。
    // 制約チェックにより、SH が存在する場合は先頭 OBU (index 0) のみが残る。
    let config_sh_payload_and_fields = match config_obus
        .first()
        .filter(|o| o.obu_type == Av1ObuType::SequenceHeader)
    {
        Some(obu) => {
            let fields = parse_sequence_header(obu.payload).map_err(|e| {
                Mp4Error::InvalidAv1Track(format!(
                    "configOBUs 内 Sequence Header の解析に失敗しました: {e:?}"
                ))
            })?;
            if !has_supported_operating_points(&fields) {
                return Err(Mp4Error::InvalidAv1Track(format!(
                    "configOBUs 内 Sequence Header が単一 operating point ではありません: operating_points_cnt_minus_1={} operating_point_idc_0={}",
                    fields.operating_points_cnt_minus_1, fields.operating_point_idc_0
                )));
            }
            if !sequence_header_matches_av1c(&fields, av1_config) {
                return Err(Mp4Error::InvalidAv1Track(
                    "configOBUs 内 Sequence Header と av1C が一致しません".to_string(),
                ));
            }
            Some((obu.payload.to_vec(), fields))
        }
        None => None,
    };

    // 2. sync sample 要件を確認する。sample data を読む前に安価に判定できるチェックを先に。
    if !is_keyframes.iter().any(|k| *k) {
        return Err(Mp4Error::InvalidAv1Track(
            "AV1 トラックに sync sample がありません".to_string(),
        ));
    }
    // samples が空なら呼び出し側 (NoVideoSamples) で既に弾かれている前提。
    if !is_keyframes[0] {
        return Err(Mp4Error::InvalidAv1Track(
            "AV1 トラックの先頭 sample が sync sample ではありません".to_string(),
        ));
    }

    // 3. 各 sample を parse し、SH 一貫性・sync sample 条件・RTP 順序を検証する。
    //
    // Sequence Header payload 基準:
    //   - config_has_permanent_sh: config に SH がある。基準は config の SH payload で不変。
    //   - それ以外: 各 sync sample の最初の SH で per-CVS 基準を更新する。
    let config_has_permanent_sh = config_sh_payload_and_fields.is_some();
    let permanent_sh_payload: Option<Vec<u8>> = config_sh_payload_and_fields
        .as_ref()
        .map(|(p, _)| p.clone());
    // Sequence Header の field 比較・Frame Header 解析には
    // reduced_still_picture_header の context が必要。config に SH があれば以降不変、
    // なければ per-CVS で更新する。config にも sample にも SH がない状態で Frame Header に
    // 到達したら error（sync sample であれば「SH がない」条件、non-sync sample であれば
    // 「先頭 sync sample が SH を持つ」invariant により論理的に発生しない）。
    let mut current_sh_context: Option<Av1SequenceHeader> =
        config_sh_payload_and_fields.as_ref().map(|(_, f)| *f);
    let mut cvs_sh_payload: Option<Vec<u8>> = None;

    for (sample_index, is_keyframe) in is_keyframes.iter().enumerate() {
        let bytes = read_sample(sample_index)?;
        let sample_obus = parse_obus(&bytes, Av1ObuParseContext::Sample).map_err(|e| {
            Mp4Error::InvalidAv1Track(format!(
                "sample={sample_index} の OBU 解析に失敗しました: {e:?}"
            ))
        })?;

        // sync sample では per-CVS 基準を「この sync sample の最初の SH」で更新する。
        // config に permanent SH があるときは触らない。
        if *is_keyframe && !config_has_permanent_sh {
            cvs_sh_payload = None;
        }

        let mut first_sh_index: Option<usize> = None;
        let mut first_frame_index: Option<usize> = None;

        for (obu_index, obu) in sample_obus.iter().enumerate() {
            match obu.obu_type {
                Av1ObuType::SequenceHeader => {
                    if first_sh_index.is_none() {
                        first_sh_index = Some(obu_index);
                    }
                    let fields = parse_sequence_header(obu.payload).map_err(|e| {
                        Mp4Error::InvalidAv1Track(format!(
                            "sample={sample_index} 内 Sequence Header の解析に失敗しました: {e:?}"
                        ))
                    })?;
                    if !has_supported_operating_points(&fields) {
                        return Err(Mp4Error::InvalidAv1Track(format!(
                            "sample={sample_index} 内 Sequence Header が単一 operating point ではありません: operating_points_cnt_minus_1={} operating_point_idc_0={}",
                            fields.operating_points_cnt_minus_1, fields.operating_point_idc_0
                        )));
                    }
                    if !sequence_header_matches_av1c(&fields, av1_config) {
                        return Err(Mp4Error::InvalidAv1Track(format!(
                            "sample={sample_index} 内 Sequence Header と av1C が一致しません"
                        )));
                    }
                    // payload 一貫性: config permanent 優先、なければ per-CVS。
                    let baseline: Option<&[u8]> = if config_has_permanent_sh {
                        permanent_sh_payload.as_deref()
                    } else {
                        cvs_sh_payload.as_deref()
                    };
                    match baseline {
                        Some(b) => {
                            if obu.payload != b {
                                return Err(Mp4Error::InvalidAv1Track(format!(
                                    "sample={sample_index} 内 Sequence Header payload が同一 coded video sequence 内の基準と一致しません"
                                )));
                            }
                        }
                        None => {
                            // per-CVS 基準がまだ設定されていない場合のみ。
                            // sync sample の最初の SH を新しい基準にする。
                            cvs_sh_payload = Some(obu.payload.to_vec());
                        }
                    }
                    current_sh_context = Some(fields);
                }
                Av1ObuType::FrameHeader | Av1ObuType::Frame if first_frame_index.is_none() => {
                    first_frame_index = Some(obu_index);
                    if *is_keyframe && let Some(sh) = current_sh_context {
                        // sync sample の最初の Frame Header/Frame OBU について
                        // uncompressed_header 冒頭を parse し、KEY_FRAME 条件を検証する。
                        // context がまだ None (= config にも先行 sample にも SH がない) なら、
                        // 以下の sync sample post-checks で「SH がない」として弾く。
                        // Frame 系 OBU 自体が無い sync sample も post-checks で拒否する。
                        let prefix = parse_frame_header_prefix(obu.payload, &sh).map_err(|e| {
                            Mp4Error::InvalidAv1Track(format!(
                                "sample={sample_index} 内 Frame Header の解析に失敗しました: {e:?}"
                            ))
                        })?;
                        match prefix {
                            Av1FrameHeaderPrefix::ShowExistingFrame => {
                                return Err(Mp4Error::InvalidAv1Track(format!(
                                    "sync sample の show_existing_frame が 1 です: sample={sample_index}"
                                )));
                            }
                            Av1FrameHeaderPrefix::NewFrame {
                                frame_type,
                                show_frame,
                            } => {
                                if frame_type != Av1FrameType::Key {
                                    return Err(Mp4Error::InvalidAv1Track(format!(
                                        "sync sample の frame_type が KEY_FRAME ではありません: sample={sample_index} frame_type={}",
                                        av1_frame_type_name(frame_type)
                                    )));
                                }
                                if !show_frame {
                                    return Err(Mp4Error::InvalidAv1Track(format!(
                                        "sync sample の show_frame が 0 です: sample={sample_index}"
                                    )));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if *is_keyframe {
            let sh_index = first_sh_index.ok_or_else(|| {
                Mp4Error::InvalidAv1Track(format!(
                    "sync sample が Sequence Header OBU を含みません: sample={sample_index}"
                ))
            })?;
            // Binding v1.3.0 Section 2.4: sync sample は RAP であり、最初の frame が
            // Key Frame かつ show_frame=1。Frame Header / Frame が無いと RAP を検証できない。
            let fr_index = first_frame_index.ok_or_else(|| {
                Mp4Error::InvalidAv1Track(format!(
                    "sync sample に Frame Header / Frame OBU がありません: sample={sample_index}"
                ))
            })?;
            if sh_index > fr_index {
                return Err(Mp4Error::InvalidAv1Track(format!(
                    "sync sample で Sequence Header が最初の Frame より後に現れます: sample={sample_index}"
                )));
            }

            // RTP packetizer 順序検証: configOBUs || sample の連結列を
            // Temporal Delimiter / Padding を除いてスキャンし、最初の OBU が
            // Sequence Header であることを要求する。
            let first_significant = config_obus
                .iter()
                .map(|o| o.obu_type)
                .chain(sample_obus.iter().map(|o| o.obu_type))
                .find(|t| !matches!(t, Av1ObuType::TemporalDelimiter | Av1ObuType::Padding));
            match first_significant {
                Some(Av1ObuType::SequenceHeader) => {}
                Some(t) => {
                    return Err(Mp4Error::InvalidAv1Track(format!(
                        "RTP 送信対象の最初の OBU が Sequence Header ではありません: sample={sample_index} first_obu_type={}",
                        av1_obu_type_name(t)
                    )));
                }
                None => {
                    // Temporal Delimiter と Padding だけの sync sample は
                    // 直前の SH 欠如で既に「SH がない」条件に落ちる。
                    // ここには到達しない想定だが、defensive に扱う。
                    return Err(Mp4Error::InvalidAv1Track(format!(
                        "sync sample が Sequence Header OBU を含みません: sample={sample_index}"
                    )));
                }
            }
        }
    }

    Ok(())
}

// -----------------------------------------------------------------
// 固定 libwebrtc の AV1 コード source audit
// (m152.7977.0.0 / commit 6f37672d358475cd17544121a12494da454d85fb)
//
// 依存 `shiguredo_webrtc` の libwebrtc を更新した場合は、以下 file / function の
// 挙動を再検証し、本コメントを更新すること。RTP packetization と SDP profile 判定を
// 委ねる境界であり、本 SDK の AV1 payload 生成と SDP 検証はこの挙動に依存する。
// m150 (commit 1f975dfd761af6e5d76d28333191973b258d82a8) との差分確認済み:
// `rtp_packetizer_av1.cc` は同一、`av1_profile.cc` は `params.find(std::string(...))`
// の std::string ラップという 1 行の差分だけで挙動は変わらない。
//
// `modules/rtp_rtcp/source/rtp_packetizer_av1.cc` の `RtpPacketizerAv1::ParseObus`:
// - encoder callback から渡された payload を Low Overhead Bitstream Format の OBU 列として
//   再解析し、各 OBU の size field (`obu_has_size_field == 1`) を RTP OBU element length へ変換する
// - RTP 送出 OBU header からは `obu_has_size_field` bit を除去して転送する
// - Temporal Delimiter (type 2)、Tile List (type 8)、Padding (type 15) は RTP 送信対象から除外する
// - malformed input (forbidden bit != 0、reserved bit != 0、size field 不整合) では
//   RTP packet を生成しない
// - key frame の temporal unit で、除去対象 OBU を除いた先頭 OBU が Sequence Header の場合、
//   aggregation header の N bit を 1 に設定する
// - temporal unit の最終 RTP packet に marker bit を 1 に設定する
//
// `api/video_codecs/av1_profile.cc` の `ParseSdpForAV1Profile` と `AV1IsSameProfile`:
// - `ParseSdpForAV1Profile` は SDP fmtp の `profile` 省略を 0 (Main) と解釈する
// - `AV1IsSameProfile` は profile の一致だけを判定し、level-idx / tier は考慮しない
// - level / tier の上限判定は libwebrtc に委ねず、本 SDK の [`resolve_av1_incoming`] で検証する
//
// 本 SDK は上記挙動に合わせて `Mp4SampleReader::new_inner` で以下を要求する:
// - `configOBUs || sample data` の先頭から Temporal Delimiter / Padding を飛ばした最初の OBU が
//   Sequence Header であること (packetizer が RTP N bit を必ず設定できる保証)
// - Tile List OBU は `parse_obus` が AV1 Codec ISO Media File Format Binding に従い拒否する
// - Metadata OBU が Sequence Header より前にあると拒否 (packetizer が除外せず、順序仕様外)
// - encoder callback は再構成済み `configOBUs || sample data` を byte-for-byte で渡し、
//   RTP aggregation header や OBU element length の生成は packetizer 側に一任する
// -----------------------------------------------------------------

/// AV1 sample の encoder callback payload を組み立てる。
///
/// AV1 Codec ISO Media File Format Binding v1.3.0 Section 2.3.4 に従い、
/// sync sample かつ `configOBUs` が非空なら `configOBUs || sample data` を返す。
/// non-sync sample、または `configOBUs` が空、または `av1_config` が `None` なら
/// sample data をそのまま返す。
///
/// - `configOBUs` は格納順のまま全 byte を付与する（AV1 spec が要求する OBU 順序を保つ）
/// - `configOBUs` と sample の双方に同一 Sequence Header OBU がある場合も
///   両方を格納順のまま残す（deduplicate しない）
/// - RTP packetizer 順序の検証は [`validate_av1_track`] で reader 初期化時に済ませているため、
///   ここでは byte 列の結合だけを行う
pub(super) fn assemble_av1_encoded_sample_data(
    raw_sample: Vec<u8>,
    is_keyframe: bool,
    av1_config: Option<&Av1TrackConfig>,
) -> Vec<u8> {
    if is_keyframe
        && let Some(config) = av1_config
        && !config.config_obus.is_empty()
    {
        let mut combined = Vec::with_capacity(config.config_obus.len() + raw_sample.len());
        combined.extend_from_slice(&config.config_obus);
        combined.extend_from_slice(&raw_sample);
        combined
    } else {
        raw_sample
    }
}

/// AV1 SDP parameter (`profile` / `level-idx` / `tier`) の 1 個を parse する。
///
/// 挙動:
/// - `params` に該当 key が **存在しない** (`omitted`) 場合は `default` を返す
/// - 該当 key が **存在するが空文字列** の場合は非 10 進として拒否 (`None`)
/// - 該当 key が **存在して非空** の場合、ASCII 10 進整数として parse する
///   - `u8::from_str` を使うため、負数・非 10 進・overflow はすべて拒否
///   - parse 結果が `max` を超える場合も拒否
///
/// AV1 RTP Payload Format Section 7.2 が定義する省略値は
/// `profile` = 0、`level-idx` = 5、`tier` = 0。
pub(super) fn parse_av1_sdp_field(
    params: &std::collections::HashMap<String, String>,
    key: &str,
    default: u8,
    max: u8,
) -> Option<u8> {
    let Some(raw) = params.get(key) else {
        return Some(default);
    };
    let parsed: u8 = raw.parse().ok()?;
    if parsed > max { None } else { Some(parsed) }
}

/// AV1 の incoming SDP format を required に照らして解決する。
///
/// 判定:
/// - codec 名が `AV1` でなければ拒否
/// - incoming と required の `profile` / `level-idx` / `tier` を [`parse_av1_sdp_field`] で parse
///   (`profile` 0..=2、`level-idx` 0..=31、`tier` 0..=1、省略値 0/5/0)
/// - 固定 libwebrtc の `AV1IsSameProfile` に合わせ、`profile` は完全一致を要求
/// - `level-idx` / `tier` は required (bitstream 実値) が incoming (receiving capability) 以下
///
/// 通過時は required をそのまま返す (offer 段の profile/level/tier は sender の宣言なので
/// bitstream 実値を上書きしない)。
pub(super) fn resolve_av1_incoming(
    required: &SdpVideoFormat,
    mut incoming: SdpVideoFormatRef<'_>,
) -> Option<SdpVideoFormat> {
    // codec 名の一致 (AV1)
    let name = incoming.name().ok()?;
    if VideoCodecType::try_from(name.as_str()).ok()? != VideoCodecType::Av1 {
        return None;
    }

    let incoming_params: std::collections::HashMap<String, String> =
        incoming.parameters_mut().iter().collect();
    let incoming_profile = parse_av1_sdp_field(&incoming_params, "profile", 0, 2)?;
    let incoming_level = parse_av1_sdp_field(&incoming_params, "level-idx", 5, 31)?;
    let incoming_tier = parse_av1_sdp_field(&incoming_params, "tier", 0, 1)?;

    // required も同じ helper で parse する。実装 invariant として
    // av1_required_sdp_format が 3 field を av1c 由来の 10 進文字列で必ず設定するが、
    // 万一未設定でも省略値経路が動くよう共通 parse を通す。
    let mut required_owned = required.clone();
    let required_params: std::collections::HashMap<String, String> =
        required_owned.parameters_mut().iter().collect();
    let required_profile = parse_av1_sdp_field(&required_params, "profile", 0, 2)?;
    let required_level = parse_av1_sdp_field(&required_params, "level-idx", 5, 31)?;
    let required_tier = parse_av1_sdp_field(&required_params, "tier", 0, 1)?;

    if required_profile != incoming_profile {
        return None;
    }
    if required_level > incoming_level {
        return None;
    }
    if required_tier > incoming_tier {
        return None;
    }
    Some(required_owned)
}

/// AV1 track の required [`SdpVideoFormat`] を組み立てる。
///
/// AV1CodecConfigurationRecord の `seq_profile` / `seq_level_idx_0` / `seq_tier_0` を
/// 10 進文字列で `profile` / `level-idx` / `tier` parameter として必ず設定する。
/// AV1 RTP Payload Format Section 7.2 では省略値をそれぞれ 0 / 5 / 0 と解釈するが、
/// 本 SDK は AV1CodecConfigurationRecord 由来の値を明示することで、
/// receiver 側 negotiation が省略値との一致で成立するのを防ぐ。
///
/// `av1_config` が `None` の場合（`Mp4VideoTrackInfo::av1_config` 未設定）は
/// parameter を設定せず、生の `AV1` format だけを返す。実運用では
/// `Mp4SampleReader::new_inner` が AV1 track で `av1_config` を必ず `Some` にする
/// invariant を持つため、この経路は到達しない想定。
pub(super) fn av1_required_sdp_format(av1_config: Option<&Av1TrackConfig>) -> SdpVideoFormat {
    let mut format = SdpVideoFormat::new("AV1");
    if let Some(config) = av1_config {
        let mut params = format.parameters_mut();
        params.set("profile", &config.seq_profile.to_string());
        params.set("level-idx", &config.seq_level_idx_0.to_string());
        params.set("tier", &config.seq_tier_0.to_string());
    }
    format
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_mp4::bitstream::av1::parse_sequence_header;

    /// テスト用の基準 `Av1TrackConfig` を返す。
    fn base_config() -> Av1TrackConfig {
        Av1TrackConfig {
            seq_profile: 0,
            seq_level_idx_0: 0,
            seq_tier_0: 0,
            high_bitdepth: false,
            twelve_bit: false,
            monochrome: false,
            chroma_subsampling_x: 1,
            chroma_subsampling_y: 1,
            chroma_sample_position: 0,
            initial_presentation_delay_minus_one: None,
            config_obus: vec![0x0A, 0x0B],
        }
    }

    /// Sequence Header payload から `Av1SequenceHeader` を parse するテストヘルパー。
    fn parse_sh(payload: &[u8]) -> Av1SequenceHeader {
        parse_sequence_header(payload).expect("テスト用 Sequence Header は parse できるはず")
    }

    /// reduced_still_picture_header 経路の Sequence Header payload を組み立てる。
    ///
    /// reduced_still 経路は operating_points / timing_info / order_hint など多数の分岐を
    /// 飛ばせるため、bit 列が短く読みやすい。
    /// mp4-rs の `parse_sequence_header` は chroma_sample_position 以降も
    /// separate_uv_delta_q / film_grain_params_present まで読むため、末尾 2 bit も書く。
    ///
    /// 引数の `seq_profile` と `seq_level_idx_0` はテストで多用する field。
    /// その他 (subsampling / bitdepth 等) は reduced_still + seq_profile=0 の暗黙値と合わせる:
    ///   high_bitdepth=false / monochrome=false / chroma_subsampling=(1,1) /
    ///   chroma_sample_position=0
    fn make_reduced_still_sh_payload(seq_profile: u8, seq_level_idx_0: u8) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bits(seq_profile as u32, 3); // seq_profile
        w.write_bit(true); // still_picture = 1
        w.write_bit(true); // reduced_still_picture_header = 1
        w.write_bits(seq_level_idx_0 as u32, 5); // seq_level_idx[0]
        w.write_bits(0, 4); // frame_width_bits_minus_1
        w.write_bits(0, 4); // frame_height_bits_minus_1
        w.write_bits(0, 1); // max_frame_width_minus_1
        w.write_bits(0, 1); // max_frame_height_minus_1
        w.write_bits(0, 3); // use_128x128_superblock | enable_filter_intra | enable_intra_edge_filter
        w.write_bits(0, 3); // enable_superres | enable_cdef | enable_restoration
        w.write_bit(false); // high_bitdepth
        w.write_bit(false); // monochrome
        w.write_bit(false); // color_description_present_flag
        w.write_bit(false); // color_range
        w.write_bits(0, 2); // chroma_sample_position (CSP_UNKNOWN)
        w.write_bit(false); // separate_uv_delta_q
        w.write_bit(false); // film_grain_params_present
        w.finish()
    }

    /// reduced_still 経路の SH payload を `max_frame_size` を指定して組み立てる。
    ///
    /// max_frame_size は av1C に含まれない field のため、av1C との一致を保ったまま
    /// payload を byte 単位で変えるテスト用。
    fn make_reduced_still_sh_payload_with_dimensions(
        max_w_minus_1: bool,
        max_h_minus_1: bool,
    ) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bits(0, 3); // seq_profile
        w.write_bit(true); // still_picture = 1
        w.write_bit(true); // reduced_still_picture_header = 1
        w.write_bits(0, 5); // seq_level_idx[0]
        w.write_bits(0, 4); // frame_width_bits_minus_1
        w.write_bits(0, 4); // frame_height_bits_minus_1
        w.write_bit(max_w_minus_1); // max_frame_width_minus_1
        w.write_bit(max_h_minus_1); // max_frame_height_minus_1
        w.write_bits(0, 3); // use_128x128_superblock | enable_filter_intra | enable_intra_edge_filter
        w.write_bits(0, 3); // enable_superres | enable_cdef | enable_restoration
        w.write_bit(false); // high_bitdepth
        w.write_bit(false); // monochrome
        w.write_bit(false); // color_description_present_flag
        w.write_bit(false); // color_range
        w.write_bits(0, 2); // chroma_sample_position (CSP_UNKNOWN)
        w.write_bit(false); // separate_uv_delta_q
        w.write_bit(false); // film_grain_params_present
        w.finish()
    }

    /// reduced_still_picture_header = 0 の通常経路の Sequence Header payload を組み立てる。
    ///
    /// operating point の個数 / `operating_point_idc[0]` を指定できる。
    /// それ以外の field は mp4-rs の `parse_sequence_header` が読む最小限の 0 埋めにする。
    fn make_full_sh_payload(
        operating_points_cnt_minus_1: u8,
        operating_point_idc_0: u16,
        seq_level_idx_0: u8,
    ) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bits(0, 3); // seq_profile
        w.write_bit(false); // still_picture
        w.write_bit(false); // reduced_still_picture_header
        w.write_bit(false); // timing_info_present_flag
        w.write_bit(false); // initial_display_delay_present_flag
        w.write_bits(operating_points_cnt_minus_1 as u32, 5); // operating_points_cnt_minus_1
        for i in 0..=operating_points_cnt_minus_1 {
            let idc = if i == 0 { operating_point_idc_0 } else { 0 };
            w.write_bits(idc as u32, 12); // operating_point_idc[i]
            let level = if i == 0 { seq_level_idx_0 } else { 0 };
            w.write_bits(level as u32, 5); // seq_level_idx[i]
            if level > 7 {
                w.write_bit(false); // seq_tier[i]
            }
        }
        w.write_bits(0, 4); // frame_width_bits_minus_1
        w.write_bits(0, 4); // frame_height_bits_minus_1
        w.write_bits(0, 1); // max_frame_width_minus_1
        w.write_bits(0, 1); // max_frame_height_minus_1
        w.write_bit(false); // frame_id_numbers_present_flag
        w.write_bits(0, 3); // use_128x128_superblock | enable_filter_intra | enable_intra_edge_filter
        w.write_bits(0, 5); // enable_interintra_compound | enable_masked_compound |
        // enable_warped_motion | enable_dual_filter | enable_order_hint
        w.write_bit(false); // seq_choose_screen_content_tools
        w.write_bit(false); // seq_force_screen_content_tools
        w.write_bits(0, 3); // enable_superres | enable_cdef | enable_restoration
        w.write_bit(false); // high_bitdepth
        w.write_bit(false); // monochrome
        w.write_bit(false); // color_description_present_flag
        w.write_bit(false); // color_range
        w.write_bits(0, 2); // chroma_sample_position (CSP_UNKNOWN)
        w.write_bit(false); // separate_uv_delta_q
        w.write_bit(false); // film_grain_params_present
        w.finish()
    }

    /// MSB first で bit を書き込む小さな writer。
    struct BitWriter {
        buf: Vec<u8>,
        bit_position: usize,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                buf: Vec::new(),
                bit_position: 0,
            }
        }

        fn write_bit(&mut self, bit: bool) {
            let byte_idx = self.bit_position / 8;
            if byte_idx >= self.buf.len() {
                self.buf.push(0);
            }
            let bit_in_byte = 7 - (self.bit_position % 8);
            if bit {
                self.buf[byte_idx] |= 1 << bit_in_byte;
            }
            self.bit_position += 1;
        }

        fn write_bits(&mut self, value: u32, n: usize) {
            for i in (0..n).rev() {
                self.write_bit(((value >> i) & 1) != 0);
            }
        }

        fn finish(self) -> Vec<u8> {
            self.buf
        }
    }

    /// AV1 の LEB128 で 32bit 値をエンコードする。テストで使う値は常に 128 未満のため 1 byte。
    /// AV1 spec Section 4.10.5 に従い、下位 7bit ずつ push し、最終 byte で continuation を 0 にする。
    fn encode_leb128(mut value: u32) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
        out
    }

    /// obu_has_size_field = 1、extension flag = 0 の最小 OBU を組み立てる。
    /// forbidden bit / reserved 1 bit は常に 0。テストで扱う payload は常に 128 byte 未満。
    fn make_obu(obu_type_bits: u8, payload: &[u8]) -> Vec<u8> {
        assert!(obu_type_bits < 16, "obu_type_bits は 4 bit 範囲");
        let mut buf = Vec::new();
        // header: forbidden(1)=0 | type(4) | ext(1)=0 | has_size(1)=1 | reserved(1)=0
        //   → (type << 3) | 0b010
        buf.push((obu_type_bits << 3) | 0b010);
        buf.extend_from_slice(&encode_leb128(payload.len() as u32));
        buf.extend_from_slice(payload);
        buf
    }

    /// reduced_still 経路の SH に対応する Av1TrackConfig を組み立てる。
    /// SH parser が返す暗黙値 (chroma_subsampling=(1,1) 等) と一致させる。
    fn av1_track_config_for_reduced_still(config_obus: Vec<u8>) -> Av1TrackConfig {
        Av1TrackConfig {
            seq_profile: 0,
            seq_level_idx_0: 0,
            seq_tier_0: 0,
            high_bitdepth: false,
            twelve_bit: false,
            monochrome: false,
            chroma_subsampling_x: 1,
            chroma_subsampling_y: 1,
            chroma_sample_position: 0,
            initial_presentation_delay_minus_one: None,
            config_obus,
        }
    }

    /// validate_av1_track を synthetic な sample bytes で実行するヘルパー。
    /// `entries[i]` は (sample bytes, is_keyframe)。
    fn validate_av1_samples(entries: Vec<(Vec<u8>, bool)>, config: &Av1TrackConfig) -> Result<()> {
        let mut is_keyframes = Vec::new();
        let mut bytes_table = Vec::new();
        for (data, is_keyframe) in entries {
            is_keyframes.push(is_keyframe);
            bytes_table.push(data);
        }
        validate_av1_track(&is_keyframes, config, |i| Ok(bytes_table[i].clone()))
    }

    /// `InvalidAv1Track` エラーを取り出し、メッセージが `expected` を含むことを確認する。
    fn assert_invalid_av1_track(result: &Result<()>, expected: &str) {
        match result {
            Err(Mp4Error::InvalidAv1Track(message)) => {
                assert!(
                    message.contains(expected),
                    "エラーメッセージ {message:?} に {expected:?} が含まれるはずです"
                );
            }
            other => panic!("InvalidAv1Track を期待しましたが: {other:?}"),
        }
    }

    // OBU type の定数。header byte 組み立てで使う 4 bit 値。
    const OBU_TYPE_SEQUENCE_HEADER: u8 = 1;
    const OBU_TYPE_TEMPORAL_DELIMITER: u8 = 2;
    const OBU_TYPE_METADATA: u8 = 5;
    const OBU_TYPE_FRAME: u8 = 6;

    #[test]
    fn sequence_header_matches_av1c_compares_corresponding_fields() {
        let sh = parse_sh(&make_reduced_still_sh_payload(0, 0));
        let av1c = base_config();
        assert!(
            sequence_header_matches_av1c(&sh, &av1c),
            "SH と av1C が一致する場合は true を返すはずです"
        );

        let mut mismatched = av1c;
        mismatched.seq_profile = 1;
        assert!(
            !sequence_header_matches_av1c(&sh, &mismatched),
            "対応 field が食い違う場合は false を返すはずです"
        );
    }

    #[test]
    fn has_supported_operating_points_accepts_single_zero_idc() {
        // reduced_still_picture_header 経路は operating point が暗黙値 (0 / 0)。
        let sh = parse_sh(&make_reduced_still_sh_payload(0, 0));
        assert!(
            has_supported_operating_points(&sh),
            "単一 operating point かつ idc 0 は受理されるはずです"
        );
    }

    /// 1 個の sync sample が「SH + Frame」で構成された最小の有効ケースが受理されることを確認する。
    #[test]
    fn validate_av1_track_accepts_minimal_sync_only() {
        let sh_payload = make_reduced_still_sh_payload(0, 0);
        let mut sample_bytes = Vec::new();
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[])); // reduced_still 経路で payload 不要

        let config = av1_track_config_for_reduced_still(Vec::new());
        let result = validate_av1_samples(vec![(sample_bytes, true)], &config);
        assert!(
            result.is_ok(),
            "最小 sync sample の validate_av1_track は Ok を返すはずですが: {result:?}"
        );
    }

    /// sync sample が 1 件もない track と、先頭 sample が sync sample でない track を拒否する。
    #[test]
    fn validate_av1_track_rejects_random_access_not_possible() {
        let sh_payload = make_reduced_still_sh_payload(0, 0);
        let mut sync_sample = Vec::new();
        sync_sample.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        sync_sample.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));

        // sync sample が 1 件もない track。
        let config = av1_track_config_for_reduced_still(Vec::new());
        let result = validate_av1_samples(vec![(sync_sample.clone(), false)], &config);
        assert_invalid_av1_track(&result, "sync sample がありません");

        // 先頭 sample が sync sample でない track。
        let non_sync_sample = make_obu(OBU_TYPE_FRAME, &[]);
        let result =
            validate_av1_samples(vec![(non_sync_sample, false), (sync_sample, true)], &config);
        assert_invalid_av1_track(&result, "先頭 sample が sync sample ではありません");
    }

    /// sync sample が Sequence Header OBU を含まない場合と、
    /// Sequence Header が最初の Frame より後に現れる場合を拒否する。
    #[test]
    fn validate_av1_track_rejects_sync_sample_without_leading_sequence_header() {
        let sh_payload = make_reduced_still_sh_payload(0, 0);
        let config = av1_track_config_for_reduced_still(Vec::new());

        // Frame OBU のみ (SH なし) の sync sample。
        let sample_bytes = make_obu(OBU_TYPE_FRAME, &[]);
        let result = validate_av1_samples(vec![(sample_bytes, true)], &config);
        assert_invalid_av1_track(&result, "Sequence Header OBU を含みません");

        // Frame 先、SH 後の逆順で並べた sync sample。
        let mut reversed = Vec::new();
        reversed.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));
        reversed.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        let result = validate_av1_samples(vec![(reversed, true)], &config);
        assert_invalid_av1_track(&result, "Sequence Header が最初の Frame より後に現れます");
    }

    /// sync sample に Frame Header / Frame OBU が無いと RAP を検証できないので拒否する。
    #[test]
    fn validate_av1_track_rejects_sync_sample_without_frame() {
        let sh_payload = make_reduced_still_sh_payload(0, 0);
        let sample_bytes = make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload);
        let config = av1_track_config_for_reduced_still(Vec::new());
        let result = validate_av1_samples(vec![(sample_bytes, true)], &config);
        assert_invalid_av1_track(&result, "Frame Header / Frame OBU がありません");
    }

    /// configOBUs || sample の連結で Temporal Delimiter / Padding を飛ばした先頭が
    /// Sequence Header 以外だと拒否する。
    ///
    /// AV1 Codec ISO Media File Format Binding v1.3.0 Section 2.4 は Metadata OBU を
    /// SH より前に置くことも仕様上許容するが、固定 libwebrtc の RtpPacketizerAv1 は
    /// Metadata を除去せず N bit を設定できないため、本 SDK では明示的に拒否する。
    #[test]
    fn validate_av1_track_rejects_metadata_before_sequence_header() {
        let sh_payload = make_reduced_still_sh_payload(0, 0);
        let mut sample_bytes = Vec::new();
        // TD → Metadata → SH → Frame の順。TD は skip されるが Metadata で
        // 「first significant OBU」判定にヒットする。
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_TEMPORAL_DELIMITER, &[]));
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_METADATA, &[]));
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));

        let config = av1_track_config_for_reduced_still(Vec::new());
        let result = validate_av1_samples(vec![(sample_bytes, true)], &config);
        assert_invalid_av1_track(
            &result,
            "RTP 送信対象の最初の OBU が Sequence Header ではありません",
        );
        assert_invalid_av1_track(&result, "metadata");
    }

    /// sample 内 Sequence Header の field が AV1CodecConfigurationRecord と食い違うと拒否する。
    #[test]
    fn validate_av1_track_rejects_sample_sh_mismatch_with_av1c() {
        // sample 側 SH は seq_profile=0, av1c 側は seq_profile=1 に設定する。
        let sh_payload = make_reduced_still_sh_payload(0, 0);
        let mut sample_bytes = Vec::new();
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));

        let mut config = av1_track_config_for_reduced_still(Vec::new());
        config.seq_profile = 1; // 意図的に不一致にする

        let result = validate_av1_samples(vec![(sample_bytes, true)], &config);
        assert_invalid_av1_track(&result, "Sequence Header と av1C が一致しません");
    }

    /// configOBUs の Sequence Header が複数存在する場合と、先頭 OBU でない場合を拒否する。
    #[test]
    fn validate_av1_track_rejects_config_sequence_header_placement() {
        let sh_payload = make_reduced_still_sh_payload(0, 0);
        let mut sample_bytes = Vec::new();
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));

        // configOBUs に SH が 2 個。
        let mut config_obus = Vec::new();
        config_obus.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        config_obus.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        let config = av1_track_config_for_reduced_still(config_obus);
        let result = validate_av1_samples(vec![(sample_bytes.clone(), true)], &config);
        assert_invalid_av1_track(&result, "Sequence Header OBU が複数存在します");

        // configOBUs の SH が先頭でない (TD が先行)。
        let mut config_obus = Vec::new();
        config_obus.extend_from_slice(&make_obu(OBU_TYPE_TEMPORAL_DELIMITER, &[]));
        config_obus.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        let config = av1_track_config_for_reduced_still(config_obus);
        let result = validate_av1_samples(vec![(sample_bytes, true)], &config);
        assert_invalid_av1_track(&result, "Sequence Header OBU が先頭ではありません");
    }

    /// 複数 operating point と非 0 の `operating_point_idc[0]` を拒否する。
    ///
    /// この拒否は本 SDK のポリシーであり、mp4-rs の汎用 parser は複数 operating point を
    /// 汎用解析して field を公開するだけである。
    #[test]
    fn validate_av1_track_rejects_unsupported_operating_points() {
        let sh_payload = make_reduced_still_sh_payload(0, 0);
        let mut sample_bytes = Vec::new();
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_payload));
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));

        // configOBUs の SH が複数 operating point を持つ場合。
        let config_obus = make_obu(OBU_TYPE_SEQUENCE_HEADER, &make_full_sh_payload(1, 0, 0));
        let config = av1_track_config_for_reduced_still(config_obus);
        let result = validate_av1_samples(vec![(sample_bytes.clone(), true)], &config);
        assert_invalid_av1_track(&result, "単一 operating point ではありません");

        // sample 内 SH が非 0 の `operating_point_idc[0]` を持つ場合。
        let mut sample_bytes = Vec::new();
        sample_bytes.extend_from_slice(&make_obu(
            OBU_TYPE_SEQUENCE_HEADER,
            &make_full_sh_payload(0, 0xABC, 0),
        ));
        sample_bytes.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));
        let config = av1_track_config_for_reduced_still(Vec::new());
        let result = validate_av1_samples(vec![(sample_bytes, true)], &config);
        assert_invalid_av1_track(&result, "単一 operating point ではありません");
    }

    /// 同一 coded video sequence 内で Sequence Header payload が食い違うと拒否する。
    ///
    /// per-CVS 基準の判定を確認するため、configOBUs は空にする。
    /// 先頭 sync sample が per-CVS 基準を確定させ、後続 non-sync sample の SH が
    /// 別 payload だと不一致になる。
    /// av1C に含まれない max_frame_size だけを変えて、field 一致を保ったまま
    /// payload を byte 単位で異ならせる。
    #[test]
    fn validate_av1_track_rejects_sh_payload_inconsistent_within_cvs() {
        // sync sample #0: 基準となる SH (max size 1) を含める。
        let sh_a = make_reduced_still_sh_payload_with_dimensions(false, false);
        let mut sample0 = Vec::new();
        sample0.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_a));
        sample0.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));

        // non-sync sample #1: max size が異なる SH を含める。av1C field は一致するが
        // payload は byte 不一致。
        let sh_b = make_reduced_still_sh_payload_with_dimensions(true, false);
        let mut sample1 = Vec::new();
        sample1.extend_from_slice(&make_obu(OBU_TYPE_SEQUENCE_HEADER, &sh_b));
        sample1.extend_from_slice(&make_obu(OBU_TYPE_FRAME, &[]));

        assert_ne!(
            sh_a, sh_b,
            "テスト前提: 2 つの SH payload は byte 単位で異なるはずです"
        );

        let config = av1_track_config_for_reduced_still(Vec::new());
        let result = validate_av1_samples(vec![(sample0, true), (sample1, false)], &config);
        assert_invalid_av1_track(
            &result,
            "Sequence Header payload が同一 coded video sequence 内の基準と一致しません",
        );
    }

    /// sync sample かつ `configOBUs` が非空なら `configOBUs || sample data` を返し、
    /// それ以外 (non-sync / 空 config / config なし) では sample data をそのまま返す。
    #[test]
    fn assemble_av1_encoded_sample_data_prepends_config_obus_for_sync_sample() {
        let config = Av1TrackConfig {
            config_obus: vec![0xAA, 0xBB, 0xCC],
            ..av1_track_config_for_reduced_still(Vec::new())
        };
        let sample = vec![0x11, 0x22];
        let out = assemble_av1_encoded_sample_data(sample.clone(), true, Some(&config));
        assert_eq!(
            out,
            vec![0xAA, 0xBB, 0xCC, 0x11, 0x22],
            "sync sample には configOBUs が sample data の前に付与されるはずです"
        );
    }

    /// `configOBUs` を付与しない条件 (non-sync / 空 config / config なし) では
    /// sample data をそのまま返す（既存 payload を変更しない）。
    #[test]
    fn assemble_av1_encoded_sample_data_leaves_sample_unchanged_without_prepend_conditions() {
        let config = Av1TrackConfig {
            config_obus: vec![0xAA, 0xBB, 0xCC],
            ..av1_track_config_for_reduced_still(Vec::new())
        };
        let sample = vec![0x11, 0x22];

        // non-sync sample。
        let out = assemble_av1_encoded_sample_data(sample.clone(), false, Some(&config));
        assert_eq!(
            out, sample,
            "non-sync sample では configOBUs を付与せず raw sample を返すはずです"
        );

        // 空 configOBUs の sync sample。
        let empty_config = av1_track_config_for_reduced_still(Vec::new());
        assert!(
            empty_config.config_obus.is_empty(),
            "テスト前提: config_obus は空であるはずです"
        );
        let out = assemble_av1_encoded_sample_data(sample.clone(), true, Some(&empty_config));
        assert_eq!(
            out, sample,
            "configOBUs が空なら sync sample でも既存 payload を変更しないはずです"
        );

        // av1_config が None の防御経路。
        let out = assemble_av1_encoded_sample_data(sample.clone(), true, None);
        assert_eq!(
            out, sample,
            "av1_config が None なら sync sample でも既存 payload を変更しないはずです"
        );
    }

    /// AV1 required SDP format は av1_config から profile / level-idx / tier を
    /// 10 進文字列で必ず設定する。
    #[test]
    fn av1_required_sdp_format_sets_profile_level_tier_from_config() {
        let config = Av1TrackConfig {
            seq_profile: 1,
            seq_level_idx_0: 10,
            seq_tier_0: 1,
            ..av1_track_config_for_reduced_still(Vec::new())
        };
        let mut format = av1_required_sdp_format(Some(&config));
        assert_eq!(
            format.name().expect("codec 名を取得できるはず"),
            "AV1",
            "codec 名は AV1 であるはずです"
        );
        let params: std::collections::HashMap<String, String> =
            format.parameters_mut().iter().collect();
        assert_eq!(
            params.get("profile").map(String::as_str),
            Some("1"),
            "profile は av1_config.seq_profile の 10 進文字列であるはずです"
        );
        assert_eq!(
            params.get("level-idx").map(String::as_str),
            Some("10"),
            "level-idx は av1_config.seq_level_idx_0 の 10 進文字列であるはずです"
        );
        assert_eq!(
            params.get("tier").map(String::as_str),
            Some("1"),
            "tier は av1_config.seq_tier_0 の 10 進文字列であるはずです"
        );
    }

    /// 省略値 (profile=0, level-idx=5, tier=0) でも明示的に parameter を設定する。
    /// AV1 RTP Payload Format Section 7.2 の省略値と一致しても、受信側 negotiation の
    /// 曖昧さを避けるため常に明示する。
    #[test]
    fn av1_required_sdp_format_sets_defaults_explicitly() {
        let config = Av1TrackConfig {
            seq_profile: 0,
            seq_level_idx_0: 5,
            seq_tier_0: 0,
            ..av1_track_config_for_reduced_still(Vec::new())
        };
        let mut format = av1_required_sdp_format(Some(&config));
        let params: std::collections::HashMap<String, String> =
            format.parameters_mut().iter().collect();
        assert_eq!(params.get("profile").map(String::as_str), Some("0"));
        assert_eq!(params.get("level-idx").map(String::as_str), Some("5"));
        assert_eq!(params.get("tier").map(String::as_str), Some("0"));
    }

    /// `av1_config` が `None` の防御経路では codec 名だけを返し、parameter は設定しない。
    #[test]
    fn av1_required_sdp_format_omits_parameters_when_config_missing() {
        let mut format = av1_required_sdp_format(None);
        assert_eq!(format.name().expect("codec 名を取得できるはず"), "AV1");
        let params: std::collections::HashMap<String, String> =
            format.parameters_mut().iter().collect();
        assert!(
            params.is_empty(),
            "av1_config が None なら profile / level-idx / tier は設定されないはずです: {params:?}"
        );
    }

    /// AV1 required SDP format に対して同 profile / 上位 level・tier の incoming が受理される。
    #[test]
    fn resolve_av1_incoming_accepts_matching_profile_with_higher_level_and_tier() {
        // required: profile=0, level-idx=5, tier=0
        let config = av1_track_config_for_reduced_still(Vec::new());
        let required = av1_required_sdp_format(Some(&config));

        // incoming: profile=0 (一致)、level-idx=10 (required 5 以上)、tier=0 (required 以上)
        let mut incoming = SdpVideoFormat::new("AV1");
        {
            let mut params = incoming.parameters_mut();
            params.set("profile", "0");
            params.set("level-idx", "10");
            params.set("tier", "0");
        }

        let resolved = resolve_av1_incoming(&required, incoming.as_ref());
        assert!(
            resolved.is_some(),
            "profile 一致かつ level/tier 上限内は受理されるはずです"
        );
    }

    /// profile 完全一致を要求するため、incoming と required の profile が違えば拒否する。
    #[test]
    fn resolve_av1_incoming_rejects_profile_mismatch() {
        let config = av1_track_config_for_reduced_still(Vec::new());
        let required = av1_required_sdp_format(Some(&config));

        let mut incoming = SdpVideoFormat::new("AV1");
        {
            let mut params = incoming.parameters_mut();
            params.set("profile", "1"); // required 0 と不一致
        }

        let resolved = resolve_av1_incoming(&required, incoming.as_ref());
        assert!(resolved.is_none(), "profile 不一致は拒否されるはずです");
    }

    /// required level が incoming level より大きい (受信容量不足) は拒否する。
    #[test]
    fn resolve_av1_incoming_rejects_when_required_level_exceeds_incoming() {
        let config = Av1TrackConfig {
            seq_level_idx_0: 15,
            ..av1_track_config_for_reduced_still(Vec::new())
        };
        let required = av1_required_sdp_format(Some(&config));

        let mut incoming = SdpVideoFormat::new("AV1");
        {
            let mut params = incoming.parameters_mut();
            params.set("profile", "0");
            params.set("level-idx", "10"); // required 15 未満 = 受信容量が足りない
        }

        let resolved = resolve_av1_incoming(&required, incoming.as_ref());
        assert!(
            resolved.is_none(),
            "required level が incoming より大きいなら拒否されるはずですが、Some でした"
        );
    }

    /// required tier が incoming tier より大きい (Main 対 High の逆) は拒否する。
    #[test]
    fn resolve_av1_incoming_rejects_when_required_tier_exceeds_incoming() {
        let config = Av1TrackConfig {
            seq_tier_0: 1, // High tier
            ..av1_track_config_for_reduced_still(Vec::new())
        };
        let required = av1_required_sdp_format(Some(&config));

        let mut incoming = SdpVideoFormat::new("AV1");
        {
            let mut params = incoming.parameters_mut();
            params.set("profile", "0");
            params.set("level-idx", "5");
            params.set("tier", "0"); // required 1 (High) 未満 = 受信容量不足
        }

        let resolved = resolve_av1_incoming(&required, incoming.as_ref());
        assert!(
            resolved.is_none(),
            "required tier が incoming より大きいなら拒否されるはずですが、Some でした"
        );
    }

    /// incoming の profile / level-idx / tier が省略されている場合、
    /// AV1 RTP Payload Format Section 7.2 に従い 0 / 5 / 0 として解釈する。
    #[test]
    fn resolve_av1_incoming_uses_default_values_for_omitted_parameters() {
        // required は省略値と同じ (profile=0, level-idx=5, tier=0)。
        // level-idx を 5 にすることで、incoming 省略を 0 と誤解釈すると
        // required > incoming で拒否される。
        let config = Av1TrackConfig {
            seq_level_idx_0: 5,
            ..av1_track_config_for_reduced_still(Vec::new())
        };
        let required = av1_required_sdp_format(Some(&config));

        // incoming は AV1 だけで parameter は完全省略。
        let incoming = SdpVideoFormat::new("AV1");
        let resolved = resolve_av1_incoming(&required, incoming.as_ref());
        assert!(
            resolved.is_some(),
            "省略値どうしなら受理されるはずですが、None でした"
        );
    }

    /// 範囲外の値 (profile=3、level-idx=32、tier=2) は拒否する。
    #[test]
    fn resolve_av1_incoming_rejects_out_of_range_values() {
        let config = av1_track_config_for_reduced_still(Vec::new());
        let required = av1_required_sdp_format(Some(&config));

        for (key, out_of_range) in [("profile", "3"), ("level-idx", "32"), ("tier", "2")] {
            let mut incoming = SdpVideoFormat::new("AV1");
            {
                let mut params = incoming.parameters_mut();
                params.set(key, out_of_range);
            }
            let resolved = resolve_av1_incoming(&required, incoming.as_ref());
            assert!(
                resolved.is_none(),
                "{key}={out_of_range} は範囲外なので拒否されるはずですが、Some でした"
            );
        }
    }

    /// 非 10 進や負数 (`-1`) や overflow (`256`) は parse 失敗として拒否する。
    #[test]
    fn resolve_av1_incoming_rejects_non_decimal_or_overflow_values() {
        let config = av1_track_config_for_reduced_still(Vec::new());
        let required = av1_required_sdp_format(Some(&config));

        for bad_profile in ["abc", "", "-1", "256", "0x1", "1.0"] {
            let mut incoming = SdpVideoFormat::new("AV1");
            {
                let mut params = incoming.parameters_mut();
                params.set("profile", bad_profile);
            }
            let resolved = resolve_av1_incoming(&required, incoming.as_ref());
            assert!(
                resolved.is_none(),
                "profile={bad_profile:?} は不正 parse として拒否されるはずですが、Some でした"
            );
        }
    }

    /// codec 名が AV1 でない incoming は拒否する。
    #[test]
    fn resolve_av1_incoming_rejects_non_av1_codec() {
        let config = av1_track_config_for_reduced_still(Vec::new());
        let required = av1_required_sdp_format(Some(&config));

        let incoming = SdpVideoFormat::new("VP9");
        let resolved = resolve_av1_incoming(&required, incoming.as_ref());
        assert!(
            resolved.is_none(),
            "codec 名が AV1 でなければ拒否されるはずですが、Some でした"
        );
    }
}
