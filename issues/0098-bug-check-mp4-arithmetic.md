# MP4 demux の位置・時刻算術を安全化する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-arithmetic
- Polished: 2026-08-10
- Updated: 2026-08-12

## 目的

`shiguredo_mp4` と `Mp4SampleReader` が MP4 内の位置、サイズ、sample count、sample duration から metadata と入力範囲を計算する際に、整数の切り捨て、overflow、範囲外 slice で panic または wraparound しないようにする。
構造上は parse できても表現可能な位置・時刻範囲を超える入力は、demuxer または reader の初期化 error として拒否する。

## 優先度根拠

High。
攻撃者が直接送るネットワーク入力ではないが、公開 API へ渡す小さなローカル MP4 ファイルの box large size、`stts`、`ctts`、`stco` / `co64` を書き換えるだけで到達できる。
SDK が metadata を受け取る前の依存 crate 内の未検査演算は `shiguredo_mp4 2026.4.0` で解消済みだが、SDK 側の未検査演算は debug build の panic と release build の wraparound で挙動が変わる。

## 現状

### `shiguredo_mp4 2026.4.0`

起票時点の `2026.3.0` で指摘した未検査演算のうち、次は固定中の `2026.4.0` で実装済みである。

- `BoxHeader::box_size` の `u64` から `usize` への変換は `usize::try_from` と error 化された
- `stts` / `ctts` の累積 sample count は `checked_add` と `SampleTableAccessorError::SampleCountOverflow` で検証される
- chunk 内の offset への sample size の加算は `checked_add` と `SampleDataOffsetOverflow` で検証される
- 累積 duration と `SampleAccessor::timestamp` は、Σ sample count <= `u32::MAX` なら総 duration < `u64::MAX` となる invariant 証明により infallible のまま維持される

### `sora-rust-sdk`

required input range と sample range の未検査演算は、issue 0138 のファイルベース化で解決済みである。

- required input の `position` は `u64` のまま seek に使われ、 `required.position > file_size` の検査と `usize::try_from` の error 化により、上位 bit の切り捨てと加算 wraparound は存在しない
- sample range は `Mp4SampleReader::new_inner` で `data_offset.checked_add(data_size as u64)` と `end > file_size` を検証し、 `InconsistentSampleTable` を返す
- `get_sample` は `&mut self` の `Result` 返しに変更され、 I/O エラーを `Mp4Error::Io` で返す
- `Mp4Error::InputPositionOutOfRange` と `InconsistentSampleTable` の `file_size` は `u64` に変更された

残っている未検査演算は `cumulative_us` の構築のみである。

- `acc += sample.duration as u64` （`u64` の加算 overflow）
- `(acc * 1_000_000) / timescale` （`u64` の乗算 overflow。累積 duration が `u64::MAX / 1_000_000` を超えると overflow する）

本 issue は、この duration 累積と microseconds 変換の算術 overflow を初期化時検証へ統合する。

## 設計方針

### `shiguredo_mp4` の安全化

`shiguredo_mp4 2026.4.0` で実装済みであり、本 issue の対応対象から外す。

- `BoxHeader::box_size` の `u64` から `usize` への検査付き変換
- `stts` / `ctts` の累積 sample count の `checked_add` と `SampleCountOverflow`
- chunk 内の offset への sample size 加算の `checked_add` と `SampleDataOffsetOverflow`
- `SampleTableAccessorError` の具体的 variant と `DemuxError` からの source chain 伝播
- 累積 duration と `SampleAccessor::timestamp` の invariant 証明（Σ sample count <= `u32::MAX` なら総 duration < `u64::MAX`）

### duration 累積と microseconds 変換

`cumulative_us` の構築で行う `acc += duration as u64` と `(acc * 1_000_000) / timescale` は checked arithmetic へ変更する。
加算または乗算が overflow した場合は、sample index を含む `DurationOverflow` error で reader 初期化を失敗させる。
検証は reader 初期化時（`cumulative_us` 構築時）に行い、`get_sample` の hot path には未検査演算を残さない。

### required input range と sample range

issue 0138 のファイルベース化で解決済みであるため、本 issue の対応対象から外す。

- required input の `position` は `u64` のまま seek に使われ、ファイルサイズとの比較と `usize::try_from` で安全化されている
- sample range は `Mp4SampleReader::new_inner` で `data_offset.checked_add(data_size as u64)` と `end > file_size` を検証し、 `InconsistentSampleTable` を返す
- `get_sample` は `&mut self` の `Result` 返しに変更され、 I/O エラーを `Mp4Error::Io` で返す

### error

`cumulative_us` 構築の加算 / 乗算 overflow には sample index を保持する `DurationOverflow` を追加する。
`Display` と `std::error::Error::source` を更新し、error message は日本語とする。

upstream arithmetic error は `Mp4Error::Demux` から source chain を保持して公開 `crate::error::Error` へ伝播させる。
SDK 側で文字列へ変換して原因を失わない。

## test

mock / stub、sleep、外部 command、ネットワークを使わず、実 `Mp4SampleReader` をテストする。

- `cumulative_us` 構築の加算 / 乗算が overflow する duration 入力を `DurationOverflow`（sample index 付き）で拒否し、正常な入力では従来どおりの累積値になることを確認する
- malformed MP4 は `catch_unwind` で panic の不在だけを確認せず、具体的な error variant、sample index を検証する
- 既存の composition time offset が 0 の fixture (`testdata/red-320x320-h264.mp4` 等) について、sample payload、送信順序、全 deadline が変わらないことを確認する

fixture を byte patch する場合は、書き換え前の box type、box size、対象 field を `assert_eq!` で確認してから変更し、偶然別の byte 列を書き換えたテストを成功させない。

## 変更対象

- `src/video_codecs/mp4.rs`
- `CHANGES.md`

## 完了条件

- SDK 内の duration 累積、microseconds 変換に未検査の `+`、`*`、`as` が残っていない
- `cumulative_us` 構築の加算 / 乗算が overflow する入力で、panic せず `DurationOverflow`（sample index 付き）を返す
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
- Rust standard library `std::primitive::usize::checked_add`
