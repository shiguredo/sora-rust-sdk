# MP4 サンプルのメタデータをタプルから構造体に変更する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-mp4-sample-meta
- Polished: {YYYY-MM-DD}

## 目的

`Mp4SampleReader::samples` のタプルを構造体に変更し、フィールド名で意味を明確にする。

## 現状

issue 0098 の対応後、`samples` は `(Range<usize>, bool, u32)` の 3 要素タプル（サンプルデータのファイル内範囲、キーフレームかどうか、サンプルの長さ）で保持される。
タプルの添え字参照は意味が分かりにくく、フィールドが増えると壊れやすい。

## 設計方針

- タプル `(Range<usize>, bool, u32)` を構造体 `Mp4SampleMeta { data_range, is_keyframe, duration }` に変更する（単純な置き換え）
- 構築（`new_inner` のサンプルループ）と参照（`get_sample`、`build_cumulative_us` 用の duration 収集）をフィールド参照に書き換える
- 動作の変更は行わない

## 完了条件

- `samples` が `Mp4SampleMeta` 構造体で保持される
- サンプルの payload、送信順序、タイミングが従来どおりである（回帰テスト）
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- コメントとテストの assertion message は日本語にする
