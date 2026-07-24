# `Mp4EncodedSample` の公開 API を最小化する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-hide-mp4-encoded-sample-public-api
- Polished: {YYYY-MM-DD}

## 目的

`Mp4EncodedSample` は `pub struct` でフィールドがすべて `pub`、`#[non_exhaustive]` も付いていない。ユーザーが直接構築する必要がなく、`Mp4SampleReader::get_sample()` が返すだけの型だが、`get_sample` は現状 `pub(crate)` 相当で外部から呼べないため、公開する意味自体が薄い。公開範囲を最小化して将来のフィールド追加を後方互換で行えるようにする。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。正式リリース後は「pub フィールドを private に変える」「フィールドを追加する」が破壊的変更となり、SemVer 上の major bump が必要になる。

## 現状

`src/video_codecs/mp4.rs:87-100`:

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

以下のいずれか:

- **案 A (推奨)**: `Mp4EncodedSample` を `pub(crate)` に降格し、`lib.rs` の `pub use` から除去する。ユーザーが直接触る必要がなければこれで十分。
- **案 B**: `pub` を維持しつつ、フィールドをすべて `pub(crate)` に変えて `accessor` メソッドで公開。`#[non_exhaustive]` を付与。
- **案 C**: `pub` を維持しつつ `#[non_exhaustive]` だけ付与 (最小の変更)。ただしフィールド変更の自由度は残らない。

`Mp4EncodedSample` を利用側 (ユーザー) が構築する必要は無く、`Mp4VideoCapturer` の内部データ搬送用の役割しかないため、案 A が最も適切。ユーザーが将来 API アクセスしたくなった場合は accessor 追加で対応。

## 完了条件

- `Mp4EncodedSample` が `pub(crate)` に降格されている (もしくは `#[non_exhaustive]` + private field + accessor)。
- `lib.rs` の `pub use` から除去されている (案 A 選択時)。
- ユーザーコード (sumomo / e2e-tests) にビルドエラーが出ないこと。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
