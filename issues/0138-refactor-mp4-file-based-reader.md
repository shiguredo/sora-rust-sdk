# MP4 読み込みをファイルベースに変更する

- Priority: Medium
- Created: 2026-08-10
- Completed: 2026-08-12
- Branch: feature/refactor-mp4-file-based-reader
- Polished: 2026-08-10

## 目的

ファイル全体をメモリに読み込む設計を見直し、大きなファイルのメモリ消費を抑える。
ファイルサイズが `usize` に縛られる問題（32 bit target では 4GB 超を扱えない）も解消する。

## 現状

`Mp4SampleReader::new_inner` は `std::fs::read` でファイル全体を `Vec<u8>` に読み込む。
doc コメントには「大きなファイルではメモリ使用量に注意」とあり、実際に必要ない範囲（mdat の大部分等）もメモリに保持する。

`shiguredo_mp4` の demuxer は `required_input()` が要求する範囲を `handle_input()` で供給する方式のため、ファイル全体を読み込む必要はなく、要求された範囲だけを都度読み込めばよい。
ただし、`handle_input` は要求範囲を一度に全て渡す必要がある（ストリーミング用途での使用は想定されていない）。

## 設計方針

- `Mp4SampleReader` は `BufReader<File>` を保持し、demuxer の `required_input` の要求範囲を `seek + read` で都度読み込む
- サンプルデータも `get_sample` の際に `seek + read` で読み込む
- `Mp4SampleMeta` は変更しない（`data_offset: u64` + `data_size: usize` のまま。`data_size` の値域は stsz の `u32` に収まる）
- `get_sample` は `seek + read` の失敗（ファイルの変更・削除等）を `Mp4Error` で返すように変更する
  - 0098 の「`get_sample` を `Result` 化しない」方針は、メモリベースで slice のみを行う前提だったため、ファイルベース化に伴い見直す
- `Mp4Error::InputPositionOutOfRange` と `InconsistentSampleTable` の `file_size` フィールドを `usize` から `u64` に変更する（ファイルサイズが `usize` に縛られないため）
- パフォーマンス検証は行わない（I/O パターンが単純で、コードから I/O 回数が分かるため）

## 0098 への影響

本 issue の完了後、0098 の以下の項目が不要になる。

- `required_input_range` の `usize` 変換（`File::seek(u64)` を使えるため）
- `Mp4SampleMeta` の型変更（`Range<usize>` 化）
- `get_sample` の `as usize` 除去（ファイルベースでは変換しないため）
- 「`get_sample` を `Result` 化しない」API 方針の見直し（I/O 導入のため）

0098 は `cumulative_us` の checked 化など、メモリベースに依存しない部分に集中する。

## 完了条件

- 保持するメモリがサンプル数の metadata のみに比例し、ファイルサイズに依存しないこと（コードレビューで確認）
- ファイルサイズが `usize` の表現範囲に縛られないこと（`data_offset` とエラー型の `file_size` が `u64` で表現されること）
- `get_sample` が `seek + read` の失敗を `Mp4Error` で返し、capturer がエラー時に停止すること
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに追記する
- コメントとテストの assertion message は日本語にする

## 解決方法

`Mp4SampleReader` をファイルベースに変更し、ファイル全体をメモリに保持しないようにした。

- `Mp4SampleReader` が `BufReader<File>` を保持し、demuxer の `required_input` が要求する範囲とサンプルデータを `seek + read_exact` で都度読み込む
- `get_sample` を `&mut self` の `Result` 返しに変更し、ファイル読み込みの失敗を `Mp4Error::Io` で返す
- `Mp4VideoCapturer` はサンプル読み込みの失敗時に `rtc_log_error` で記録してフィーダースレッドを終了する
- `Mp4Error::InputPositionOutOfRange` と `InconsistentSampleTable` の `file_size` フィールドを `usize` から `u64` に変更する
- `seek + read_exact` の組み合わせは `read_bytes_at` に集約し、required_input の読み込みサイズ計算は `usize::MAX` センチネルを廃止して `try_from` + エラー伝播に変更する（32 bit target での OOM 経路も解消）
- `CHANGES.md` の develop セクションに `[CHANGE]`（`file_size` の `u64` 化）と `[UPDATE]`（ファイルベース化）を追記する
- テストを追加・強化する（ファイル縮小後の I/O エラー経路、サンプルデータの内容一致検証、stco/stsz/avcC からの期待値計算）
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` は成功する
