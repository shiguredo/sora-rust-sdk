# `Error::Mp4` を `source` 保持型に変更し、エラーチェーンを復活させる

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-28
- Model: Opus 4.7
- Branch: feature/change-preserve-mp4-error-source-chain
- Polished: 2026-07-27

## 目的

`Error::Mp4 { reason: String }` は `From<Mp4Error>` で `err.to_string()` に落としており、`std::error::Error::source()` チェーンが完全に切れる。他エラー (`Error::Amf { source }`, `Error::Vpl { source }`, `Error::V4l2 { source }` 等) は `source` を保持し `Error::source()` から辿れるのに、MP4 だけ設計が異なる。source チェーンを復活させ、これらのエラーと同じ扱いにする。

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

一方 `src/video_codecs/mp4.rs:37-46` の `Mp4Error` は `pub(crate)` で内部型のまま:

```rust
pub(crate) enum Mp4Error {
    Io(io::Error),
    Demux(shiguredo_mp4::demux::DemuxError),
    NoVideoTrack,
    NoVideoSamples,
    UnsupportedVideoCodec,
    InvalidNalLengthSize(u8),
}
```

## 設計方針

1. `Mp4Error` を `pub` に格上げし、`lib.rs` から再エクスポートする。
2. `Error::Mp4 { reason: String }` を `Error::Mp4 { source: Mp4Error }` に変更する。
3. `From<Mp4Error> for Error` を `Error::Mp4 { source: err }` に修正する。
4. `Error::source()` の match に `Error::Mp4 { source } => Some(source)` を追加する。
5. `Display` 実装 (error.rs) では従来通り日本語エラーメッセージを出力するが、`source` の Display と重複しないように調整する。他 feature 系エラーと同じ `"{PRODUCT} error: {source}"` 形式に揃えず、日本語で「MP4 エラー: {source}」とする（`Mp4Error::Display` の各メッセージから「MP4」プレフィックスが除去済みのため、重複しない）。例: `Self::Io(err)` (`"読み込みに失敗しました: {err}"`) と合成されて `"MP4 エラー: 読み込みに失敗しました: No such file"`。
6. `src/video_codecs/mp4.rs` 内のテスト `sample_reader_rejects_invalid_length_size_minus_one` で `Error::Mp4 { reason }` にパターンマッチしている箇所を `Error::Mp4 { source }` に修正する。
7. ユーザーは `matches!(err, Error::Mp4 { source: Mp4Error::NoVideoTrack })` で分岐可能になる。
8. `CHANGES.md` の `## develop` に `[CHANGE]` エントリを追加する。

## 解決方法

`Error::Mp4` を `source` 保持型に変更し、エラーチェーンを復活させた。
- `Error::Mp4 { reason: String }` → `Error::Mp4 { source: Mp4Error }` に変更。
- `Mp4Error` を `pub` に格上げし、`lib.rs` から再エクスポート。
- `Error::source()` の match に `Error::Mp4 { source } => Some(source)` を追加。
- `From<Mp4Error> for Error` を `Error::Mp4 { source: err }` に修正。
- 既存テストのパターンマッチを新しいバリアント形式に更新した。


## 完了条件

- `Error::Mp4` が `source: Mp4Error` フィールドでエラー型を保持し、`Error::source()` からエラーチェーンが辿れるようになっている（他エラーの `source` 保持パターンと一致）。
- `Mp4Error` が公開型として `pub use crate::video_codecs::mp4::Mp4Error;` されている。
- `src/video_codecs/mp4.rs` 内のテスト `sample_reader_rejects_invalid_length_size_minus_one` が新しいバリアント形式 (`Error::Mp4 { source }`) に修正されている。
- `CHANGES.md` の `## develop` に `[CHANGE]` エントリが追加されている。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
