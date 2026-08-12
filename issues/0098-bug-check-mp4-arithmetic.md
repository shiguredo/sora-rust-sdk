# MP4 demux の位置・時刻算術を安全化する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-arithmetic
- Polished: 2026-08-10
- Updated: 2026-08-10

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

`src/video_codecs/mp4.rs` の `Mp4SampleReader::new_inner` には次の未検査演算が残る。

- `required.position as usize` は、`u64` の `position` が `usize` に収まらない target で上位 bit を切り捨てる
- `required.size` は `Option<usize>` であり、`start + size` は `usize` 同士の検査なし加算で、overflow すると `min(file_data.len())` でクランプされる（wraparound とクランプの組による未検査演算）
- sample の `data_offset` と `data_size` は `u64` / `usize` のまま保持され、検証後の `get_sample` で `as` 変換と加算をやり直す
- `cumulative_us` の構築で `acc += duration as u64` と `(acc * 1_000_000) / timescale` を未検査で行う。後者は `u64` の overflow を起こし得る

issue 0061 は `required.position > file_data.len()`、issue 0062 は sample range がファイル末尾を超える場合を既に修正した。
本 issue はその初期化時検証方針を維持し、残っている型変換と算術 overflow を同じ検証へ統合する。

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

### required input range

`RequiredInput` から slice range を作る pure helper を追加し、`Mp4SampleReader::new_inner` はその helper の結果だけで `file_data` を slice する。

1. `usize::try_from(required.position)` で開始位置を変換する
2. 変換できない場合、または開始位置が `file_data.len()` を超える場合は、既存の `Mp4Error::InputPositionOutOfRange` を返す
3. `required.size == Some(size)` なら `start.checked_add(size)` で要求終端を計算する
4. 加算が overflow する場合は、`position` と `size` を保持する新しい input range overflow error を返す
5. 加算できるが終端がファイル末尾を超える場合は、現在と同じく終端を `file_data.len()` に丸め、truncated input の最終判定は demuxer に委ねる
6. `required.size == None` ならファイル末尾を終端とする
7. `file_data.get(start..end)` を維持し、計算済み range に対する最後の防御とする

`checked_add` の失敗を `file_data.len()` への丸めとして受理してはならない。
算術的に表現できない要求と、表現できるが入力が不足している要求を区別する。

### sample range

demuxer から video sample を受け取る時点で `data_offset` を `usize::try_from` し、`offset.checked_add(data_size)` と `end <= file_data.len()` を検証する。
変換、加算、ファイル範囲のいずれかに失敗した場合は、sample index、元の `u64` offset、size、file size を保持する既存の `Mp4Error::InconsistentSampleTable` を返す。

検証後の `samples` には `usize` の開始位置と終端位置を保持する。
`get_sample` は検証済みの `Range<usize>` で slice し、`data_offset as usize` と `data_offset as usize + data_size` を hot path に残さない。
issue 0062 で定めた「reader 初期化時に全 sample を検証し、`get_sample` を `Result` 化しない」という API 方針は維持する。

### error

required input の `start + size` overflow には、`position` と `size` を保持する専用の `Mp4Error` variant を追加する。
`cumulative_us` 構築の加算 / 乗算 overflow には sample index を保持する `DurationOverflow` を追加する。
`Display` と `std::error::Error::source` を更新し、error message は日本語とする。
position 変換または開始位置の範囲外は既存の `InputPositionOutOfRange`、sample range の変換・加算・EOF 超過は既存の `InconsistentSampleTable` を使う。

upstream arithmetic error は `Mp4Error::Demux` から source chain を保持して公開 `crate::error::Error` へ伝播させる。
SDK 側で文字列へ変換して原因を失わない。

## test

mock / stub、sleep、外部 command、ネットワークを使わず、pure helper と実 `Mp4SampleReader` をテストする。

- required input range helper で、開始位置 0、ファイル末尾ちょうど、ファイル末尾 1 バイト超過、`start + size` overflow、`size == None` を確認する
- `required.position` の変換は target pointer width ごとに期待値を分け、64 bit では `u64::MAX` の変換成功後に file size error、32 bit の conditional test では `u32::MAX + 1` を変換 error にする
- 既存 H.264 fixture の `moov` header を 64 bit `largesize` 形式の `u64::MAX` へ書き換え、`position > 0` かつ `size == usize::MAX` となる入力で reader が panic せず input range overflow error を返すことを確認する
- sample range helper で、空サンプル、ファイル末尾ちょうど、ファイル末尾 1 バイト超過、`usize::MAX` 近傍の offset / size と加算 overflow を確認する
- `cumulative_us` 構築の加算 / 乗算が overflow する duration 入力を `DurationOverflow`（sample index 付き）で拒否し、正常な入力では従来どおりの累積値になることを確認する
- malformed MP4 は `catch_unwind` で panic の不在だけを確認せず、具体的な error variant、sample index、position、size を検証する
- 既存の composition time offset が 0 の fixture (`testdata/red-320x320-h264.mp4` 等) について、sample payload、送信順序、全 deadline が変わらないことを確認する

fixture を byte patch する場合は、書き換え前の box type、box size、対象 field を `assert_eq!` で確認してから変更し、偶然別の byte 列を書き換えたテストを成功させない。

## 変更対象

- `src/video_codecs/mp4.rs`
- `CHANGES.md`

## 完了条件

- required input の `position` を `usize::try_from` し、`start + size` を `checked_add` してから slice range を構築する
- required input range の変換または加算に失敗すると、panic や clamp ではなく対応する `Mp4Error` を返す
- sample の offset を reader 初期化時に `usize` へ検査付き変換し、`checked_add` 済みの range を `samples` に保持する
- `get_sample` に `as usize` と未検査の range 加算が残っていない
- sample range が変換不能、加算 overflow、またはファイル範囲外の場合に `InconsistentSampleTable` を返す
- SDK 内の duration 累積、microseconds 変換に未検査の `+`、`*`、`as` が残っていない
- large-size box、sample range の境界 test が具体的な error と終了結果を検証する
- 通常 fixture の sample payload、送信順序、deadline が従来どおりである
- debug / release profile に依存せず、同じ不正入力が同じ error になる
- `cargo test --workspace` が成功する
- `cargo test --workspace --release` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 対象外

- `Mp4VideoCapturer` の停止可能な待機は issue 0135 で扱う
- sample 数の上限は issue 0136 で扱う
- feeder thread の deadline 計算の `checked_add` 化は issue 0137 で扱う
- MP4 読み込みのファイルベース化は issue 0138 で扱う
- サンプルメタデータの構造体化は issue 0139 で扱う

## 参考

- `src/video_codecs/mp4.rs`
- `issues/pending/0096-bug-preserve-mp4-presentation-timestamps.md`
- `issues/closed/0061-bug-fix-mp4-demuxer-required-input-oob.md`
- `issues/closed/0062-bug-fix-mp4-get-sample-oob-panic.md`
- `shiguredo_mp4 2026.4.0` の `src/auxiliary.rs`
- `shiguredo_mp4 2026.4.0` の `src/demux_mp4_file.rs`
- Rust standard library `std::primitive::usize::checked_add`
