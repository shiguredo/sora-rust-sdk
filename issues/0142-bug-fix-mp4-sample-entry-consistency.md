# MP4 の sample description 切り替わりを明示エラーで拒否する

- Priority: Medium
- Created: 2026-08-13
- Completed: 2026-08-14
- Branch: feature/fix-mp4-sample-entry-consistency
- Polished: {YYYY-MM-DD}

## 目的

MP4 の途中で codec_type / 解像度 / SPS / PPS / NAL 長サイズが変わる sample description を、silent breakage ではなく `Mp4Error::InconsistentSampleDescription` で reader 初期化時に明示的に拒否する。

## 優先度根拠

Medium。
現状の `Mp4SampleReader::new_inner` は最初の `Some(sample_entry)` だけを見て `Mp4VideoTrackInfo` を確定し、以後の `Some(sample_entry)` を無視する。
sample description が途中で切り替わる MP4 でも silently 最初の configuration のまま encoder callback へ渡され、受信 decoder は途中から parameter 不一致で映像が壊れる。
通常の MP4 では sample description は 1 つだけで発生しないが、連結された `MediaRecorder` 出力や edit 済み MP4 のように複数 sample description を持つ入力ではこの silent breakage を踏む。
本 issue は「起きるとユーザーが原因不明の映像破損に遭遇する」経路を、明示エラーで塞ぐ fail-safe として位置付ける。

## 現状

`Mp4SampleReader::new_inner` の while ループは以下の形になっている。

```rust
if track_info.is_none()
    && let Some(entry) = sample.sample_entry
{
    track_info = Some(Self::extract_track_info(entry, timescale)?);
}
```

`track_info.is_none()` 条件により、最初の Some(sample_entry) で `Mp4VideoTrackInfo` を確定した後は、`shiguredo_mp4::demux::Sample::sample_entry` が再度 `Some` を返しても評価されない。
`shiguredo_mp4` の demuxer は sample description が切り替わるサンプルで `Some(sample_entry)` を返し、byte-for-byte 完全一致の再掲は `None` に normalize するため、後発の `Some(sample_entry)` は「前回と異なる configuration」であることが確定している。

## 設計方針

### 対象範囲

- `src/video_codecs/mp4.rs` の `Mp4SampleReader::new_inner` ループの sample_entry 判定と、`Mp4Error` バリアント追加のみを対象とする
- capability / encoder handler / preference validation / factory 経路は本 issue で変更しない
- codec 固有 field（H.264 の `avcC` header / profile-level-id、AV1 の `av1C` / configOBUs など）の bit-identical 検証は各 codec 固有の別対応で `Mp4VideoTrackInfo` を拡張する形で加える。本 issue は現在 `Mp4VideoTrackInfo` に保持している field のみを対象にする

### sample entry の一貫性検証

`Mp4SampleReader::new_inner` の while ループを次のように変える。

- 最初の `Some(sample_entry)` で `extract_track_info` を呼び、`Mp4VideoTrackInfo` を確定する
- 2 個目以降の `Some(sample_entry)` でも `extract_track_info` を呼び、以下を最初の `Mp4VideoTrackInfo` と field 単位で比較する
  - `codec_type`
  - `width` / `height`
  - `nal_length_size`
  - `parameter_sets` の byte 列（`Option<Vec<u8>>` を byte-for-byte 比較）
- `timescale` は `mdhd` の track 単位属性で `SampleEntry` からは抽出されず、`extract_track_info` にはループ外の同一 scalar が毎回渡されるため、sample entry 間で変わり得ない値として比較対象に含めない
- いずれかが変わった場合は sample index と相違した field 名を含む新設の `Mp4Error::InconsistentSampleDescription` で reader 初期化を失敗させる
- byte-for-byte 完全一致の sample entry の再掲は `shiguredo_mp4` の demuxer 側で `None` に normalize されるため、reader ループには到達せず自然に受理される

### エラー種別

`Mp4Error` に以下を追加する。

```rust
InconsistentSampleDescription {
    /// 相違が検出されたビデオサンプルの 0 始まりインデックス。
    index: usize,
    /// 相違した Mp4VideoTrackInfo の field 名。
    fields: Vec<&'static str>,
},
```

`Display` / `Error::source` / `From` の実装にも追加する。

### ヘルパー関数

`collect_mismatched_track_info_fields(first: &Mp4VideoTrackInfo, current: &Mp4VideoTrackInfo) -> Vec<&'static str>` を pure helper として追加する。
field 単位比較の順序は本 issue の「設計方針 → sample entry の一貫性検証」の記載順（codec_type → width → height → nal_length_size → parameter_sets）とする。
`Mp4VideoTrackInfo` に `PartialEq` は導入せず、field を直接比較する（今後 codec 固有 field を拡張する際に、比較対象の追加意図を明示的にコード上に残すため）。

## 変更対象

- `src/video_codecs/mp4.rs`
- `CHANGES.md`

## 完了条件

- `Mp4SampleReader::new_inner` は最初の `Some(sample_entry)` 以外にも `extract_track_info` を呼び、`codec_type` / `width` / `height` / `nal_length_size` / `parameter_sets` のいずれかが最初と異なる場合は sample index と相違項目を含む `Mp4Error::InconsistentSampleDescription` で失敗する
- `Mp4Error::InconsistentSampleDescription` の `Display` 実装が sample index と相違 field 名を含む
- `collect_mismatched_track_info_fields` の table-driven test で以下を確認する
  - 完全一致は空 Vec を返す
  - 単一 field の相違はその field 名だけを返す
  - 複数 field の相違は設計方針の記載順で並ぶ
  - `timescale` を変えても相違として報告されない（そもそも比較対象外）
- 既存の合成 fixture / real fixture の reader test が引き続き成功する
- 本 issue の unit test は mock / stub、sleep、`#[ignore]`、外部 command、ネットワークを使用しない
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 解決方法

### 実装

- `Mp4Error::InconsistentSampleDescription { index: usize, fields: Vec<&'static str> }` variant を追加する
- `Mp4SampleReader::new_inner` の while ループを、最初の `Some(sample_entry)` だけでなく、以後の `Some(sample_entry)` も `extract_track_info` に通して一貫性検証する形に変更する
- `Mp4SampleReader::collect_mismatched_track_info_fields` private method を追加し、`codec_type` / `width` / `height` / `nal_length_size` / `parameter_sets` の 5 フィールドを比較する。相違があれば `Mp4Error::InconsistentSampleDescription` を返す
- `Mp4VideoTrackInfo` の両側を exhaustive に destructure して、新フィールド追加時にヘルパー未更新が compile error として検出されるようにする（`timescale` は `mdhd` の track 単位属性で `SampleEntry` からは抽出されないため比較対象外とし、`_` に束縛する）
- `Mp4Error::InconsistentSampleDescription` の `Display` は `sample=<index> fields=<fields>` 形式で相違した フィールド名 を含めて出力する

### テスト

- `sample_description_consistency_check_reports_field_mismatches`: 5 フィールドそれぞれの単独相違、`parameter_sets` の Some/None 単独遷移、複数フィールド同時変更時の順序、`timescale` が比較対象外であることを検証する
- `inconsistent_sample_description_display_and_source`: `Display` 実装が sample index と全ての相違フィールド名を含むこと、`Error::source()` が None を返すことを検証する

### CHANGES.md

- `[FIX] MP4 の途中でサンプルエントリーが切り替わる入力を Mp4SampleReader の初期化時に拒否する` を追加する
