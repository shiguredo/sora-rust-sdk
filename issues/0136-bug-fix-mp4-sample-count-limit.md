# 巨大なサンプル数の MP4 入力を拒否する

- Priority: High
- Created: 2026-08-10
- Completed: 2026-08-10
- Branch: feature/fix-mp4-sample-count-limit
- Polished: 2026-08-10

## 目的

巨大なサンプル数を宣言した MP4 で、`Mp4SampleReader` が長時間ループしないようにする。

## 現状

`stts` は run-length 形式であり、1 entry だけで巨大なサンプル数 (最大 `u32::MAX`) を表現できる。
`Mp4SampleReader::new_inner` は `next_sample()` をサンプル数分ループして全 metadata を `Vec` に展開するため、サンプル数に比例する処理時間とメモリがかかる。

なお、依存 crate は moov box の decode 時に全 sample 分の `sample_data_offsets` を eager に構築するため、巨大なサンプル数を宣言した入力では、SDK 側の検証へ到達する前に依存 crate 内で大きな allocation が発生し得る。
本 issue の上限チェックは、その allocation を通過できるサンプル数に対する SDK 側のループ回数と metadata のメモリを制限する。

## 設計方針

- `MAX_SAMPLE_COUNT_PER_TRACK = 10_368_000`（120 fps を 24 時間保持できる件数）を設け、上限値ちょうどを受理し、1 超過を拒否する
- `next_sample()` のループでサンプル数を数え、上限超過を検出した時点で `SampleCountLimitExceeded` error（sample index 付き）を返す
- 上限判定は sample index を受け取る pure helper に抽出し、helper は上限値ちょうどを受理して 1 超過を拒否する
- 依存 crate が moov box の decode 時に eager に構築する `sample_data_offsets` の allocation は対象外とする（依存 crate の設計による制約）

## 完了条件

- `MAX_SAMPLE_COUNT_PER_TRACK` ちょうどのサンプル数を受理し、1 超過を sample index を含む `SampleCountLimitExceeded` error で拒否する
- 上限判定 helper に、上限値ちょうどの最後の index (`MAX - 1`) を渡して受理し、1 超過の最初の index (`MAX`) を渡して拒否することを確認する（境界テストは実 reader 経路では 10.4M サンプルの構築が必要になるため、helper 経由で検証する）
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- コメントとテストの assertion message は日本語にする

## 対応しない理由

サンプル数のみの上限は、「壊れた MP4 で長時間処理が起きる」問題への対策として中途半端である。
サンプル数が少なくてもサンプルの尺が長い場合に同様の長時間処理が起きるため、サンプル数だけを制限しても実質的な対策にならない。
どのケースが問題になるかは個々の利用シーンに依存し、ライブラリで一律に制限するのは不適切なため、利用側で対処する方針とする。
