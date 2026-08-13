# MP4 の時刻算術を安全化する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-duration-overflow
- Polished: 2026-08-13
- Updated: 2026-08-12

## 目的

`Mp4SampleReader` が MP4 内の sample duration から再生時刻を計算する際に、整数の overflow で panic または wraparound しないようにする。
構造上は parse できても表現可能な時刻範囲を超える入力は、reader の初期化 error として拒否する。

## 優先度根拠

High。
攻撃者が直接送るネットワーク入力ではないが、公開 API へ渡す小さなローカル MP4 ファイルの `stts` を書き換えるだけで到達できる。
`stts` は run-length 形式のため、1 エントリで巨大な duration を表現できる。
SDK 側の未検査演算は debug build の panic と release build の wraparound で挙動が変わる。

## 現状

### `shiguredo_mp4 2026.4.0`

2026.3.0 で指摘した未検査演算のうち、以下は 2026.4.0 で実装済みである。

- `BoxHeader::box_size` の `u64` から `usize` への変換は `usize::try_from` と error 化された
- `stts` / `ctts` の累積 sample count は `checked_add` と `SampleTableAccessorError::SampleCountOverflow` で検証される
- chunk 内の offset への sample size の加算は `checked_add` と `SampleDataOffsetOverflow` で検証される
- 累積 duration と `SampleAccessor::timestamp` は、Σ sample count <= `u32::MAX` なら総 duration < `u64::MAX` となる invariant 証明により infallible のまま維持される

### `sora-rust-sdk`

required input range と sample range の未検査演算は、issue 0138 のファイルベース化で解決済みである。

- required input の `position` は `u64` のまま seek に使われ、`required.position > file_size` の検査と `usize::try_from` の error 化により、上位 bit の切り捨てと加算 wraparound は存在しない
- sample range は `Mp4SampleReader::new_inner` で `data_offset.checked_add(data_size as u64)` と `end > file_size` を検証し、`InconsistentSampleTable` を返す
- `get_sample` は `&mut self` の `Result` 返しに変更され、I/O エラーを `Mp4Error::Io` で返す
- `Mp4Error::InputPositionOutOfRange` と `InconsistentSampleTable` の `file_size` は `u64` に変更された

残っている未検査演算は `cumulative_us` の構築のみである。

- `(acc * 1_000_000) / timescale`（`u64` の乗算 overflow。累積 duration が `u64::MAX / 1_000_000` を超えると overflow する）

本 issue は、この microseconds 変換の乗算 overflow を初期化時検証へ統合する。

## 設計方針

### duration 累積と microseconds 変換

`cumulative_us` の構築で行う `(acc * 1_000_000) / timescale` は checked arithmetic へ変更する。
乗算が overflow した場合は、sample index を含む `DurationOverflow` error で reader 初期化を失敗させる。
検証は reader 初期化時（`cumulative_us` 構築時）に行い、`get_sample` の hot path には未検査演算を残さない。

`acc += duration as u64` の加算は、`shiguredo_mp4` の invariant（Σ sample count <= `u32::MAX` なら総 duration < `u64::MAX`）により overflow せず、checked 化の対象としない。
invariant による保証をコメントで明記する。

### error

`cumulative_us` 構築の乗算 overflow には、sample index を保持する `DurationOverflow` を追加する。
sample index は 0 始まりのビデオサンプル連番で、overflow に達した時点で最後に加算したサンプルを示す。
`Display` と `std::error::Error::source` を更新し、error message は日本語とする。

## test

mock / stub、sleep、外部 command、ネットワークを使わず、実 `Mp4SampleReader` をテストする。

- `cumulative_us` 構築の乗算が overflow する duration 入力を `DurationOverflow`（sample index 付き）で拒否し、閾値直前の 4294 サンプルでは受理されることを確認する
  - 既存フィクスチャの byte patch では累積 duration が `u64::MAX / 1_000_000` に届かないため、テスト内で合成 MP4 を組み立てる
  - 最小到達条件は duration を `u32::MAX` とした 4295 サンプル以上（`stts` 1 エントリ `{sample_count: 4295, sample_delta: 4294967295}`、`stsz` / `stsc` / `stco` を整合させる。`stsd` は SDK が受理する SampleEntry にし、`mdat` のペイロードはサンプルサイズの合計以上を確保する）
  - 加算 overflow は `shiguredo_mp4` の invariant により到達不能であり、テスト対象にしない
- malformed MP4 は panic せずに具体的な error variant、sample index を検証する
- 既存の composition time offset が 0 の fixture (`testdata/red-320x320-h264.mp4` 等) について、sample payload、送信順序、`cumulative_us` の全値が変わらないことを確認する

fixture を byte patch する場合は、書き換え前の box type、box size、対象 field を `assert_eq!` で確認してから変更し、偶然別の byte 列を書き換えたテストを成功させない。

## 変更対象

- `src/video_codecs/mp4.rs`
- `CHANGES.md`

## 完了条件

- SDK 内の microseconds 変換に未検査の `*` が残っていない（加算は invariant により overflow しないため対象外）
- `cumulative_us` 構築の乗算が overflow する入力で、panic せず `DurationOverflow`（sample index 付き）を返す
- 加算 overflow が `shiguredo_mp4` の invariant により到達不能である旨がコメントに明記されている
- debug / release profile に依存せず、同じ不正入力が同じ error になる
- `cargo test --workspace` が成功する
- `cargo test --workspace --release` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 参考

- `src/video_codecs/mp4.rs`
- `issues/pending/0096-bug-preserve-mp4-presentation-timestamps.md`
- `issues/closed/0061-bug-fix-mp4-demuxer-required-input-oob.md`
- `issues/closed/0062-bug-fix-mp4-get-sample-oob-panic.md`
- `shiguredo_mp4 2026.4.0` の `src/auxiliary.rs`
- `shiguredo_mp4 2026.4.0` の `src/demux_mp4_file.rs`
- Rust standard library `std::primitive::u64::checked_mul`
