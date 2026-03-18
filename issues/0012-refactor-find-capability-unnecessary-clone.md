# find_capability で不要な implementation.clone() を削除する

## 概要

`find_capability` 内で `implementation.clone()` を行っているが、
`VideoCodecImplementation` の `PartialEq` は `name` のみで比較しているため、
`&VideoCodecImplementation` 同士の参照比較で十分。`.clone()` は無駄なアロケーション。

## 該当箇所

- `src/video_codec.rs:164`
- `src/video_codec_preference.rs:348`

## 優先度

低
