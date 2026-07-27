# `Openh264VideoCodecCapability` の encoder/decoder 生成が format を検査しない問題を修正する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-openh264-decoder-format-check
- Polished: 2026-07-27

## 目的

`Openh264VideoCodecCapability` の `create_video_decoder` と encoder builder closure の両方で `format` を検査しておらず、いかなる format が渡っても H.264 コーデックが返る。`codec_type_from_format` で format を検査し、H.264 以外は `None` を返すよう修正する。

## 優先度根拠

High。トレイト仕様上 `get_supported_formats()` で返さないフォーマットが渡された場合の動作は実装依存とされているが、実際に非 H.264 format を渡すと不正な H.264 デコーダが返り実行時に破綻する。通常は `SoraVideoDecoderFactory::create` が preference で選ばれた実装から呼ぶため VP8 等が渡ることは無いが、ユーザーが直接 `create_video_decoder(env, VP8_format)` を呼んだ場合に問題が顕在化する。AMF・VPL・NVCodec の `create_video_decoder` は `codec_type_from_format(&format)?` で format を検査しており、OpenH264 だけが未検査で対称性が崩れている。

## 現状

### create_video_decoder

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

### encoder builder closure

`src/video_codecs/openh264.rs:506-513`:

```rust
move |_env, _format| {
    Some(VideoEncoder::new_with_handler(Box::new(
        Openh264VideoEncoder::new(library.clone()),
    )))
}
```

同様に `_format` を無視している。V4L2・AMF・VPL・NVCodec の同 builder closure は既に `codec_type_from_format(&format)?` で検査済み。

## 設計方針

1. `codec_type_from_format(&format)?` で `VideoCodecType` を取得し、`VideoCodecType::H264` 以外であれば `None` を返す（AMF・VPL・NVCodec の `create_video_decoder` と同じパターン。`codec_type_from_format` は `src/video_codec.rs:421` で定義されている既存ヘルパー）。
2. `#[expect(unused_variables)]` を除去する。
3. `new()` 内の builder closure (openh264.rs:508) でも format 検査を追加する。`create_video_encoder` メソッド自体は `simulcast_capability_helper` へ委譲しているだけであり、実際に `_format` を無視しているのは builder closure 側（`|_env, _format|`）のため。V4L2・AMF・VPL・NVCodec は同 closure 内で `codec_type_from_format(&format)?` + `VideoCodecType::H264` チェックを既に行っている。
4. 単体テストで `create_video_decoder(env, vp8_format)` が None を返すことを検証する。

## 完了条件

- `Openh264VideoCodecCapability::create_video_decoder` が非 H.264 format に対して `None` を返す。
- encoder builder closure が非 H.264 format に対して `None` を返す。
- `#[expect(unused_variables)]` が除去され、builder closure の `_format` が `format` になっている。
- 単体テストで decoder と encoder の両方について非 H.264 format で `None` が返ることを検証している。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
