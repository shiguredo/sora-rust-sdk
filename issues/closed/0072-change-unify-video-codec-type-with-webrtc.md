# `sora_sdk::VideoCodecType` の命名を Rust 規約に矯正し、`shiguredo_webrtc::VideoCodecType` との相互変換を整備する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-28
- Model: Opus 4.7
- Branch: feature/change-unify-video-codec-type-with-webrtc
- Polished: 2026-07-27

## 目的

`sora_sdk::VideoCodecType` (`VP8, VP9, H264, H265, AV1` 全大文字) と `shiguredo_webrtc::VideoCodecType` (`Vp8, Vp9, H264, H265, Av1`) の 2 つの同名別型が公開 API に混在している。両者は役割が異なるため、別型のまま以下の対応を行う:

1. `sora_sdk::VideoCodecType` のバリアント命名を Rust 規約 (RFC 430、UpperCamelCase) に矯正する (`VP8` → `Vp8`, `AV1` → `Av1`)
2. 両型の相互 `From` 実装を追加する

ユーザーは `sora_sdk::VideoCodecType` を使って `Video` を構築し、`shiguredo_webrtc::VideoCodecType` を使う `VideoCodecCapability` や `PreferenceCodec` には相互変換で渡す。これにより、ユーザーが `sora_sdk::VideoCodecType::H264` を間違って webrtc 側に渡すコンパイルエラーがなくなり、型の出自と変換ポイントが明示的になる。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。正式リリース後は後方互換の縛りで固定される。`sora_sdk` 側は Rust 命名規約 (RFC 430、UpperCamelCase) 違反であり、正式版で `VP8` / `AV1` を残すのはコード品質上望ましくない。

## 現状

- `src/types.rs:207-219` に `pub enum VideoCodecType { VP8, VP9, H264, H265, AV1 }` (全大文字、命名規約違反)。
- `src/types.rs:221-231` に `DisplayJson` 実装 (JSON 出力用)。シグナリング層の `Video.codec_type` フィールドで利用。
- `src/video_codecs/*.rs`, `src/video_codec.rs`, `src/video_codec_capability.rs`, `src/video_codec_preference.rs` はすべて `shiguredo_webrtc::VideoCodecType` (`Vp8`/`Vp9`/`H264`/`H265`/`Av1`) を利用。
- `VideoCodecCapability::is_supported(direction, codec_type: VideoCodecType)` などトレイト API は webrtc 側の型を要求。
- `PreferenceCodec::new(direction, codec_type: VideoCodecType, ...)` は webrtc 側の型を要求。
- `VideoCodecPreference` の `validate_video_codec_preference` も webrtc 側の型で動作。
- `connection.rs` と `signaling_types.rs` は `VideoCodecType` に直接触れず、`Video` 構造体全体を流している。
- 2 種類の同名型がユーザー面に露出しており、ユーザーは間違った型を渡してコンパイルエラーになる。

## 設計方針

**案 B (採用)**: `crate::types::VideoCodecType` を残しつつ命名を矯正し、`shiguredo_webrtc::VideoCodecType` との相互 `From` 実装を追加する。

採用理由:
- `sora_sdk::VideoCodecType` はシグナリング層 (`DisplayJson` による JSON シリアライズ、`Video` 構造体のフィールド) の関心事
- `shiguredo_webrtc::VideoCodecType` は WebRTC 層 (SDP ネゴシエーション、コーデック実装) の関心事
- 両者の用途は異なるため、別型のまま相互変換を明示的に用意することで、どこで層をまたぐかが明確になる
- 単一型への統一や再エクスポートでは型の出自が曖昧になり、かえって混乱する

### 他 issue との依存関係

[0075](0075-change-split-video-codec-params-by-variant.md) (`Video::Video` のバリアント分割) が先に完了すると `codec_type` フィールドが削除され、本 issue の対応内容 3 (`new_vp8()` 等のバリアント名更新) が対象を失う。本 issue を先に実装することが望ましい。

### 対応内容

1. `sora_sdk::VideoCodecType` のバリアント名を `Vp8` / `Vp9` / `H264` / `H265` / `Av1` に矯正
2. `DisplayJson` 実装のマッチアームを新しいバリアント名に更新 (JSON 出力文字列 `"VP8"` / `"VP9"` / `"H264"` / `"H265"` / `"AV1"` は維持)
3. `Video::new_vp8()` / `Video::new_av1()` 等のコンストラクタで新しいバリアント名を使用
4. `From<sora_sdk::VideoCodecType> for shiguredo_webrtc::VideoCodecType` を `src/types.rs` に実装
5. `From<shiguredo_webrtc::VideoCodecType> for sora_sdk::VideoCodecType` を `src/types.rs` に実装（両方の方向で変換可能にする）
6. `src/types.rs` 内の `#[cfg(test)]` インラインテストでバリアント名を更新（JSON 期待値は変更しない）
7. `sora-rust-sdk/SKILL.md` の `VideoCodecType` 列挙テーブル (バリアント名を記載している行) を更新

`AudioCodecType::OPUS` → `Opus` の命名矯正は本 issue から分離し、[0084](0084-change-fix-audio-codec-type-naming.md) で対応する。

## 解決方法

`sora_sdk::VideoCodecType` のバリアント名を Rust 命名規約 (UpperCamelCase) に矯正した。
- `VP8` → `Vp8`, `VP9` → `Vp9`, `AV1` → `Av1` に変更。`H264` / `H265` は変更なし。
- `DisplayJson` 実装のマッチアームを更新（JSON 出力文字列は維持）。
- `From<sora_sdk::VideoCodecType> for shiguredo_webrtc::VideoCodecType` と逆向きの `From` 実装を追加した。
- `Video::new_vp8()` / `Video::new_av1()` 等のバリアント名を更新。


## 完了条件

- `sora_sdk::VideoCodecType` のバリアント名が Rust 命名規約に沿っている (`Vp8` / `Vp9` / `H264` / `H265` / `Av1`)
- `sora_sdk::VideoCodecType` ↔ `shiguredo_webrtc::VideoCodecType` の相互 `From` 実装が存在する
- `Video::new_vp8()` / `Video::new_h264()` 等のコンストラクタが新しいバリアント名で動作する
- `DisplayJson` の出力文字列 (`"VP8"` / `"VP9"` / `"H264"` / `"H265"` / `"AV1"`) が変更前と同一である
- `sora-rust-sdk/SKILL.md` の列挙テーブルが更新されている
- `cargo clippy --workspace --all-features -- -D warnings` が通る
- `cargo test --workspace` が通る
