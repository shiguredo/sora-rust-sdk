# `sora_sdk::VideoCodecType` と `shiguredo_webrtc::VideoCodecType` の重複を解消する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-unify-video-codec-type-with-webrtc
- Polished: {YYYY-MM-DD}

## 目的

`sora_sdk::VideoCodecType` (`VP8, VP9, H264, H265, AV1` 全大文字) と `shiguredo_webrtc::VideoCodecType` (`Vp8, Vp9, Av1, ...`) が **別型で公開 API に混在** している。同名別型で命名規約も違うため、ユーザーは `sora_sdk::VideoCodecType::H264` を渡してコンパイルエラー、`shiguredo_webrtc` を明示 import させられる。正式リリース前に 1 種類に統一する。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。正式リリース後は後方互換の縛りで固定される。同時に `sora_sdk` 側は Rust 命名規約 (RFC 430、UpperCamelCase) 違反でもあり、正式版で `VP8` を残すのはコード品質上望ましくない。

## 現状

- `src/types.rs:207-219` に `pub enum VideoCodecType { VP8, VP9, H264, H265, AV1 }` (全大文字、命名規約違反)。`#[allow(clippy::upper_case_acronyms)]` の抑止すら無く clippy がエラーを吐く可能性。
- 一方 `src/video_codecs/*.rs` は `shiguredo_webrtc::VideoCodecType::Vp8` などを使用。
- `VideoCodecCapability::is_supported(direction, codec_type: VideoCodecType)` などのトレイト API は webrtc 側の型を要求。
- `sora_sdk::Video { codec_type: Option<VideoCodecType>, ... }` は SDK 側の型を要求。
- 2 種類の同名型がユーザー面に露出。

## 設計方針

以下のいずれかを選択:

- **案 A (推奨)**: `crate::types::VideoCodecType` を廃止し、`shiguredo_webrtc::VideoCodecType` を `pub use` で単一の型として再エクスポート。命名は webrtc 側の `Vp8` / `Vp9` / `H264` / `H265` / `Av1` に統一。
- **案 B**: `crate::types::VideoCodecType` を残しつつ命名だけ矯正 (`Vp8` 等) し、`shiguredo_webrtc` 側との相互変換を SDK 内で完全に隠蔽。webrtc 型を受け取る公開 API を SDK 型に置き換える。
- **案 C**: `crate::types::VideoCodecType` をリネーム (例: `SignalingVideoCodecType`) して意味を分ける。

案 A が破壊的変更としては最もクリーンで、SDK が webrtc 型を露出する現状 (公開 API に `RtpTransceiver` などが出ている) との整合性も取れる。

同時に `AudioCodecType::OPUS` も `Opus` に矯正する。SKILL.md の記載も更新する。

## 完了条件

- `sora_sdk::VideoCodecType` が 1 種類のみ (webrtc 側と一致) に統一されている。
- 命名が Rust 命名規約に沿っている (`Vp8` / `Vp9` / `H264` / `H265` / `Av1` / `Opus`)。
- `sora_sdk::VideoCodecType` を経由してユーザーが `Video::new_h264(...)` や `PreferenceCodec::new(direction, codec_type, impl)` を書ける。
- SKILL.md / README のコード例が更新されている。
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace` が通る。
