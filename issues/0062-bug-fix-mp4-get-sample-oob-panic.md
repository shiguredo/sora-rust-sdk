# MP4 の `get_sample` で不整合な `stsz` / `stco` を持つファイルによる範囲外 panic を解消する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-mp4-get-sample-oob-panic
- Polished: {YYYY-MM-DD}

## 目的

`Mp4SampleReader::get_sample` が `self.samples[index]` と `file_data[data_offset..data_offset+data_size]` を無防備にインデックスするため、壊れた MP4 (`stsz` / `stco` に不整合がある等) で range panic する。フェッチャースレッド (`Mp4VideoCapturer` の feeder) がクラッシュする経路になっており、`Result` を経由した安全な失敗経路に置き換える。

## 優先度根拠

High。`Mp4VideoCapturer` はユーザーが指定した MP4 ファイルを読み込んで送信する公開 API 経路。悪意ある / 破損した MP4 でフィーダースレッドが落ちるとキャプチャが止まり、ユーザーは原因を知る術がない。

## 現状

`src/video_codecs/mp4.rs:382-408` 付近で、以下のようにサンプルデータを取り出している:

```rust
let (data_offset, data_size, keyframe, _, _) = self.samples[index];
let raw_data = &self.file_data[data_offset as usize..data_offset as usize + data_size];
```

- `data_offset + data_size` の `usize` オーバーフローは未チェック。
- `data_offset + data_size` が `file_data.len()` を超える場合スライスで panic。
- `Mp4SampleReader::new` の段階で全サンプルについての境界検証が行われていない。

## 設計方針

1. `Mp4SampleReader::new` で全サンプルについて `data_offset.checked_add(data_size as u64).map(|end| end <= file_data.len() as u64)` を検証し、範囲外のサンプルがあれば `Mp4Error` を返す。
2. あるいは `get_sample` 側で `file_data.get(range)` を使い、`Option` を経由してエラーを返すよう `get_sample` 自体を `Result` 化する。
3. どちらのアプローチでも panic には至らないことを、単体テストで担保する。
4. コンストラクタ側で検証する方が「読み込み時点で不正 MP4 を弾ける」ため望ましいが、`file_data` が非常に大きい場合の初期化コストを考慮して選択する。

## 完了条件

- 不整合な `stsz` / `stco` を持つ MP4 を渡しても `Mp4SampleReader::new` または `get_sample` が panic せず、`Mp4Error` として呼び出し側に返る。
- `cargo test --workspace` に該当ケースのフィクスチャを使った単体テストが追加されている。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
