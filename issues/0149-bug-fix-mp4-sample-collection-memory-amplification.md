# 壊れた MP4 のサンプル収集でメモリが増幅するのを防ぐ

- Created: 2026-08-21
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-mp4-sample-collection-memory-amplification
- Polished: {YYYY-MM-DD}
- Milestone: 2026.1.0

## 目的

壊れた MP4 ファイルで、`Mp4SampleReader::new` が宣言サンプル数に比例して大量のメモリを確保し、プロセスが OOM で停止するのを防ぐ。

## 現状

`Mp4SampleReader::new`（`src/video_codecs/mp4.rs`）は `Mp4FileDemuxer::next_sample()` をサンプル数分ループして全サンプルの metadata を `Vec` に展開した後、`InconsistentSampleTable` の範囲検証（`data_offset + data_size` が `file_size` を超えていないか）を実行している。

`stts` は run-length 形式のため、1 エントリだけで最大 `u32::MAX`（約 42.9 億）のサンプル数を宣言できる。`stsz` を `Fixed`（`sample_size` 非ゼロ）にすると per-sample の `entry_sizes` 配列が不要になるため、宣言するサンプル数に対してファイルサイズの下限がなくなる。つまり数バイトの u32 値（`stts` / `stsc`）で決まる `sample_count` が物理的な入力サイズで縛られず、数 KB の細工ファイルで巨大なサンプル数を宣言して SDK 側の検証より前に全サンプルを収集させることができる。

- `Mp4SampleMeta`（`src/video_codecs/mp4.rs`）は 24 バイト、累積時刻テーブルの `Mp4Timestamp` は 16 バイト
- `u32::MAX` サンプルを宣言された場合、SDK 側だけで `(24 + 16) × 42.9 億 ≈ 172 GB (160 GiB)` の確保になり OOM による DoS が成立する

### 実測

fixture（`testdata/red-320x320-h264.mp4`、2157 バイト）の `stts` / `stsc` / `stsz` を細工して確認した。

- **1 チャンク型**（`stco` 1 エントリ、100 万サンプル宣言）: SDK は約 45 MB を消費してから `InconsistentSampleTable { index: 2109, offset: 2157, size: 1, file_size: 2157 }` でエラーになる。45 MB の内訳は上流の `sample_data_offsets` 8 MB + SDK の `samples` Vec 24 MB + その他
- **複数チャンク型**（`stco` 50 エントリ、各チャンクのオフセットを mdat 先頭に揃え、500 万サンプル宣言）: SDK は約 173 MB を消費してから index 2201 でエラーになる。内訳は上流 40 MB + SDK 120 MB + その他

つまり範囲検証は全サンプル収集の後でしか機能せず、検証前にメモリが増幅する。

## 設計方針

- `InconsistentSampleTable` の範囲検証（`data_offset + data_size > file_size`）をサンプル収集ループ内へ移動する
  - 1 チャンク型の破損入力では、収集ループ内で検証が走るため即エラーになり、SDK 側の増幅を防げる
- サンプル数の上限は設けない（`issues/closed/0136-bug-fix-mp4-sample-count-limit.md` で「サンプル数のみの上限は中途半端」として対応しない方針のため、それと衝突しない既存検証の早期化で対処する）

### ループ内検証の限界

複数チャンク型の破損入力（`stco` の各チャンクオフセットを mdat 内に揃えた細工）では、ループ内検証でも増幅が残る。

各チャンクの先頭サンプルはファイル内を指すため検証を通過し、チャンク内で `data_offset + data_size` が `file_size` を超える位置まで収集が続く。チャンク数を増やすほど通過するサンプル数は増え、SDK 側は「チャンク数 × チャンク内通過数」ぶんの `samples` Vec を確保する。チャンク数は `stco` エントリ数（1 エントリ 4 バイト）で決まるため、入力サイズにほぼ比例して増やせる。

つまり SDK 側のループ内検証だけでは完全には防げず、本質的な対策は上流（shiguredo_mp4）で `StszBox::Fixed` のとき `data_offset` を `chunk.offset() + (index - chunk_first_index) × sample_size` の算術計算で求め、prefix-sum テーブル（`sample_data_offsets`）は `StszBox::Variable` のときだけ構築すること。これで確保量が常に入力サイズのオーダーに収まる。

### スコープ外

以下は上流（shiguredo_mp4）の修正が必要なため本 issue では扱わない。shiguredo/mp4-rs 側で別途対応を検討する。

- `SampleTableAccessor::new`（`src/auxiliary.rs`）が moov ボックスパース時に全サンプル分の `sample_data_offsets`（8 バイト/サンプル）を eager に構築する問題（`u32::MAX` なら約 34 GB (32 GiB)）
- `StszBox::Fixed` のとき `data_offset` を算術計算で求める修正（上記の複数チャンク型への本質的対策）

## 完了条件

- 範囲検証がサンプル収集ループ内で行われ、破損した `stco` / `co64` / `stsz` による `data_offset + data_size > file_size` を検出した時点で `InconsistentSampleTable` エラーを返す
- 1 チャンク型の細工入力（100 万サンプル宣言）で、収集ループが 2109 サンプル目で即エラーになり、SDK 側のメモリ消費が大きくならないことの回帰テストがある
- 複数チャンク型の細工入力でも、ループ内検証により収集されるサンプル数が「チャンク数 × チャンク内通過数」に抑えられること（従来の全サンプル収集より増幅が小さくなる）の回帰テストがある
- 正常な MP4 入力の動作が変わらない
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` エントリを追記する
- コメントとテストの assertion message は日本語にする
