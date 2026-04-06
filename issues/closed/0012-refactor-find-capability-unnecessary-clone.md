# find_capability で不要な implementation.clone() を削除する

Created: 2026-03-18
Completed: 2026-04-07
Model: GPT-5.4

## 概要

`find_capability` 内で `implementation.clone()` を行っているが、
`VideoCodecImplementation` の `PartialEq` は `name` のみで比較しているため、
`&VideoCodecImplementation` 同士の参照比較で十分。`.clone()` は無駄なアロケーション。

## 該当箇所

- `src/video_codec.rs:145`
- `src/video_codec_preference.rs:304`

## 優先度

低

## 解決方法

`find_capability` の比較を `implementation.clone()` ベースから
`implementation.name()` ベースに変更し、不要な clone を削除した。
