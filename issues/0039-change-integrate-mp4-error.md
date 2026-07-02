# `Mp4Error` を `Error` へ統合する

- Priority: High
- Created: 2026-07-02
- Completed: {YYYY-MM-DD}
- Model: DeepSeek V4 Pro
- Branch: feature/change-integrate-mp4-error
- Polished: 2026-07-02

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Must 派生 issue M7。

## 目的

`Mp4Error` は `sora_sdk` のエラー型体系から独立した型として定義されており、`Error` 列挙型に統合されていない。また `Display` 実装が英語で書かれている。正式リリース前に `Error` へ統合し、`Display` を日本語化する。

## 優先度根拠

- 正式リリース後では `Error` 列挙型のバリアント追加は破壊的変更にあたる

## 現状

`src/video_codecs/mp4.rs:37-43` で `pub enum Mp4Error` が独立して定義されている。
`src/lib.rs:51` で `pub use` により個別に再エクスポートされている。

`Mp4Error` のバリアント:

- `Io(io::Error)` - MP4 ファイル読み込みエラー
- `Demux(shiguredo_mp4::demux::DemuxError)` - デマルチプレクサエラー
- `NoVideoTrack` - 映像トラックなし
- `NoVideoSamples` - 映像サンプルなし
- `UnsupportedVideoCodec` - 未対応コーデック

`Display` 実装 (`mp4.rs:45-57`) はすべて英語で書かれている。
`std::error::Error::source()` 実装 (`mp4.rs:59-67`) は既に存在する。
`From<io::Error>` と `From<DemuxError>` の実装 (`mp4.rs:69-79`) も存在する。

`Error` 列挙型（`src/error.rs:10-171`）には `Mp4` 関連のバリアントや `From<Mp4Error>` 実装が存在しない。

`examples/sumomo/src/error.rs` は `AppError::Mp4(Mp4Error)` バリアント（14 行目）と `From<Mp4Error> for AppError`（72-76 行目）を持っており、`Mp4Error` の `pub(crate)` 化でコンパイル不能になる。
`examples/sumomo/src/main.rs:240,359` で `Mp4SampleReader::new()` や `Mp4VideoCapturer::new()` の呼び出しに `?` 演算子を使い `Mp4Error` を伝播している。

`skills/sora-rust-sdk/SKILL.md:254` に `Mp4Error` が公開 API として記載されている。

なお `shiguredo_mp4` は常時有効（`Cargo.toml:92` で optional 指定なし）のため、feature ゲートは不要。

## 設計方針

本変更は完了済みの #0031（`Error::InvalidSystemTime` 追加）と同様のパターンに従い、バリアント追加 → `Display` → `source()` → `From` の 4 点セットで実装する。

### 1. `Error` 列挙型へのバリアント追加

`src/error.rs:10-171` の `Error` 列挙型に、`Mp4` バリアントをタプル形式で追加する。
`shiguredo_mp4` は常時有効であり feature gate 不要のため、常時有効な他バリアント（`Io`, `Webrtc`, `Websocket` 等）と同じタプル形式とする。

```rust
// src/error.rs に追加
Mp4(Mp4Error),
```

バリアントを利用するため、`src/error.rs` の先頭に以下を追加する:

```rust
use crate::video_codecs::mp4::Mp4Error;
```

### 2. `From<Mp4Error> for Error` の実装

`src/error.rs` に以下を追加する:

```rust
impl From<Mp4Error> for Error {
    fn from(err: Mp4Error) -> Self {
        Error::Mp4(err)
    }
}
```

### 3. `Error::Mp4` の `Display` を日本語で実装

`src/error.rs:173-352` の `Display::fmt()` の match 式に、`Error::Mp4` アームを追加する:

```rust
Error::Mp4(err) => write!(f, "MP4 ファイルの処理に失敗しました: {err}"),
```

これに伴い、`Mp4Error` 自体の `Display`（`mp4.rs:45-57`）も日本語化する。

ただし `Error::Mp4` の Display が `"MP4 ファイルの処理に失敗しました: {err}"` と前置きするため、
`Mp4Error` 側のメッセージに「MP4 ファイルの」「MP4 の」を含めると「MP4 ファイルの処理に失敗しました: MP4 ファイルの読み込みに失敗しました: ...」と冗長になる。
このため `Mp4Error::Display` の各メッセージから重複するプレフィックスを削除する:

```rust
// Before (mp4.rs:45-57)
impl std::fmt::Display for Mp4Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read MP4 file: {err}"),
            Self::Demux(err) => write!(f, "failed to demux MP4 file: {err}"),
            Self::NoVideoTrack => f.write_str("no video track found in MP4"),
            Self::NoVideoSamples => f.write_str("no video samples found in MP4"),
            Self::UnsupportedVideoCodec => f.write_str("unsupported MP4 video codec: expected H.264, H.265, VP8, VP9, or AV1"),
        }
    }
}

// After
impl std::fmt::Display for Mp4Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "読み込みに失敗しました: {err}"),
            Self::Demux(err) => write!(f, "デマルチプレクスに失敗しました: {err}"),
            Self::NoVideoTrack => f.write_str("映像トラックがありません"),
            Self::NoVideoSamples => f.write_str("映像サンプルがありません"),
            Self::UnsupportedVideoCodec => f.write_str("映像コーデックが未対応です (H.264, H.265, VP8, VP9, AV1 のみ対応)"),
        }
    }
}

合成結果の例: `Io` → `"MP4 ファイルの処理に失敗しました: 読み込みに失敗しました: No such file or directory (os error 2)"`
```

### 4. `std::error::Error::source()` の実装

`src/error.rs:355-388` の `source()` の match 式に、`Error::Mp4` アームを追加する:

```rust
Error::Mp4(err) => Some(err),
```

（`Mp4Error` は既に `std::error::Error` を実装しているため、エラーチェーンが維持される）

### 5. `Mp4Error` を `pub(crate)` 化

`src/video_codecs/mp4.rs:37`:

```rust
// Before
pub enum Mp4Error { ... }

// After
pub(crate) enum Mp4Error { ... }
```

削除はしない。理由:
- `Error::Mp4(Mp4Error)` の内包型として必要
- `mp4.rs` 内部の `From<io::Error>` や `From<DemuxError>` をそのまま流用できる
- `mp4.rs` 内の `type Result<T>` の変更が最小限で済む

### 6. `mp4.rs` 内の `type Result<T>` の扱い

`mp4.rs:81` の `type Result<T> = std::result::Result<T, Mp4Error>` は現状維持とする。
`Mp4Error` が `pub(crate)` であればクレート内で引き続き利用可能なため、mp4.rs 内部のエラーフローに変更は不要。

### 7. `lib.rs` の `pub use` からの `Mp4Error` 削除

`src/lib.rs:50-53`:

```rust
// Before
pub use crate::video_codecs::mp4::{
    Mp4EncodedSample, Mp4Error, Mp4PassthroughVideoCodecCapability, Mp4SampleReader,
    Mp4VideoCapturer,
};

// After
pub use crate::video_codecs::mp4::{
    Mp4EncodedSample, Mp4PassthroughVideoCodecCapability, Mp4SampleReader,
    Mp4VideoCapturer,
};
```

### 8. `examples/sumomo` の修正

`Mp4Error` が `pub(crate)` 化されるため、`examples/sumomo` から `Mp4Error` への直接参照を削除する。

#### `examples/sumomo/src/error.rs`:

```rust
// Before (行 1)
use sora_sdk::Mp4Error;

// After
// この use 文を削除
```

```rust
// Before (行 14)
Mp4(Mp4Error),

// After
// このバリアントを削除（Mp4Error は pub(crate) になったため外からアクセス不可）
```

```rust
// Before (行 30)
AppError::Mp4(err) => write!(f, "AppError::Mp4: {err}"),

// After
// このアームを削除
```

```rust
// Before (行 41)
AppError::Mp4(err) => Some(err),

// After
// このアームを削除
```

```rust
// Before (行 72-76)
impl From<Mp4Error> for AppError {
    fn from(err: Mp4Error) -> Self {
        AppError::Mp4(err)
    }
}

// After
// この impl ブロックを削除
```

#### `examples/sumomo/src/main.rs`:

`Mp4SampleReader::new()` や `Mp4VideoCapturer::new()` の呼び出しで、`?` 演算子の代わりに `map_err` で `Mp4Error` を `sora_sdk::Error` に変換する。
`From<Error> for AppError` は既に存在する (`sumomo/src/error.rs:54-57`) ため、`Error` に変換すれば `?` が使える。

```rust
// Before (行 240)
let reader = Mp4SampleReader::new(mp4_path)?;

// After
let reader = Mp4SampleReader::new(mp4_path).map_err(sora_sdk::Error::from)?;
```

```rust
// Before (行 359)
let mp4_capturer = Mp4VideoCapturer::new(reader)?;

// After
let mp4_capturer = Mp4VideoCapturer::new(reader).map_err(sora_sdk::Error::from)?;
```

### 9. ドキュメントの修正

`skills/sora-rust-sdk/SKILL.md:254` の `Mp4Error` 行を `Error::Mp4` に置き換える:

```markdown
// Before
| `Mp4Error` | 常時 | MP4 関連のエラー |

// After
| `Error::Mp4` | 常時 | MP4 関連のエラー |
```

### 10. 変更履歴

`CHANGES.md` の `## develop` セクションに以下のエントリを追加する:

```markdown
- [CHANGE] `Mp4Error` を `Error` 列挙型に統合し、`pub(crate)` 化する
  - `Error::Mp4(Mp4Error)` バリアントを追加し、`From<Mp4Error> for Error` を実装する
  - `Mp4Error` は `pub(crate)` になり、外部から直接参照できなくなる
  - `Mp4Error::Display` を日本語化する
  - @melpon
```

## 完了条件

- `src/error.rs` に `Error::Mp4(Mp4Error)` バリアントが追加されている
- `src/error.rs` に `use crate::video_codecs::mp4::Mp4Error;` が追加されている
- `src/error.rs` に `impl From<Mp4Error> for Error` が実装されている
- `src/error.rs:Display::fmt()` に `Error::Mp4(err) => write!(f, "MP4 ファイルの処理に失敗しました: {err}")` が追加されている
- `src/error.rs:source()` に `Error::Mp4(err) => Some(err)` が追加されている
- `src/video_codecs/mp4.rs:37` の `Mp4Error` が `pub(crate)` になっている
- `src/video_codecs/mp4.rs:45-57` の `Mp4Error::Display` が日本語化されている
- `src/lib.rs:50-53` の `pub use` から `Mp4Error` が削除されている
- `examples/sumomo/src/error.rs` の `Mp4Error` 関連コード（use 文、バリアント、Display アーム、source() アーム、From 実装）が削除されている
- `examples/sumomo/src/main.rs:240,359` の `?` が `.map_err(sora_sdk::Error::from)?` に変更されている
- `skills/sora-rust-sdk/SKILL.md` の `Mp4Error` 行が `Error::Mp4` に更新されている
- `CHANGES.md` の `## develop` に `[CHANGE]` エントリが追加されている
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- `cargo test` が全テスト通過する

## 解決方法

1. `src/video_codecs/mp4.rs:45-57` の `Mp4Error::Display` を日本語化する
2. `src/video_codecs/mp4.rs:37` の `pub enum Mp4Error` を `pub(crate) enum Mp4Error` に変更する
3. `src/error.rs` の先頭に `use crate::video_codecs::mp4::Mp4Error;` を追加する
4. `src/error.rs:10-171` の `Error` 列挙型に `Mp4(Mp4Error)` バリアントを追加する
5. `src/error.rs` の `Display::fmt()` に `Error::Mp4(err) => write!(f, "MP4 ファイルの処理に失敗しました: {err}")` を追加する
6. `src/error.rs` の `source()` に `Error::Mp4(err) => Some(err)` を追加する
7. `src/error.rs` に `impl From<Mp4Error> for Error` を追加する
8. `src/lib.rs:50-53` の `pub use` から `Mp4Error` を削除する
9. `examples/sumomo/src/error.rs` の `Mp4Error` 関連コードを削除する
10. `examples/sumomo/src/main.rs:240,359` の `?` を `.map_err(sora_sdk::Error::from)?` に変更する（全ファイルを一括編集し、Step 2 の `pub(crate)` 化と Step 9-10 の sumomo 修正は同一コミットで行う）
11. `skills/sora-rust-sdk/SKILL.md:254` の `Mp4Error` を `Error::Mp4` に変更する
12. `CHANGES.md` の `## develop` に設計方針 10 のエントリを追加する
13. `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo test` がすべて通ることを確認する

### 修正ファイル

- `src/error.rs`
- `src/video_codecs/mp4.rs`
- `src/lib.rs`
- `examples/sumomo/src/error.rs`
- `examples/sumomo/src/main.rs`
- `skills/sora-rust-sdk/SKILL.md`
- `CHANGES.md`
