# MP4 サンプルのメタデータをタプルから構造体に変更する

- Priority: Medium
- Created: 2026-08-10
- Completed: 2026-08-10
- Branch: feature/refactor-mp4-sample-meta
- Polished: {YYYY-MM-DD}
- Updated: 2026-08-10

## 目的

`Mp4SampleReader::samples` のタプルを構造体に変更し、フィールド名で意味を明確にする。

## 現状

`Mp4SampleReader::samples` は `(u64, usize, bool, u64, u32)` の 5 要素タプル（サンプルデータのファイル内オフセット、データサイズ、キーフレームかどうか、decode timestamp、サンプルの長さ）で保持される。
タプルの添え字参照は意味が分かりにくく、フィールドが増えると壊れやすい。

## 設計方針

- タプル `(u64, usize, bool, u64, u32)` を構造体 `Mp4SampleMeta { data_offset, data_size, is_keyframe, duration }` に変更する（単純な置き換え）
  - `timestamp` はどこからも参照されていないため削除する
- 構築（`new_inner` のサンプルループ）と参照（`get_sample`、`cumulative_us` 構築）をフィールド参照に書き換える
- 動作の変更は行わない

## 完了条件

- `samples` が `Mp4SampleMeta` 構造体で保持される
- サンプルの payload、送信順序、タイミングが従来どおりである（回帰テスト）
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- コメントとテストの assertion message は日本語にする

## 解決方法

- `Mp4SampleMeta { data_offset, data_size, is_keyframe, duration }` 構造体を定義し、`samples` を `Vec<(u64, usize, bool, u64, u32)>` から `Vec<Mp4SampleMeta>` に変更した
- `new_inner` のサンプルループ、一括検証ループ、`cumulative_us` 構築、`get_sample` の参照をフィールド参照に書き換えた
- `timestamp` はどこからも参照されていなかったため削除した
- `samples` フィールドの冗長なコメント（構造体側に記載済みの内容）を削除した
- 動作の変更はなく、既存テスト（20 件）がすべて成功することを確認した
