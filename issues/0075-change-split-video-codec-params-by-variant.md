# `Video::Video` のコーデックパラメータをバリアント分割して排他を型で保証する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-split-video-codec-params-by-variant
- Polished: 2026-07-27

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

2. `Video::new_vp8` / `new_vp9` / `new_av1` / `new_h264` / `new_h265` のコンストラクタは維持し、それぞれ対応するバリアントを返す。
   - コンストラクタの引数名 (`vp9_params` 等) は新バリアントのフィールド名 (`params`) に合わせて変更する。
3. `DisplayJson for Video` を各バリアント用に書き換える。
   - 各 variant のアームで `"codec_type": "VP9"` 等の JSON 文字列を出力する（`VideoCodecType::DisplayJson` を再利用する）。
   - 対応する `params` フィールドのみを出力し、JSON キー名 (`"vp9_params"` / `"h264_params"` 等) と出力形式は変更前と同一に保つ。
4. `Audio` も同様に `Audio::Audio { codec_type, opus_params }` を `Audio::Opus { bit_rate, params }` に分割し、`Video` と対称な構造に整理する。
   - 新バリアント `Audio::Opus` から `codec_type` フィールドを削除する（コーデックは Opus 固定のため）。
   - `DisplayJson` 実装では `AudioCodecType::OPUS` を参照して `"codec_type": "OPUS"` を出力し、出力形式を変更前と同一に保つ。
   - `new_opus` コンストラクタの引数名を `opus_params` から `params` に変更する。
5. SKILL.md / README のサンプルコードを更新する。
6. `CHANGES.md` の `## develop` に `[CHANGE]` を追記する（公開 API 破壊的変更のため）。

なお、本 issue のバリアント名 `Vp8` / `Vp9` / `H264` / `H265` / `Av1` は issue 0072 (VideoCodecType の命名矯正) の完了を前提とする。issue 0072 が先に実装されることが望ましい（`issues/0072-change-unify-video-codec-type-with-webrtc.md:46` 参照）。

## 完了条件

- `Video::Video { codec_type: Some(VP8), h264_params: Some(...) }` のような矛盾する組合せがコンパイル不可になる。
- 全コンストラクタが対応するバリアントを返す。
- `DisplayJson for Video` の出力形式が変更前と同一である（JSON キー名・値の形式に変化がない）。
- `Audio` が `Video` と対称な構造（`Audio::Opus { bit_rate, params }`）になり、コンストラクタが対応するバリアントを返す。`DisplayJson for Audio` の出力形式が変更前と同一である。
- SKILL.md / README のサンプルコードが新バリアントに追従している。
- `CHANGES.md` の `## develop` に `[CHANGE]` エントリが追記されている。
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace` が通る。
