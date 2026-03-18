# validate_video_codec_preference の引数を &[Box<dyn VideoCodecCapability>] に変更する

## 概要

`validate_video_codec_preference` の引数が `&Vec<Box<dyn VideoCodecCapability>>` になっている。
公開 API なので `&[Box<dyn VideoCodecCapability>]` にすべき。

## 該当箇所

- `src/video_codec_preference.rs:233`

## 優先度

中
