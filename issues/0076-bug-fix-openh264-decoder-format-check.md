# `Openh264VideoCodecCapability::create_video_decoder` が format を検査せず常に H.264 デコーダを返す問題を修正する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-openh264-decoder-format-check
- Polished: {YYYY-MM-DD}

## 目的

`Openh264VideoCodecCapability::create_video_decoder` は `#[expect(unused_variables)]` で `format` を完全無視し、いかなる format が渡っても H.264 デコーダを返す。`format.name()` を検査して H.264 以外は `None` を返すよう修正する。

## 優先度根拠

High。API 契約違反かつ実行時破綻の危険。通常は `SoraVideoDecoderFactory::create` が preference で選ばれた実装から呼ぶため VP8 等が渡ることは無いが、ユーザーが直接 `create_video_decoder(env, VP8_format)` を呼んだ場合、不正な H.264 デコーダが返り実行時に破綻する。他 backend (`Mp4PassthroughVideoCodecCapability` 等) は format 検査で `None` を返す実装があり、対称性が崩れている。

## 現状

`src/video_codecs/openh264.rs:537-547`:

```rust
#[expect(unused_variables)]
fn create_video_decoder(
    &self,
    env: shiguredo_webrtc::EnvironmentRef<'_>,
    format: SdpVideoFormatRef<'_>,
) -> Option<VideoDecoder> {
    Some(VideoDecoder::new_with_handler(Box::new(
        Openh264VideoDecoder::new(self.library.clone()),
    )))
}
```

`format` を検査しないため、VP8 / VP9 / AV1 等が渡っても H.264 デコーダが返る。

## 設計方針

1. `format.name()` を取得し、`"H264"` (もしくは対応する VideoCodecType) 以外は `None` を返す。
2. `#[expect(unused_variables)]` を除去する。
3. `create_video_encoder` (openh264.rs:528-535) も同様に format 検査を追加するか、少なくとも `simulcast_capability_helper.create_video_encoder` 内で対応する。
4. 単体テストで `create_video_decoder(env, vp8_format)` が None を返すことを検証する。

## 完了条件

- `Openh264VideoCodecCapability::create_video_decoder` が非 H.264 format に対して `None` を返す。
- `#[expect(unused_variables)]` が除去されている。
- 単体テストで検証済み。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
