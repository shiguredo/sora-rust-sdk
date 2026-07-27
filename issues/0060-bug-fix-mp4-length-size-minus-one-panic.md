# MP4 の `length_size_minus_one == 2` (reserved) で `length_prefixed_nalu_to_annex_b` が panic するのを解消する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-mp4-length-size-minus-one-panic
- Polished: 2026-07-27

## 目的

ISO/IEC 14496-15 の AVCConfigurationBox / HEVCConfigurationBox の `lengthSizeMinusOne` は 2-bit フィールドで、値 2 は reserved。現在の実装はこの値をそのまま `+1` して `length_prefixed_nalu_to_annex_b` に渡すため、`nal_length_size=3` になり `assert!` で panic する。破損した MP4 や仕様外の値を持つ MP4 でキャプチャースレッドが落ちる経路を塞ぐ。

## 優先度根拠

High。`Mp4SampleReader` は入力の妥当性を保証しない外部 MP4 ファイルを受け入れる公開 API であり、悪意ある or 破損した MP4 でキャプチャースレッドが即クラッシュする。ユーザーは `Result` で失敗をハンドリングしたいはずが、panic 経路のためどうにもならない。

## 現状

`src/video_codecs/mp4.rs:428-431` に以下のアサーションがある:

```rust
fn length_prefixed_nalu_to_annex_b(data: &[u8], nal_length_size: u8) -> Vec<u8> {
    assert!(
        nal_length_size == 1 || nal_length_size == 2 || nal_length_size == 4,
        "nal_length_size must be 1, 2, or 4"
    );
    ...
}
```

`extract_track_info` (mp4.rs:263-313) は `avc1` / `hev1` / `hvc1` の 3 経路すべてで `length_size_minus_one.get() + 1` をそのまま格納しており、値 2 (nal_length_size=3) の入力検証が無い。

## 設計方針

1. `extract_track_info` の段階で `length_size_minus_one` の値を検証し、`0` / `1` / `3` (=nal_length_size 1/2/4) 以外は新たに追加する `Mp4Error::InvalidNalLengthSize` を返す。`UnsupportedVideoCodec` はコーデック自体が未対応の場合のエラーであり、NAL 長サイズの不正には意味が合わないため流用しない。新 variant 追加に伴い `Display` 実装と `Error::source` の match arm も追加する。
2. `length_prefixed_nalu_to_annex_b` 内の `assert!` は `debug_assert!` に降格 (production では前段で弾かれる想定)。
3. 型として `enum NalLengthSize { One, Two, Four }` に置き換えるのが理想だが、破壊的変更を避けるためスコープからは外す (別 issue 化を検討)。

## 完了条件

- `length_size_minus_one == 2` を持つ MP4 を入力しても panic せず、`Mp4Error` として呼び出し側にエラーが返る。
- `length_prefixed_nalu_to_annex_b` の `assert!` が `debug_assert!` に降格されている。
- `cargo test --workspace` に「不正 `length_size_minus_one` のフィクスチャで `Mp4SampleReader::new` が Err を返す」単体テストが追加されている。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
