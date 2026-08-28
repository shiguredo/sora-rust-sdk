//! H.264 の profile-level-id を固定 libwebrtc と同じ規則で解釈する SDK 固有ポリシー層。
//!
//! 6 桁 hex との可逆変換は `shiguredo_mp4::bitstream::h264::H264ProfileLevelId` に
//! 寄せ、本モジュールでは固定 libwebrtc の `h264_profile_level_id.cc` に相当する
//! sub-profile / level の判定、avcC 由来の抽出情報、passthrough 用の SDP format 解決を
//! 提供する。
//! 本モジュールの判定（[`parse_profile_level_id`] / [`resolve_h264_incoming`]）は
//! 失敗を `Option` で表し、エラー variant は作らない。
//! reader 初期化時の track 検証は `crate::video_codecs::mp4` の `extract_track_info` が
//! `Mp4Error::InvalidH264Track(String)` で報告する（av1.rs が自モジュールで track 検証を
//! 行うのと役割分担が異なる）。

use shiguredo_mp4::bitstream::h264::H264ProfileLevelId;
use shiguredo_webrtc::{SdpVideoFormat, SdpVideoFormatRef, VideoCodecType};

/// H.264 の avcC から抽出した、SDP 広告と sample entry 一貫性検証に使う情報。
///
/// `profile_level_id` は reader 初期化時に全 SPS との一致と固定 libwebrtc との
/// 互換性を検証済みである。
/// `avcc_box` は `Mp4VideoTrackInfo::parameter_sets`（SPS / PPS の Annex B 化）では
/// 担保できない `avcC` header や `length_size_minus_one` を含む box 全体の一致を
/// 確認するためのものである。
/// ISO/IEC 14496-15 に違反するが実在する chroma 拡張欠落の avcC は mp4-rs が
/// decode で許容し、re-encode できないため `avcc_box` は `None` になる。
/// この場合も header / SPS / PPS / `length_size_minus_one` は他の field で一致検証される。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct H264TrackConfig {
    /// 検証済みの profile-level-id。
    pub(super) profile_level_id: H264ProfileLevelId,
    /// avcC box 全体の byte 列（再エンコード結果）。再エンコード不能な場合は `None`。
    pub(super) avcc_box: Option<Vec<u8>>,
}

/// 固定 libwebrtc が認識する H.264 sub-profile。
///
/// RFC 6184 Section 8.1 Table 5 は 12 profile を列挙する。固定 libwebrtc の
/// `kProfilePatterns` が認識するのはそのうち Constrained Baseline / Baseline /
/// Main / High / Predictive High 4:4:4 の 5 つに、 Table 5 に行が無い
/// Constrained High を加えた 6 profile である。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H264SubProfile {
    ConstrainedBaseline,
    Baseline,
    Main,
    High,
    /// Constrained High（RFC 6184 Table 5 には行が無いが、固定 libwebrtc が認識する）
    ConstrainedHigh,
    /// High 4:4:4 Predictive（固定 libwebrtc は `kProfilePredictiveHigh444` と名付ける）
    PredictiveHigh444,
}

/// 固定 libwebrtc が認識する H.264 level。
///
/// ITU-T H.264 Annex A Table A-1 のうち、固定 libwebrtc の `H264Level` enum が
/// 持つ level だけを持つ。Level 6 / 6.1 / 6.2 は含まれない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H264Level {
    Level1b,
    Level1,
    Level1_1,
    Level1_2,
    Level1_3,
    Level2,
    Level2_1,
    Level2_2,
    Level3,
    Level3_1,
    Level3_2,
    Level4,
    Level4_1,
    Level4_2,
    Level5,
    Level5_1,
    Level5_2,
}

impl H264Level {
    /// level の能力順での大小比較用の順序値。
    ///
    /// 能力順は ITU-T H.264 Table A-1 の Level 1b が Level 1 と同 FS / MBPS のまま
    /// MaxBR を 2 倍（128 kbit/s）に引き上げていることに基づき、
    /// Level 1 < Level 1b < Level 1.1 < ... < Level 5.2 とする。
    /// 固定 libwebrtc の `kLevelConstraints` 配列も Level 1 → Level 1b → Level 1.1 の順に並ぶ。
    /// 固定 libwebrtc の `H264Level` enum 値（kLevel1_b=0, kLevel1=10, kLevel1_1=11, ...）は
    /// level_idc 由来の定数であり、能力順ではない（kLevel1_b=0 は Level 1 より小さい）。
    fn capability_order(self) -> u8 {
        match self {
            Self::Level1 => 1,
            Self::Level1b => 2,
            Self::Level1_1 => 3,
            Self::Level1_2 => 4,
            Self::Level1_3 => 5,
            Self::Level2 => 6,
            Self::Level2_1 => 7,
            Self::Level2_2 => 8,
            Self::Level3 => 9,
            Self::Level3_1 => 10,
            Self::Level3_2 => 11,
            Self::Level4 => 12,
            Self::Level4_1 => 13,
            Self::Level4_2 => 14,
            Self::Level5 => 15,
            Self::Level5_1 => 16,
            Self::Level5_2 => 17,
        }
    }
}

/// sub-profile と level の正規化結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct H264ProfileLevel {
    profile: H264SubProfile,
    level: H264Level,
}

/// 固定 libwebrtc の `kProfilePatterns` の 1 行。
///
/// `mask` は profile-iop の固定ビット位置、`value` はその固定ビットの期待値。
/// 固定 libwebrtc の `BitPattern` 表記（"x1xx0000" など）の `x`（どちらでもよい）は
/// mask から外れる。
struct H264ProfilePattern {
    profile: H264SubProfile,
    profile_idc: u8,
    mask: u8,
    value: u8,
}

/// 固定 libwebrtc の `kProfilePatterns` 相当。
///
/// RFC 6184 Section 8.1 Table 5 の (profile_idc, profile-iop) 組み合わせのうち、
/// 固定 libwebrtc が認識する 8 行 (Constrained Baseline 3 / Baseline 2 / Main /
/// High / Predictive High 4:4:4) に、 Table 5 に無い Constrained High
/// (profile_idc 64、 profile-iop 00001100) を加えた 9 行を mask / value で表現する。
/// Extended / High 10 / High 4:2:2 / 各 Intra / CAVLC 4:4:4 Intra など、 Table 5 の
/// 他の profile はここに含まれず unsupported になる。
/// 全パターンが profile-iop の下位 2 bit（reserved_zero_2bits）に 0 を要求するため、
/// 非 0 の組み合わせも自然に拒否される。
const H264_PROFILE_PATTERNS: &[H264ProfilePattern] = &[
    // CB: profile_idc 42 (66) + x1xx0000
    H264ProfilePattern {
        profile: H264SubProfile::ConstrainedBaseline,
        profile_idc: 0x42,
        mask: 0b0100_1111,
        value: 0b0100_0000,
    },
    // CB: profile_idc 4D (77) + 1xxx0000
    H264ProfilePattern {
        profile: H264SubProfile::ConstrainedBaseline,
        profile_idc: 0x4D,
        mask: 0b1000_1111,
        value: 0b1000_0000,
    },
    // CB: profile_idc 58 (88) + 11xx0000
    H264ProfilePattern {
        profile: H264SubProfile::ConstrainedBaseline,
        profile_idc: 0x58,
        mask: 0b1100_1111,
        value: 0b1100_0000,
    },
    // B: profile_idc 42 (66) + x0xx0000
    H264ProfilePattern {
        profile: H264SubProfile::Baseline,
        profile_idc: 0x42,
        mask: 0b0100_1111,
        value: 0,
    },
    // B: profile_idc 58 (88) + 10xx0000
    H264ProfilePattern {
        profile: H264SubProfile::Baseline,
        profile_idc: 0x58,
        mask: 0b1100_1111,
        value: 0b1000_0000,
    },
    // M: profile_idc 4D (77) + 0x0x0000
    H264ProfilePattern {
        profile: H264SubProfile::Main,
        profile_idc: 0x4D,
        mask: 0b1010_1111,
        value: 0,
    },
    // H: profile_idc 64 (100) + 00000000
    H264ProfilePattern {
        profile: H264SubProfile::High,
        profile_idc: 0x64,
        mask: 0xFF,
        value: 0,
    },
    // Constrained High: profile_idc 64 (100) + 00001100
    H264ProfilePattern {
        profile: H264SubProfile::ConstrainedHigh,
        profile_idc: 0x64,
        mask: 0xFF,
        value: 0x0C,
    },
    // Predictive High 4:4:4: profile_idc F4 (244) + 00000000
    H264ProfilePattern {
        profile: H264SubProfile::PredictiveHigh444,
        profile_idc: 0xF4,
        mask: 0xFF,
        value: 0,
    },
];

/// 固定 libwebrtc の `ParseH264ProfileLevelId` 相当で profile-level-id を判定する。
///
/// 判定:
/// - level は `level_idc` を固定 libwebrtc の `H264Level` へ写す。
///   `level_idc == 11` かつ `constraint_set3_flag == 1` の表現だけを Level 1b とし、
///   RFC 6184 Section 8.2.2 が informative note で言及する `level_idc == 9` の
///   Level 1b 表現は固定 parser が認識せず PeerConnection negotiation を成立させられない
///   ため unsupported とする。
///   Level 6 / 6.1 / 6.2（`level_idc` 60 / 61 / 62）も固定 libwebrtc の `H264Level`
///   enum に無いため unsupported とする
/// - sub-profile は `H264_PROFILE_PATTERNS` の mask / value で判定する。
///   固定 libwebrtc の `H264IsSameProfile` は双方の parse 成功を要求するため、
///   `kProfilePatterns` に一致しない profile / constraint の組み合わせは
///   required と incoming が byte-for-byte 一致しても unsupported とする
///
/// 根拠: 固定 libwebrtc (m152.7977.0.0 / commit 6f37672d358475cd17544121a12494da454d85fb) の
/// `api/video_codecs/h264_profile_level_id.cc` の `ParseH264ProfileLevelId` /
/// `kProfilePatterns` / `H264IsSameProfile`。
/// 依存 `shiguredo_webrtc` の libwebrtc を更新した場合は、同ファイルの挙動を
/// 再検証し、本コメントと `H264_PROFILE_PATTERNS` を更新すること。
pub(super) fn parse_profile_level_id(plid: H264ProfileLevelId) -> Option<H264ProfileLevel> {
    // 固定 libwebrtc の `ParseH264ProfileLevelId` は level を先に決め、
    // その後で `kProfilePatterns` にマッチする sub-profile を返す。
    let level = match plid.level_idc {
        10 => H264Level::Level1,
        11 => {
            // constraint_set3_flag は profile-iop の bit 4。
            if plid.profile_iop & 0b0001_0000 != 0 {
                H264Level::Level1b
            } else {
                H264Level::Level1_1
            }
        }
        12 => H264Level::Level1_2,
        13 => H264Level::Level1_3,
        20 => H264Level::Level2,
        21 => H264Level::Level2_1,
        22 => H264Level::Level2_2,
        30 => H264Level::Level3,
        31 => H264Level::Level3_1,
        32 => H264Level::Level3_2,
        40 => H264Level::Level4,
        41 => H264Level::Level4_1,
        42 => H264Level::Level4_2,
        50 => H264Level::Level5,
        51 => H264Level::Level5_1,
        52 => H264Level::Level5_2,
        // 9（Level 1b の別表現）や 60 / 61 / 62（Level 6 系）を含む未知の level_idc。
        _ => return None,
    };

    let profile = H264_PROFILE_PATTERNS
        .iter()
        .find(|pattern| {
            pattern.profile_idc == plid.profile_idc
                && (plid.profile_iop & pattern.mask) == pattern.value
        })
        .map(|pattern| pattern.profile)?;

    Some(H264ProfileLevel { profile, level })
}

/// H.264 の incoming SDP format を required に照らして解決する。
///
/// RFC 6184 Section 8.2.2 の Offer/Answer 規則に沿った判定:
/// - codec 名が `H264` でなければ拒否
/// - `packetization-mode` は 1 のみ受理
/// - `profile-level-id` は必須で、`H264ProfileLevelId::from_hex` でデコードし
///   [`parse_profile_level_id`] で sub-profile / level を判定する
///   （Section 8.1 の省略時既定 Baseline Profile Level 1 へ fallback しない）
/// - sub-profile は required（bitstream 実値）と完全一致を要求する
///   （8.2.2 は level 以外の media format configuration を対称に使う）
/// - level は required 以上（受信側の能力が bitstream を下回る場合のみ拒否）
///
/// `level-asymmetry-allowed` と `max-recv-level` は解釈しない。
/// 本 SDK は send-only で bitstream の level を required のまま送出するため、
/// incoming の level が required 以上であることだけを見る。
///
/// 通過時は検証済みの incoming format をそのまま返す。互換な higher level の
/// negotiated format を parameter ごと保持して encoder handler へ渡すため、
/// required へ置き換えない。
pub(super) fn resolve_h264_incoming(
    required: &SdpVideoFormat,
    mut incoming: SdpVideoFormatRef<'_>,
) -> Option<SdpVideoFormat> {
    let name = incoming.name().ok()?;
    if VideoCodecType::try_from(name.as_str()).ok()? != VideoCodecType::H264 {
        return None;
    }

    let incoming_params: std::collections::HashMap<String, String> =
        incoming.parameters_mut().iter().collect();
    if incoming_params
        .get("packetization-mode")
        .map(String::as_str)
        != Some("1")
    {
        return None;
    }
    let incoming_plid_hex = incoming_params.get("profile-level-id")?;
    let incoming_plid = H264ProfileLevelId::from_hex(incoming_plid_hex).ok()?;
    let incoming_parsed = parse_profile_level_id(incoming_plid)?;

    // required は reader 初期化時に検証済みのため、必ず valid な
    // profile-level-id を持つ（破れている場合は実装バグ）。
    let mut required_owned = required.clone();
    let required_params: std::collections::HashMap<String, String> =
        required_owned.parameters_mut().iter().collect();
    let required_plid_hex = required_params
        .get("profile-level-id")
        .expect("BUG: H.264 required format must have profile-level-id");
    let required_plid = H264ProfileLevelId::from_hex(required_plid_hex)
        .expect("BUG: H.264 required profile-level-id must be valid hex");
    let required_parsed = parse_profile_level_id(required_plid)
        .expect("BUG: H.264 required profile-level-id must be recognized by libwebrtc");

    if incoming_parsed.profile != required_parsed.profile {
        return None;
    }
    if incoming_parsed.level.capability_order() < required_parsed.level.capability_order() {
        return None;
    }
    Some(incoming.to_owned())
}

/// H.264 の required SDP format を組み立てる。
///
/// `packetization-mode=1` に加えて `profile-level-id` を常に広告する
/// （RFC 6184 Section 8.1 の implicit 既定 (Baseline Profile Level 1) へ fallback しない）。
pub(super) fn h264_required_sdp_format(h264_config: &H264TrackConfig) -> SdpVideoFormat {
    let mut format = SdpVideoFormat::new("H264");
    format.parameters_mut().set("packetization-mode", "1");
    format
        .parameters_mut()
        .set("profile-level-id", &h264_config.profile_level_id.to_hex());
    format
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_mp4::bitstream::h264::H264ProfileLevelId;

    /// テスト用。3 byte から [`H264ProfileLevelId`] を組み立てて
    /// [`parse_profile_level_id`] で判定する。
    fn parse(profile_idc: u8, profile_iop: u8, level_idc: u8) -> Option<H264ProfileLevel> {
        parse_profile_level_id(H264ProfileLevelId {
            profile_idc,
            profile_iop,
            level_idc,
        })
    }

    /// 期待する sub-profile / level を持つか確認する。
    fn assert_profile_level(result: &H264ProfileLevel, profile: H264SubProfile, level: H264Level) {
        assert_eq!(
            result.profile, profile,
            "sub-profile が期待値と一致するはずです"
        );
        assert_eq!(result.level, level, "level が期待値と一致するはずです");
    }

    #[test]
    fn parses_all_k_profile_patterns_rows() {
        // kProfilePatterns の 9 row すべてが正しく sub-profile へ写ることを確認する。
        // (profile_idc, profile-iop) は libwebrtc の `kProfilePatterns` と 1 対 1 に対応する。
        assert_profile_level(
            &parse(0x42, 0b0100_0000, 30).expect("CB (42 / x1xx0000) は成功するはずです"),
            H264SubProfile::ConstrainedBaseline,
            H264Level::Level3,
        );
        assert_profile_level(
            &parse(0x4D, 0b1000_0000, 30).expect("CB (4D / 1xxx0000) は成功するはずです"),
            H264SubProfile::ConstrainedBaseline,
            H264Level::Level3,
        );
        assert_profile_level(
            &parse(0x58, 0b1100_0000, 30).expect("CB (58 / 11xx0000) は成功するはずです"),
            H264SubProfile::ConstrainedBaseline,
            H264Level::Level3,
        );
        assert_profile_level(
            &parse(0x42, 0b0000_0000, 30).expect("B (42 / x0xx0000) は成功するはずです"),
            H264SubProfile::Baseline,
            H264Level::Level3,
        );
        assert_profile_level(
            &parse(0x58, 0b1000_0000, 30).expect("B (58 / 10xx0000) は成功するはずです"),
            H264SubProfile::Baseline,
            H264Level::Level3,
        );
        assert_profile_level(
            &parse(0x4D, 0b0000_0000, 30).expect("M (4D / 0x0x0000) は成功するはずです"),
            H264SubProfile::Main,
            H264Level::Level3,
        );
        assert_profile_level(
            &parse(0x64, 0b0000_0000, 30).expect("H (64 / 00000000) は成功するはずです"),
            H264SubProfile::High,
            H264Level::Level3,
        );
        assert_profile_level(
            &parse(0x64, 0b0000_1100, 30)
                .expect("Constrained High (64 / 00001100) は成功するはずです"),
            H264SubProfile::ConstrainedHigh,
            H264Level::Level3,
        );
        assert_profile_level(
            &parse(0xF4, 0b0000_0000, 30)
                .expect("Predictive High 4:4:4 (F4 / 00000000) は成功するはずです"),
            H264SubProfile::PredictiveHigh444,
            H264Level::Level3,
        );
    }

    #[test]
    fn normalizes_equivalent_representations_to_same_sub_profile() {
        // 異なる profile_idc / profile-iop が同じ sub-profile へ正規化されることを確認する。
        // 各表現の `x` (どちらでもよい) を全て 1 にした iop を使う。
        let constrained_baselines = [
            parse(0x42, 0b1111_0000, 30).expect("CB (42 / x1xx0000) は成功するはずです"),
            parse(0x4D, 0b1111_0000, 30).expect("CB (4D / 1xxx0000) は成功するはずです"),
            parse(0x58, 0b1111_0000, 30).expect("CB (58 / 11xx0000) は成功するはずです"),
        ];
        for result in constrained_baselines {
            assert_eq!(
                result.profile,
                H264SubProfile::ConstrainedBaseline,
                "3 つの CB 表現はすべて ConstrainedBaseline へ正規化されるはずです"
            );
        }

        let baselines = [
            parse(0x42, 0b1011_0000, 30).expect("B (42 / x0xx0000) は成功するはずです"),
            parse(0x58, 0b1000_0000, 30).expect("B (58 / 10xx0000) は成功するはずです"),
        ];
        for result in baselines {
            assert_eq!(
                result.profile,
                H264SubProfile::Baseline,
                "2 つの B 表現はすべて Baseline へ正規化されるはずです"
            );
        }
    }

    #[test]
    fn rejects_constrained_high_with_other_high_iop_values() {
        // Constrained High は profile_iop の完全一致 (0x0C) を要求する。
        // mask 0xFF のため、下位 4 bit が 0x0C でも上位 bit が立つと拒否される。
        assert!(
            parse(0x64, 0b1000_1100, 30).is_none(),
            "64 / 10001100 は Constrained High の完全一致に合致しないため拒否されるはずです"
        );
        assert!(
            parse(0x64, 0b0000_1101, 30).is_none(),
            "64 / 00001101 は High / Constrained High のいずれにも一致しないため拒否されるはずです"
        );
        assert!(
            parse(0x64, 0b0000_0001, 30).is_none(),
            "64 / 00000001 は High (mask 0xFF) に一致しないため拒否されるはずです"
        );
    }

    #[test]
    fn rejects_sub_profiles_not_in_k_profile_patterns() {
        // libwebrtc の kProfilePatterns に無い profile / constraint の組み合わせは、
        // required と incoming が byte-for-byte 一致しても unsupported になる。
        // Extended (58 / 00xx0000)
        assert!(
            parse(0x58, 0b0000_0000, 30).is_none(),
            "Extended (58 / 00xx0000) は libwebrtc 非認識のため拒否されるはずです"
        );
        // High10 (6E / 00000000)
        assert!(
            parse(0x6E, 0b0000_0000, 30).is_none(),
            "High10 (6E / 00000000) は libwebrtc 非認識のため拒否されるはずです"
        );
        // High42 (7A / 00000000)
        assert!(
            parse(0x7A, 0b0000_0000, 30).is_none(),
            "High42 (7A / 00000000) は libwebrtc 非認識のため拒否されるはずです"
        );
        // High44 Intra (F4 / 00010000)
        assert!(
            parse(0xF4, 0b0001_0000, 30).is_none(),
            "High44 Intra (F4 / 00010000) は libwebrtc 非認識のため拒否されるはずです"
        );
        // CAVLC 4:4:4 Intra (2C / 00010000)
        assert!(
            parse(0x2C, 0b0001_0000, 30).is_none(),
            "CAVLC 4:4:4 Intra (2C / 00010000) は libwebrtc 非認識のため拒否されるはずです"
        );
        // 未知の profile_idc
        assert!(
            parse(0x00, 0b0000_0000, 30).is_none(),
            "未知の profile_idc は拒否されるはずです"
        );
    }

    #[test]
    fn parses_level_1b_only_for_level_idc_11_with_constraint_set3() {
        // level_idc == 11 + constraint_set3_flag == 1 の表現だけが Level 1b になる。
        // Constrained Baseline (42 / 01010000): cs1 + cs3。
        assert_profile_level(
            &parse(0x42, 0b0101_0000, 11).expect("CB + 11 + cs3 は Level 1b になるはずです"),
            H264SubProfile::ConstrainedBaseline,
            H264Level::Level1b,
        );
        // level_idc == 11 + constraint_set3_flag == 0 は Level 1.1 になる。
        assert_profile_level(
            &parse(0x42, 0b0100_0000, 11).expect("CB + 11 は Level 1.1 になるはずです"),
            H264SubProfile::ConstrainedBaseline,
            H264Level::Level1_1,
        );
        // Main (4D / 00010000) でも 11 + cs3 は Level 1b になる。
        assert_profile_level(
            &parse(0x4D, 0b0001_0000, 11).expect("M + 11 + cs3 は Level 1b になるはずです"),
            H264SubProfile::Main,
            H264Level::Level1b,
        );
        // High (64) は iop が完全一致 (00000000 / 00001100) のため cs3 を立てられず、
        // Level 1b 表現は pattern に合致しない。
        assert!(
            parse(0x64, 0b0001_0000, 11).is_none(),
            "High は cs3 を立てた iop が pattern に合致しないため拒否されるはずです"
        );
    }

    #[test]
    fn rejects_level_1b_with_level_idc_9() {
        // RFC 6184 が informative note で言及する level_idc == 9 の Level 1b 表現は、
        // 固定 libwebrtc が認識しないため拒否する。
        // profile_idc が 66 / 77 / 88 以外（ここでは High）でも同様。
        assert!(
            parse(0x42, 0b0000_0000, 9).is_none(),
            "level_idc == 9 の Level 1b 表現は拒否されるはずです"
        );
        assert!(
            parse(0x64, 0b0000_0000, 9).is_none(),
            "High でも level_idc == 9 は拒否されるはずです"
        );
    }

    #[test]
    fn rejects_level_6_and_unknown_level_idc() {
        // Level 6 / 6.1 / 6.2 (60 / 61 / 62) は固定 libwebrtc の H264Level enum に無い。
        for level_idc in [60, 61, 62] {
            assert!(
                parse(0x42, 0b0000_0000, level_idc).is_none(),
                "level_idc == {level_idc} は拒否されるはずです"
            );
        }
        // その他の未知 level_idc も整数比較で受理しない。
        for level_idc in [0, 1, 15, 63, 100, 255] {
            assert!(
                parse(0x42, 0b0000_0000, level_idc).is_none(),
                "未知の level_idc == {level_idc} は拒否されるはずです"
            );
        }
    }

    #[test]
    fn orders_level_1b_between_level_1_and_level_1_1() {
        // Level 1b は能力順で Level 1 と Level 1.1 の間に位置する
        // （ITU-T H.264 Table A-1 の MaxBR: Level 1 < Level 1b < Level 1.1）。
        assert!(
            H264Level::Level1.capability_order() < H264Level::Level1b.capability_order(),
            "Level 1 は Level 1b より小さいはずです"
        );
        assert!(
            H264Level::Level1b.capability_order() < H264Level::Level1_1.capability_order(),
            "Level 1b は Level 1.1 より小さいはずです"
        );
        // 通常 level は能力順で昇順になる。
        let ordered = [
            H264Level::Level1,
            H264Level::Level1b,
            H264Level::Level1_1,
            H264Level::Level1_2,
            H264Level::Level1_3,
            H264Level::Level2,
            H264Level::Level2_1,
            H264Level::Level2_2,
            H264Level::Level3,
            H264Level::Level3_1,
            H264Level::Level3_2,
            H264Level::Level4,
            H264Level::Level4_1,
            H264Level::Level4_2,
            H264Level::Level5,
            H264Level::Level5_1,
            H264Level::Level5_2,
        ];
        for pair in ordered.windows(2) {
            assert!(
                pair[0].capability_order() < pair[1].capability_order(),
                "level の順序が崩れています: {:?} < {:?}",
                pair[0].capability_order(),
                pair[1].capability_order()
            );
        }
    }

    #[test]
    fn rejects_reserved_zero_bits_in_profile_iop() {
        // kProfilePatterns の全パターンが profile-iop の下位 2 bit に 0 を要求するため、
        // reserved_zero_2bits が非 0 の組み合わせは拒否される。
        assert!(
            parse(0x42, 0b0100_0011, 30).is_none(),
            "CB でも reserved_zero_2bits 非 0 は拒否されるはずです"
        );
        assert!(
            parse(0x64, 0b0000_0011, 30).is_none(),
            "High でも reserved_zero_2bits 非 0 は拒否されるはずです"
        );
    }

    /// `resolve_h264_incoming` 用に `profile-level-id` を指定した H.264 format を作る。
    fn h264_format(profile_level_id: &str) -> SdpVideoFormat {
        let mut format = SdpVideoFormat::new("H264");
        let mut params = format.parameters_mut();
        params.set("packetization-mode", "1");
        params.set("profile-level-id", profile_level_id);
        format
    }

    #[test]
    fn resolve_accepts_same_sub_profile_with_higher_level() {
        let required = h264_format("4d0028"); // Main Level 4.0
        let incoming = h264_format("4d0028"); // 同一
        let resolved = resolve_h264_incoming(&required, incoming.as_ref())
            .expect("同一 format は受理されるはずです");
        let mut resolved_owned = resolved.clone();
        let params: std::collections::HashMap<String, String> =
            resolved_owned.parameters_mut().iter().collect();
        assert_eq!(
            params.get("profile-level-id").map(String::as_str),
            Some("4d0028"),
            "検証済みの incoming format を parameter ごと保持して返すはずです"
        );

        // 同じ Main sub-profile で required 以上の level は byte-for-byte 不一致でも受理する。
        let higher = h264_format("4d0032"); // Main Level 5.0
        let resolved = resolve_h264_incoming(&required, higher.as_ref())
            .expect("互換な higher level は受理されるはずです");
        let mut resolved_owned = resolved.clone();
        let params: std::collections::HashMap<String, String> =
            resolved_owned.parameters_mut().iter().collect();
        assert_eq!(
            params.get("profile-level-id").map(String::as_str),
            Some("4d0032"),
            "higher level の negotiated format は parameter を保持するはずです"
        );
    }

    #[test]
    fn resolve_rejects_incompatible_sub_profile() {
        let required = h264_format("4d0028"); // Main
        let incoming = h264_format("640028"); // High
        assert!(
            resolve_h264_incoming(&required, incoming.as_ref()).is_none(),
            "sub-profile が異なる format は拒否されるはずです"
        );
    }

    #[test]
    fn resolve_rejects_lower_level() {
        let required = h264_format("4d0028"); // Main Level 4.0
        let incoming = h264_format("4d001e"); // Main Level 3.0
        assert!(
            resolve_h264_incoming(&required, incoming.as_ref()).is_none(),
            "required より低い level は拒否されるはずです"
        );
    }

    #[test]
    fn resolve_level_1b_negotiation_bidirectional() {
        // Level 1b は能力順で Level 1 と Level 1.1 の間に位置するため、
        // required と incoming の Level 1 / 1b / 1.1 の組み合わせで双方向を確認する。
        // required=1b / incoming=1 は拒否（Level 1 decoder は 1b の MaxBR を満たせない）。
        let required_1b = h264_format("4d100b"); // Main Level 1b
        let incoming_1 = h264_format("4d000a"); // Main Level 1
        assert!(
            resolve_h264_incoming(&required_1b, incoming_1.as_ref()).is_none(),
            "required が Level 1b のとき Level 1 は拒否されるはずです"
        );
        // required=1 / incoming=1b は受理（1b decoder は Level 1 を decode できる）。
        let required_1 = h264_format("4d000a"); // Main Level 1
        let incoming_1b = h264_format("4d100b"); // Main Level 1b
        let resolved = resolve_h264_incoming(&required_1, incoming_1b.as_ref())
            .expect("required が Level 1 のとき Level 1b は受理されるはずです");
        let mut resolved_owned = resolved.clone();
        let params: std::collections::HashMap<String, String> =
            resolved_owned.parameters_mut().iter().collect();
        assert_eq!(
            params.get("profile-level-id").map(String::as_str),
            Some("4d100b"),
            "negotiated format は incoming の profile-level-id を保持するはずです"
        );
        // required=1.1 / incoming=1b は拒否（1b decoder は 1.1 を decode できない）。
        let required_1_1 = h264_format("4d000b"); // Main Level 1.1
        assert!(
            resolve_h264_incoming(&required_1_1, incoming_1b.as_ref()).is_none(),
            "required が Level 1.1 のとき Level 1b は拒否されるはずです"
        );
        // required=1b / incoming=1.1 は受理（1.1 decoder は 1b を decode できる）。
        let incoming_1_1 = h264_format("4d000b"); // Main Level 1.1
        assert!(
            resolve_h264_incoming(&required_1b, incoming_1_1.as_ref()).is_some(),
            "required が Level 1b のとき Level 1.1 は受理されるはずです"
        );
    }

    #[test]
    fn resolve_rejects_missing_profile_level_id() {
        let required = h264_format("4d0028");
        let mut incoming = SdpVideoFormat::new("H264");
        incoming.parameters_mut().set("packetization-mode", "1");
        assert!(
            resolve_h264_incoming(&required, incoming.as_ref()).is_none(),
            "profile-level-id のない incoming format は拒否されるはずです"
        );
    }

    #[test]
    fn resolve_rejects_invalid_profile_level_id() {
        let required = h264_format("4d0028");
        // 6 桁未満 / 非 base16 は from_hex 経由で拒否される。
        for bad in ["4d00", "4d002", "zz0028", "4d002g", "4d0 28"] {
            let incoming = h264_format(bad);
            assert!(
                resolve_h264_incoming(&required, incoming.as_ref()).is_none(),
                "不正な profile-level-id ({bad:?}) は拒否されるはずです"
            );
        }
        // 6 桁でも libwebrtc 非認識の sub-profile / level は拒否される。
        let unrecognized = h264_format("6e0028"); // High10
        assert!(
            resolve_h264_incoming(&required, unrecognized.as_ref()).is_none(),
            "libwebrtc 非認識の profile は拒否されるはずです"
        );
        let unknown_level = h264_format("4d003c"); // Level 6.0
        assert!(
            resolve_h264_incoming(&required, unknown_level.as_ref()).is_none(),
            "未知の level は拒否されるはずです"
        );
    }

    #[test]
    fn resolve_rejects_non_one_packetization_mode() {
        let required = h264_format("4d0028");
        let mut incoming = SdpVideoFormat::new("H264");
        incoming.parameters_mut().set("packetization-mode", "0");
        incoming.parameters_mut().set("profile-level-id", "4d0028");
        assert!(
            resolve_h264_incoming(&required, incoming.as_ref()).is_none(),
            "packetization-mode が 1 以外の format は拒否されるはずです"
        );
    }

    #[test]
    fn resolve_rejects_non_h264_codec_name() {
        let required = h264_format("4d0028");
        let incoming = SdpVideoFormat::new("VP8");
        assert!(
            resolve_h264_incoming(&required, incoming.as_ref()).is_none(),
            "codec 名が H.264 以外の format は拒否されるはずです"
        );
    }

    #[test]
    fn h264_required_sdp_format_sets_packetization_mode_and_profile_level_id() {
        let config = H264TrackConfig {
            profile_level_id: H264ProfileLevelId {
                profile_idc: 0x4d,
                profile_iop: 0x40,
                level_idc: 0x15,
            },
            avcc_box: None,
        };

        let format = h264_required_sdp_format(&config);
        assert_eq!(
            format
                .name()
                .expect("H264 format の name を取得できるはずです"),
            "H264"
        );
        let mut format_owned = format.clone();
        let params: std::collections::HashMap<String, String> =
            format_owned.parameters_mut().iter().collect();
        assert_eq!(
            params.get("packetization-mode").map(String::as_str),
            Some("1"),
            "packetization-mode=1 を広告するはずです"
        );
        assert_eq!(
            params.get("profile-level-id").map(String::as_str),
            Some("4d4015"),
            "検証済みの profile-level-id を広告するはずです"
        );
    }
}
