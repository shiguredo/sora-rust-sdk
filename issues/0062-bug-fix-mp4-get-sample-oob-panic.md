# MP4 の `get_sample` で不整合な `stsz` / `stco` を持つファイルによる範囲外 panic を解消する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-mp4-get-sample-oob-panic
- Polished: 2026-07-27

## 目的

`Mp4SampleReader::get_sample` が `file_data[data_offset as usize..data_offset as usize + data_size]` を無防備にスライスするため、壊れた MP4（`stsz` / `stco` / `co64` に不整合がある等）で range panic する。フィーダースレッド（`Mp4VideoCapturer` の feeder）がクラッシュする経路になっており、`Result` を経由した安全な失敗経路に置き換える。

なお、`demuxer.required_input()` ループ内の同種パニックは #0061 で対応する。

## 優先度根拠

High。`Mp4VideoCapturer` はユーザーが指定した MP4 ファイルを読み込んで送信する公開 API 経路。悪意ある / 破損した MP4 でフィーダースレッドが落ちるとキャプチャが止まり、ユーザーは原因を知る術がない。

## 現状

`src/video_codecs/mp4.rs:415-417` の `get_sample` で、以下のようにサンプルデータを取り出している:

```rust
let (data_offset, data_size, keyframe, _, _) = self.samples[index];
let raw_data = &self.file_data[data_offset as usize..data_offset as usize + data_size];
```

- `data_offset` は `u64`、`data_size` は `usize` で、キャストしている。64-bit 環境では安全だが型が一致していない。
- `data_offset + data_size` が `file_data.len()` を超える場合、スライスで panic。
- `data_offset + data_size` の `usize` オーバーフローは未チェック（`data_offset` が不正に大きい場合）。
- `Mp4SampleReader::new` の段階で全サンプルについての境界検証が行われていない。

## 設計方針

1. **`Mp4SampleReader::new` 側で全サンプルを事前検証する**（方針 1 を採用する。理由は後述）。
2. 検証ロジック: 各サンプル `(data_offset: u64, data_size: usize, ...)` について:
   - `data_offset.checked_add(data_size as u64)` がオーバーフローするか、またはその結果が `file_data.len() as u64` を超える場合、`Mp4Error::InconsistentSampleTable` を返す。
   - 検証は `samples` ベクタ構築後、`cumulative_us` 計算の前に行う。全サンプル数分の線形走査は既存の `cumulative_us` 計算と同オーダーであり、初期化コスト増は無視できる。
3. `get_sample` のシグネチャは変更しない（`Mp4EncodedSample` をそのまま返す）。事前検証済みのため、`get_sample` 側では範囲外が発生しない前提で動作する。
4. 新たに `Mp4Error::InconsistentSampleTable` バリアントを追加する:
   - フィールド: `index: usize`, `offset: u64`, `size: usize`, `file_size: usize`
   - `Display` 実装: `"サンプルテーブルに不整合があります: sample={index} offset={offset} size={size} file_size={file_size}"`（既存の日本語メッセージパターンに合わせる）
   - `Error::source()` アーム: 既存の `None` を返すアームに追記する
   - 上位の `Error::Mp4` への変換は既存の `From<Mp4Error> for Error` 実装が `err.to_string()` で `reason` を生成するため、新規バリアントの追加のみで伝搬する
5. **方針 2（`get_sample` を `Result` 化する）を採用しない理由**:
   - フィーダースレッド（`mp4.rs:731`）でのエラーハンドリング設計（スレッド停止 / ログ出力 / フレームスキップ）が複雑化する。
   - コンストラクタ検証は読み込み時点で不正 MP4 を弾ける（早期失敗）。#0060 の実装とも一貫する。
6. `cumulative_duration_us`（`mp4.rs:445`）も同型の unchecked indexing を持つが、呼び出し元が `i + 1 <= reader.len()` を保証しているため本 issue のスコープ外とする。

## 完了条件

- 不整合な `stsz` / `stco` / `co64` を持つ MP4 を `Mp4SampleReader::new` に渡しても panic せず、`sora_sdk::Error::Mp4` として呼び出し側に返る。
- `Mp4Error::InconsistentSampleTable` バリアントが追加され、`Display` と `Error::source()` の match arm が追加されている。
- `cargo test --workspace` に、サンプルテーブル不整合を含む MP4 フィクスチャを使った単体テストが追加されている。
  - テストの配置先は `tests/test_mp4.rs`（公開 API のテスト）。`Mp4SampleReader::new` 経由でテストする。
  - フィクスチャは既存 H.264 フィクスチャ（`src/video_codecs/testdata/archive-red-320x320-h264.mp4`）のバイト列を操作する。`tests/test_mp4.rs` からは `include_bytes!("../src/video_codecs/testdata/archive-red-320x320-h264.mp4")` で読み込む。
  - `stco` ボックスのチャンクオフセットをファイルサイズ超えに書き換える方式を基本とする。`stsz` のサイズ書き換えを併用してもよい。既存の `sample_reader_rejects_invalid_length_size_minus_one` テスト（`mp4.rs:961`）と同様に、修正対象のバイトオフセットを `assert_eq!` で確認してから書き換えるパターンを使う。
  - 最低限、以下の 3 パターンをカバーする:
    1. `data_offset` 単体が `file_data.len()` を超える（`>` で判定）
    2. `data_offset + data_size` が `file_data.len()` を超える（`>` で判定）
    3. `data_offset.checked_add(data_size as u64)` が `None` を返す（`u64` 加算オーバーフロー）
  - `shiguredo_mp4` の `demuxer.next_sample()` が `stco` の範囲外値に対して先に `DemuxError` を返す場合、上記パターンが `InconsistentSampleTable` ではなく `Mp4Error::Demux` に変換される可能性がある。実装時に `next_sample()` の内部検証の有無を確認し、必要に応じて `stsz` のサイズだけを不正にする方式に切り替える。
- `cargo fmt --all -- --check` が通る。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
