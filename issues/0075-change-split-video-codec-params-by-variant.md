# `Video::Video` のコーデックパラメータをバリアント分割して排他を型で保証する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-split-video-codec-params-by-variant
- Polished: {YYYY-MM-DD}

## 目的

`Video::Video { codec_type: Option<VideoCodecType>, bit_rate, vp9_params: Option<VideoVP9Params>, av1_params, h264_params, h265_params }` は Option×4 で「1 種のコーデックパラメータのみが有効」という排他が型で保証されていない。`codec_type: Some(VP8)` + `h264_params: Some(...)` を同時指定してもコンパイル可能。バリアント分割で型安全にする。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。`Video::new_h264` などのコンストラクタは正しく排他するが、pub struct-like variant のため直接構築されれば防げない。バリアント分割は正式リリース後は後方互換の縛りで実施困難。

## 現状

`src/types.rs:585-668` 付近:

```rust
pub enum Video {
    Bool(bool),
    Video {
        codec_type: Option<VideoCodecType>,
        bit_rate: Option<u32>,
        vp9_params: Option<VideoVP9Params>,
        av1_params: Option<VideoAV1Params>,
        h264_params: Option<VideoH264Params>,
        h265_params: Option<VideoH265Params>,
    },
}
```

`DisplayJson` 実装は指定された Option をすべて出力するため、`codec_type=VP8` + `h264_params=Some(...)` の入力を静かに受理してしまう。

## 設計方針

1. `Video::Video { ... }` をコーデック別バリアントに分割:

   ```rust
   pub enum Video {
       Bool(bool),
       Vp8  { bit_rate: Option<u32> },
       Vp9  { bit_rate: Option<u32>, params: Option<VideoVP9Params> },
       H264 { bit_rate: Option<u32>, params: Option<VideoH264Params> },
       H265 { bit_rate: Option<u32>, params: Option<VideoH265Params> },
       Av1  { bit_rate: Option<u32>, params: Option<VideoAV1Params> },
   }
   ```

2. 全バリアントに `#[non_exhaustive]` を付与 (issue 0078 と合わせて実装)。
3. `Video::new_vp8` / `new_vp9` / `new_av1` / `new_h264` / `new_h265` のコンストラクタは維持し、それぞれ対応するバリアントを返す。
4. `Audio` も同様の課題があるか調査し、対称に整理する。
5. SKILL.md / README のサンプルコードを更新する。

## 完了条件

- `Video::Video { codec_type: Some(VP8), h264_params: Some(...) }` のような矛盾する組合せがコンパイル不可になる。
- 全コンストラクタが対応するバリアントを返す。
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace` が通る。
