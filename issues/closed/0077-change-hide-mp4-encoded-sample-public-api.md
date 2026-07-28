# `Mp4EncodedSample` の公開 API を最小化する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-28
- Model: Opus 4.7
- Branch: feature/change-hide-mp4-encoded-sample-public-api
- Polished: 2026-07-27

## 目的

`Mp4EncodedSample` は `pub struct` でフィールドがすべて `pub` である。ユーザーが直接構築する必要がなく、`Mp4SampleReader::get_sample()` が返すだけの型だが、`get_sample` は現状 private (`fn`、可視性修飾子なし) で外部から呼べないため、公開する意味自体が薄い。公開範囲を最小化して将来のフィールド追加を後方互換で行えるようにする。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。正式リリース後は「pub フィールドを private に変える」「フィールドを追加する」が破壊的変更となり、SemVer 上の major bump が必要になる。

## 現状

`src/video_codecs/mp4.rs:95-112`:

```rust
pub struct Mp4EncodedSample {
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub width: u32,
    pub height: u32,
    pub codec_type: VideoCodecType,
}
```

`lib.rs:95-97` で `pub use crate::video_codecs::mp4::{ Mp4EncodedSample, ... };` で再エクスポート。

## 設計方針

`Mp4EncodedSample` を `pub(crate)` に降格し、`lib.rs` の `pub use` から除去する。

`Mp4EncodedSample` は `Mp4SampleReader::get_sample()` → `Mp4VideoCapturer` → `Mp4PassthroughEncoder::encode()` の 3 コンポーネント間を native `VideoFrameBuffer` 経由 (unsafe downcast) で受け渡される内部共有型であり、ユーザーが直接構築・参照する必要はない。`pub` のまま維持する理由がなく、ユーザーが将来 API アクセスしたくなった場合は accessor 追加で対応する。

## 解決方法

`Mp4EncodedSample` を `pub(crate)` に降格し、公開 API から削除した。
- `Mp4EncodedSample` の可視性を `pub` → `pub(crate)` に変更。
- `lib.rs` の `pub use` から `Mp4EncodedSample` を除去。


## 完了条件

- `Mp4EncodedSample` が `pub(crate)` に降格されている。
- `lib.rs` の `pub use` から `Mp4EncodedSample` が除去されている。
- `skills/sora-rust-sdk/SKILL.md` の公開 API テーブルから `Mp4EncodedSample` が除去されている。
- ユーザーコード (sumomo / e2e-tests) にビルドエラーが出ないこと。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
