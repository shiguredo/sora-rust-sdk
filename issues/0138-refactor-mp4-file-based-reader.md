# MP4 読み込みをファイルベースに変更する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-mp4-file-based-reader
- Polished: {YYYY-MM-DD}

## 目的

ファイル全体をメモリに読み込む設計を見直し、大きなファイルのメモリ消費を抑える。
ファイルサイズが `usize` に縛られる問題（32 bit target では 4GB 超を扱えない）も解消する。

## 現状

`Mp4SampleReader::new_inner` は `std::fs::read` でファイル全体を `Vec<u8>` に読み込む。
doc コメントには「大きなファイルではメモリ使用量に注意」とあり、実際に必要ない範囲（mdat の大部分等）もメモリに保持する。
また、`shiguredo_mp4` の demuxer は `handle_input(Input)` にデータを渡すストリーミング方式に対応しており、ファイル全体を読み込む必要は本来ない。

## 設計方針

- `Mp4SampleReader` は `File` を保持し、demuxer の `required_input` の要求範囲を `seek + read` で都度読み込む
- サンプルデータも `get_sample` の際に `seek + read` で読み込む
- `Mp4SampleMeta.data_range` を `Range<usize>` から `Range<u64>`（ファイル内の位置）に変更し、u64 → usize 変換を不要にする
- フレームごとのディスク I/O がキャプチャ性能に与える影響を検証する

## 完了条件

- 大きなファイルで、メモリ消費がファイルサイズに比例しないこと
- ファイルサイズが `usize` の表現範囲に縛られないこと
- `get_sample` の都度読み込みがキャプチャ性能に実用上の影響を与えないこと
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに追記する
- コメントとテストの assertion message は日本語にする
