# MP4 H.264 の profile-level-id を SDP capability に反映する

- Priority: Medium
- Created: 2026-08-13
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-mp4-h264-profile-level-id-v2
- Polished: {YYYY-MM-DD}

## 目的

MP4 の `avcC` から抽出した H.264 profile / constraint / level を SDP capability の `profile-level-id` として表明し、送信 bitstream と capability の乖離を無くす。

## 優先度根拠

Medium。
現状の `Mp4PassthroughVideoCodecCapability::get_supported_formats(Encoder)` は H.264 に対して `packetization-mode=1` だけを付けた bare `H264` を返し、`profile-level-id` を明示しない。
RFC 6184 Section 8.1 は `profile-level-id` 省略時に Baseline Profile Level 1 を implicit 既定にする。
Main / High Profile の MP4 を送信する場合、実 bitstream と capability の profile / level が食い違い、SDP 表明が objective に誤りになる。
症状の顕在化は受信側の decoder が SDP profile をどこまで厳密に扱うかに依存するが、正しい表明を advertise すること自体は独立に必要になる。
B frame Main / High Profile MP4 は issue 0096 の前提条件でもあるが、B frame を含まない Main / High Profile ファイル（I / P のみの高品質エンコード）でも同じ問題が発生するため、B frame timeline 対応と切り離して独立に修正する。

## 現状

`Mp4SampleReader::extract_track_info` の `SampleEntry::Avc1` 分岐は `avc1.avcc_box` から SPS / PPS を Annex B 形式に変換して保存するが、`AVCProfileIndication`、`profile_compatibility`、`AVCLevelIndication` などの `avcC` header field は `Mp4VideoTrackInfo` に保存しない。

issue 0140 で reader 由来の required format に一本化され、`Mp4SampleReader::required_sdp_format()` の H.264 分岐は `packetization-mode=1` だけを付けた `H264` を返す。
`Mp4PassthroughVideoCodecCapability` は `Mp4SampleReader::passthrough_capability()` から生成され、`is_supported` は Encoder かつ reader の codec type と一致する場合のみ true を返す。

`VideoCodecCapability::resolve_sdp_format` のデフォルト実装は `get_supported_formats` との fuzzy match で incoming format を解決する。
`Mp4PassthroughVideoCodecCapability` は現状 `resolve_sdp_format` を override せず、H.264 の profile-level-id を negotiation に反映していない。

固定する `shiguredo_webrtc` の libwebrtc は `m152.7977.0.0`、対応 commit は `6f37672d358475cd17544121a12494da454d85fb` である。
同 commit の `api/video_codecs/h264_profile_level_id.cc` にある `ParseSdpForH264ProfileLevelId` は `profile-level-id` を 3 byte の profile / iop / level に分解し、`kProfilePatterns` の mask / value table でマッチする sub-profile を返す。
`H264IsSameProfile` は両側の parse 成功と sub-profile 一致を要求する。
`kProfilePatterns` に含まれない profile / constraint 組み合わせは、byte-for-byte 一致でも unsupported として扱われる。
同ファイルは m150 (`1f975dfd761af6e5d76d28333191973b258d82a8`) のものと同一であり、`kProfilePatterns` 等に差分はない。

## 前提

本 issue は以下の基盤を前提とする。いずれも develop へ merge 済みである。

- issue 0140（reader-driven capability）: `Mp4SampleReader::required_sdp_format()` の H.264 分岐（`packetization-mode=1` のみ）を profile-level-id 付きに拡張できる。`Mp4PassthroughVideoCodecCapability` は `Mp4SampleReader::passthrough_capability()` から生成され、reader 由来の required format を握る
- issue 0142（sample entry 一貫性）: 全 `sample.sample_entry` の一貫性検証（`codec_type` / `width` / `height` / `nal_length_size` / `parameter_sets` / `timescale`）が動作している
- issue 0143（preference validation + factory pass-through）: `SoraVideoEncoderFactory::create` の `resolve_sdp_format` pass-through が回帰テストで固定され、`validate_video_codec_preference` の bare `SdpVideoFormat` 検証が削除されて `is_supported` の結果が preference validation の source of truth になっている

加えて、`shiguredo_mp4` を `2026.5.0-canary.0` に更新し、`bitstream::h264` モジュールの `parse_sps` / `H264Sps` を前提とする。
`parse_sps` は SPS を RBSP パースして `profile_idc` / `constraint_set_flags` / `level_idc` / `width` / `height` を返すため、`avcC` との一致検証と寸法検証に利用する。

## 設計方針

### 対象範囲

- `src/video_codecs/mp4.rs` の H.264 required format 生成、profile-level-id parser、sample entry 一貫性検証（H.264 固有）、`resolve_sdp_format` の H.264 分岐を対象とする
- RFC 6184 Section 8.1 と libwebrtc の `h264_profile_level_id.cc` を根拠にする
- H.265 / VP8 / VP9 / AV1 の required format と `resolve_sdp_format` 挙動は本 issue で変更しない
- AV1 の profile / level / tier 対応は issue 0097 で扱う
- B frame の presentation timestamp / decode pacing / capture timeline は issue 0096 で扱う
- MP4 audio track は本 issue の対象外とする

### profile-level-id の抽出

`Mp4SampleReader` は H.264 track に対して以下を行う。

- `avc1.avcc_box` の `AVCProfileIndication`、`profile_compatibility`、`AVCLevelIndication` を 3 byte として取り出し、RFC 6184 Section 8.1 の 6 桁 hexadecimal `profile-level-id` として組み立てる
- 全 SPS を `shiguredo_mp4::bitstream::h264::parse_sps` に通し、`H264Sps` の `profile_idc` / `constraint_set_flags` / `level_idc` が `avcC` の 3 byte と一致することを reader 初期化時に検証する
- `parse_sps` は truncated SPS を error にするため、短い SPS はこの検証で拒否される。複数 SPS のいずれかが `avcC` と一致しない場合も invalid H.264 configuration error にする
- `H264Sps` の `width` / `height`（クロップ適用後）と `avc1.visual.width` / `height` の一致を reader 初期化時に検証する
- 抽出した `profile-level-id` を固定 libwebrtc の parser に通し、`kProfilePatterns` に一致しない場合は広告する前に unsupported H.264 profile / level error で reader 初期化を失敗させる
- 検証済み `profile-level-id` を `Mp4VideoTrackInfo` の新設 field に保持する

`Mp4SampleReader::required_sdp_format()` の H.264 分岐は `packetization-mode=1` に加えて抽出済み `profile-level-id` を含めた `SdpVideoFormat` を返す。
暗黙の Baseline Profile Level 1 へ fallback しない。

### format negotiation

`Mp4PassthroughVideoCodecCapability::resolve_sdp_format(Encoder, incoming)` を H.264 に対して override し、RFC 6184 Section 8.1 の profile-level-id negotiation に従って incoming format を検証する。
互換なら supported format へ置き換えず、検証済みの incoming format を返す。
H.265、VP8、VP9、AV1 は issue 0140 の挙動（default trait 実装の fuzzy match）を維持する。

format 解決では次を検証する。

- `packetization-mode` は 1 とする
- remote / negotiated format に `profile-level-id` がなければ拒否する
- profile と constraint flags は required bitstream と同じ H.264 sub-profile と互換である
- negotiated receiving level は required bitstream の level 以上である
- `level-asymmetry-allowed` の有無だけを理由に byte-for-byte 一致を要求せず、down-level negotiation は拒否する

required bitstream の `profile-level-id` を SDP へ正確に広告する処理と、remote decoder capability の互換性判定を分離する。
互換な higher level は受理し、incompatible profile / constraint と lower level は受理しない。

送信先 decoder が required sub-profile と level を受信できない場合は codec negotiation を成立させず、実 bitstream と異なる profile へ downgrade しない。

### profile-level-id parser

profile-level-id は pure helper で parse し、normalized sub-profile と normalized level に分ける。

- 文字列は 6 桁の ASCII hexadecimal に限定し、大文字と小文字を同値に扱う
- `profile_idc`、`profile-iop`、`level_idc` の 3 byte へ分解し、`profile-iop` の reserved zero bits が非 0 なら拒否する
- 固定 libwebrtc commit の `api/video_codecs/h264_profile_level_id.cc` にある `kProfilePatterns` を mask / value table として実装し、RFC 6184 Section 8.1、Table 5 に由来する複数表現と Constrained High など、固定 parser が認識する WebRTC profile pattern を同じ sub-profile へ normalize する
- 固定 libwebrtc の `H264IsSameProfile` は双方の parse 成功を要求するため、`kProfilePatterns` に一致しない profile / constraint combination は required と incoming が byte-for-byte 一致しても unsupported とする
- Level 1b は固定 parser と同じく、`level_idc == 11` かつ `constraint_set3_flag == 1` の表現だけを normalize する
- RFC 6184 が定める `level_idc == 9` の Level 1b は固定 parser が認識せず PeerConnection negotiation を成立させられないため、本 issue では unsupported とする
- Level 1b は通常の Level 1.1 と区別し、Level 1 と Level 1.1 の間に順序付ける
- 通常 level は RFC 6184 が許す `level_idc` だけを normalized enum へ変換し、未知の値を単純な整数比較で受理しない

profile-level-id の sub-profile 正規化は RFC 6184（RTP Payload Format for H.264 Video）の文脈であり、MP4 コンテナ処理を担う mp4-rs の責務外として sora-rust-sdk 側に実装する。
mp4-rs に RFC 6184 準拠の profile-level-id 正規化 API を追加する issue（mp4-rs の issue 0082）が起票されており、将来そこへ寄せられる場合は libwebrtc 互換のフィルタ（`kProfilePatterns` の部分集合選択、Constrained High など）だけを sora-rust-sdk 側に残す。

### sample entry の一貫性 (H.264 拡張)

issue 0142 で `Mp4SampleReader::new_inner` は全 `sample.sample_entry` を `extract_track_info` に通し、`codec_type` / `width` / `height` / `nal_length_size` / `parameter_sets` / `timescale` の byte-for-byte 一致を検証する。

本 issue では H.264 に固有の追加検証として、以下も最初の configuration と比較する。

- `avcC` box 全体の byte-for-byte 一致（issue 0142 の `parameter_sets` 一致は SPS / PPS の Annex B 化後のみを担保するため、`avcC` header と `length_size_minus_one` などを含む box 全体の一致を独立に確認する）
- 抽出した `profile-level-id` の 3 byte の一致

いずれかが変わる sample description は sample index と相違項目を含む sample description error で reader 初期化時に拒否する（issue 0142 の `Mp4Error::InconsistentSampleDescription` の相違項目に H.264 固有 field を追加する）。

### 固定 libwebrtc の source audit

固定 libwebrtc commit の source audit で次を確認し、production コメントに file / function / revision / 依存更新時の再検証条件を記載する。

- `api/video_codecs/h264_profile_level_id.cc` の `ParseSdpForH264ProfileLevelId` が profile / iop / level の 3 byte を分解し、`kProfilePatterns` の mask / value でマッチする
- `H264IsSameProfile` が双方の parse 成功と sub-profile 一致を要求する
- `kProfilePatterns` の全 row の（mask, value, sub-profile）関係
- Level 1b の表現規則（`level_idc == 11` + `constraint_set3_flag == 1`）
- Constrained High などの WebRTC profile pattern

### test fixture

Main Profile の実 H.264 fixture（B frame を含まない I / P のみ）を `testdata/` に追加する。
fixture はリポジトリへ commit し、生成に使用した ffmpeg の version と command、`avcC` の `AVCProfileIndication` / `profile_compatibility` / `AVCLevelIndication` の値、期待する `profile-level-id`、SPS の profile / constraint / level 3 byte をテストコメントへ記録する。
CI で ffmpeg を起動したり、ネットワークから fixture を取得したりしない。
既存の High Profile Level 2.1 fixture（`red-320x320-h264.mp4` は `avcC` の `AVCProfileIndication=0x64` / `profile_compatibility=0x00` / `AVCLevelIndication=0x15`、期待する `profile-level-id` は `640015`）については、抽出される `profile-level-id` が `640015` と一致する回帰テストを追加する。

## 変更対象

- `src/video_codecs/mp4.rs`
- `src/video_codecs/h264.rs`（profile-level-id parser。libwebrtc 互換のフィルタを含む）
- `Cargo.toml`（`shiguredo_mp4` を `2026.5.0-canary.0` へ更新）
- `testdata/` の Main Profile fixture
- `CHANGES.md`

## 完了条件

- `Mp4SampleReader::required_sdp_format()` の H.264 分岐が `packetization-mode=1` と `avcC` 由来の `profile-level-id` を含む `SdpVideoFormat` を返す
- H.264 format test で次を確認する
  - fixture の `avcC` と全 SPS から期待する Baseline / Constrained Baseline / Main / High 各 Profile の `profile-level-id` を取得する
  - passthrough capability の encoder format が `packetization-mode=1` と exact `profile-level-id` を広告する
  - `profile-level-id` を省略した format、互換でない profile / constraint、required より低い level の format では encoder を生成しない
  - 同じ H.264 sub-profile で required 以上の level は byte-for-byte 不一致でも受理する
  - 6 桁未満 / 超過、非 hexadecimal、reserved bits 非 0 を拒否する
  - 固定 libwebrtc の `kProfilePatterns` 全 row と、異なる `profile_idc` / `profile-iop` から同じ sub-profile へ normalize される組み合わせを確認する
  - 固定 libwebrtc が認識する Constrained High `640c..` などの WebRTC profile pattern を確認する
  - 固定 libwebrtc の既知 pattern にない profile / constraint combination は、required と incoming が exact match でも拒否する
  - `level_idc == 11` + `constraint_set3_flag` を Level 1b へ normalize し、`level_idc == 9` の Level 1b 表現は拒否する
  - Level 1、Level 1b、Level 1.1 の順序を確認する
  - `avcC` 由来の required format が固定 libwebrtc の未知 profile / level の場合は reader 初期化時に拒否する
  - 短い SPS（`parse_sps` の truncated error）、`avcC` と SPS の不一致、複数 SPS 間の不一致を reader 初期化時に拒否する
  - `H264Sps` の `width` / `height` と `avc1.visual.width` / `height` の不一致を reader 初期化時に拒否する
- sample description test で、2 個目以降の `avcC` box 全体または抽出後の `profile-level-id` が変わる合成 fixture / synthetic table を sample index と相違項目付き error で拒否する
- 実 `Mp4PassthroughVideoCodecCapability`、`VideoCodecPreference::new_from_capability`、`validate_video_codec_preference`、`SoraVideoEncoderFactory` を通す test で次を確認する
  - `resolve_sdp_format` は profile-level-id のない実 incoming format を拒否する
  - compatible higher level の negotiated format を parameter ごと保持して encoder handler へ渡す
  - incompatible sub-profile と lower level の negotiated format では encoder を生成しない
- Main Profile 実 fixture の reader test で、抽出された `profile-level-id` が期待値と一致する
- 既存の High Profile Level 2.1 fixture について、抽出される `profile-level-id` が `640015` と一致する回帰テストがある
- 固定 libwebrtc commit `6f37672d358475cd17544121a12494da454d85fb` の `api/video_codecs/h264_profile_level_id.cc` の `ParseSdpForH264ProfileLevelId`、`H264IsSameProfile`、`kProfilePatterns` を source audit し、production コメントへ記録する
- RFC 6184 Section 8.1 と Table 5 を参照し、profile-level-id 判定の根拠を production code の日本語コメントに記載する
- reader / capability / encoder handler の unit test は mock / stub、sleep、`#[ignore]`、外部 command、ネットワークを使用しない
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
