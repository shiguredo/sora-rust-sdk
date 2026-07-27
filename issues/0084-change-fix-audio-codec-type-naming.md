# `sora_sdk::AudioCodecType` のバリアント命名を Rust 規約に矯正する

- Priority: High
- Created: 2026-07-27
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-fix-audio-codec-type-naming
- Polished: {YYYY-MM-DD}

## 目的

`sora_sdk::AudioCodecType::OPUS` は全大文字で Rust 命名規約 (RFC 430、UpperCamelCase) に違反している。`VideoCodecType` の矯正 (issue 0084 の親にあたる 0072) と同様に `Opus` に矯正する。

issue 0072 から AudioCodecType 部分を分離した派生 issue。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。正式リリース後は後方互換の縛りで固定される。Rust 命名規約違反を正式版に残すのはコード品質上望ましくない。また `VideoCodecType` と同根の理由であり、同時期に対応するのが妥当。

## 現状

- `src/types.rs:235-237` に `pub enum AudioCodecType { OPUS }` (全大文字、命名規約違反)。
- `src/types.rs:240-245` に `DisplayJson` 実装。JSON 出力文字列は `"OPUS"`。
- `src/types.rs:249-` の `Audio` 構造体に `codec_type: Option<AudioCodecType>` フィールドがある。
- `src/lib.rs:76` で `AudioCodecType` が再エクスポートされている。

## 設計方針

- `AudioCodecType::OPUS` を `AudioCodecType::Opus` にリネーム
- `DisplayJson` 実装のマッチアームを `Opus` に更新 (JSON 出力文字列 `"OPUS"` は維持)
- `Audio` の `new_opus()` コンストラクタ内のバリアント名を更新
- `src/types.rs` 内の `#[cfg(test)]` インラインテストでバリアント名を更新 (JSON 期待値は変更しない)
- `sora-rust-sdk/SKILL.md` の `AudioCodecType` 列挙テーブルを更新

## 完了条件

- `sora_sdk::AudioCodecType` のバリアント名が `Opus`
- `DisplayJson` の出力文字列が変更前と同一 (`"OPUS"`)
- `sora-rust-sdk/SKILL.md` の列挙テーブルが更新されている
- `cargo clippy --workspace --all-features -- -D warnings` が通る
- `cargo test --workspace` が通る
