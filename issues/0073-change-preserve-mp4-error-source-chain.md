# `Error::Mp4` を `source` 保持型に変更し、エラーチェーンを復活させる

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-preserve-mp4-error-source-chain
- Polished: {YYYY-MM-DD}

## 目的

`Error::Mp4 { reason: String }` は `From<Mp4Error>` で `err.to_string()` に落としており、`std::error::Error::source()` チェーンが完全に切れる。他 feature 系エラー (`Error::Amf { source }`, `Error::Vpl { source }`, `Error::NvCodec { source }`, `Error::V4l2 { source }` 等) は `source` を保持しているのに、MP4 だけ設計が異なる。source チェーンを復活させ、他エラーと同じ扱いにする。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。正式リリース後は後方互換の縛りで固定される。エラーチェーンが切れるとユーザーは MP4 側の原因 (ファイル欠損 / デコード失敗 / 未対応コーデック等) を分岐できず、実運用の障害切り分けに支障が出る。

## 現状

`src/error.rs:287-292`:

```rust
Mp4 { reason: String },
```

`src/error.rs:626-632`:

```rust
impl From<Mp4Error> for Error {
    fn from(err: Mp4Error) -> Self {
        Error::Mp4 { reason: err.to_string() }
    }
}
```

`Error::source()` の match (error.rs:479-509 付近) にも `Error::Mp4` は含まれていない。

一方 `src/video_codecs/mp4.rs:37` の `Mp4Error` は `pub(crate)` で内部型のまま:

```rust
pub(crate) enum Mp4Error {
    Io(io::Error),
    Demux(...),
    NoVideoTrack,
    NoVideoSamples,
    UnsupportedVideoCodec,
}
```

## 設計方針

1. `Mp4Error` を `pub` に格上げし、`lib.rs` から再エクスポートする。
2. `Error::Mp4 { reason: String }` を `Error::Mp4 { source: Mp4Error }` に変更する。
3. `From<Mp4Error> for Error` を `Error::Mp4 { source: err }` に修正する。
4. `Error::source()` の match に `Error::Mp4 { source } => Some(source)` を追加する。
5. `Display` 実装 (error.rs) では従来通り日本語エラーメッセージを出力するが、`source` の Display と重複しないように調整 (例:「MP4 エラー: {source}」)。
6. ユーザーは `matches!(err, Error::Mp4 { source: Mp4Error::NoVideoTrack })` で分岐可能になる。

## 完了条件

- `Error::Mp4` が `source` を保持し、`Error::source()` からエラーチェーンが辿れる。
- `Mp4Error` が公開型として `pub use crate::video_codecs::mp4::Mp4Error;` されている。
- 他 feature 系エラー (`Error::Amf` 等) と設計が揃っている。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
