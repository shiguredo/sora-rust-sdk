# validate_video_codec_preference の引数を &[Box<dyn VideoCodecCapability>] に変更する

Completed: 2026-04-07

## 概要

`validate_video_codec_preference` の引数が `&Vec<Box<dyn VideoCodecCapability>>` になっている。
公開 API なので `&[Box<dyn VideoCodecCapability>]` にすべき。

## 該当箇所

- `src/video_codec_preference.rs:186`

## 優先度

中

## 解決方法

`validate_video_codec_preference` の公開シグネチャを
`&Vec<Box<dyn VideoCodecCapability>>` から `&[Box<dyn VideoCodecCapability>]` に変更した。
あわせて、slice を明示的に渡す単体テストを追加し、API の回帰を防止した。
