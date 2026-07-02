# `Mp4Error` を `Error` へ統合する

- Priority: High
- Created: 2026-07-02
- Completed: {YYYY-MM-DD}
- Model: DeepSeek V4 Pro
- Branch: feature/change-integrate-mp4-error
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Must 派生 issue M7。

## 目的

`Mp4Error` は `sora_sdk` のエラー型体系から独立した型として定義されており、`Error` 列挙型に統合されていない。また `Display` 実装が英語で書かれている。正式リリース前に `Error` へ統合し、`Display` を日本語化する。

## 優先度根拠

- 正式リリース後では `Error` 列挙型のバリアント追加は破壊的変更にあたる
- `/review-code` の致命的指摘の一つ

## 現状

`src/video_codecs/mp4.rs:37-43` で `pub enum Mp4Error` が独立して定義されている。
`src/lib.rs:51` で `pub use` により個別に再エクスポートされている。

`Mp4Error` のバリアント:

- `Io(io::Error)` - MP4 ファイル読み込みエラー
- `Demux(shiguredo_mp4::demux::DemuxError)` - デマルチプレクサエラー
- `NoVideoTrack` - 映像トラックなし
- `NoVideoSamples` - 映像サンプルなし
- `UnsupportedVideoCodec` - 未対応コーデック

`Display` 実装はすべて英語で書かれており、プロジェクト規約の「ログメッセージは英語」とは別に、エラーメッセージも英語のままになっている。
他の `Error` バリアントの `Display` は日本語で実装されている。

`Error` 列挙型（`src/error.rs:10-171`）には `Mp4` 関連のバリアントや `From<Mp4Error>` 実装が存在しない。

なお `shiguredo_mp4` は常時有効（`Cargo.toml:92` で optional 指定なし）のため、feature ゲートは不要。

## 設計方針

1. `Error` 列挙型に `Mp4` バリアントを追加し、`Mp4Error` を内包する
2. `From<Mp4Error> for Error` を実装する
3. `Error::Mp4` の `Display` を日本語で実装する
4. `Mp4Error` 自体は `pub(crate)` に変更して内部利用を継続する（または削除する）
5. `mp4.rs` 内の `type Result<T> = std::result::Result<T, Mp4Error>` を適切に対応する

## 完了条件

- `Error` 列挙型に `Mp4` バリアントが追加されている
- `From<Mp4Error> for Error` が実装されている
- `Error::Mp4` の `Display` が日本語で実装されている
- 全テストが通る
