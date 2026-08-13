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

本 issue は、この乗算 overflow を、tick のまま累積を保持して変換を安全化する方式で解消する。

## 設計方針

### duration 累積と再生時刻の保持

`cumulative` にはタイムスケール単位 (tick) の累積 duration をそのまま保持し、マイクロ秒への事前変換は行わない。
tick と timescale を保持する `Mp4Duration` を追加し、`std::time::Duration` への変換は商と剰余に分けて行うことで overflow しない。

- 秒: `ticks / timescale` は `u64` に収まる
- ナノ秒: `(ticks % timescale) * 1_000_000_000 / timescale` は 1_000_000_000 未満になる

`acc += duration as u64` の加算は、`shiguredo_mp4` の invariant（Σ sample count <= `u32::MAX` なら総 duration < `u64::MAX`）により overflow せず、checked 化の対象としない。
invariant による保証をコメントで明記する。

## test

mock / stub、sleep、外部 command、ネットワークを使わず、実 `Mp4SampleReader` をテストする。

- `Mp4Duration::to_duration` が overflow せず正しい `Duration` を返すことを確認する（ticks=0、1 秒ちょうど、割り切れない剰余、`u64::MAX` の巨大値）
- 既存の composition time offset が 0 の fixture (`testdata/red-320x320-h264.mp4` 等) について、sample payload、送信順序、`cumulative_duration` の全値が変わらないことを確認する
- malformed MP4 は panic せずに具体的な error variant、sample index を検証する

fixture を byte patch する場合は、書き換え前の box type、box size、対象 field を `assert_eq!` で確認してから変更し、偶然別の byte 列を書き換えたテストを成功させない。

## 変更対象

- `src/video_codecs/mp4.rs`
- `CHANGES.md`

## 完了条件

- タイムスケール単位の累積 duration を `Mp4Duration` で保持し、`std::time::Duration` への変換が overflow しない
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
