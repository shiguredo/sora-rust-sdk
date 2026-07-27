# テストコード内の `Mock` / `Stub` 命名を解消し、AGENTS.md 規約に沿わせる

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-rename-mock-stub-in-test-code
- Polished: 2026-07-27
- Updated: 2026-07-24

## 目的

`#[cfg(test)]` 内で `MockCapability` / `MockVideoCodecCapability` / `StubVideoEncoder` / `StubVideoDecoder` / `StubVideoEncoderWithInfoName` の命名が AGENTS.md「モックやスタブは絶対に利用しないこと」の文面と衝突している。これらの型の実体はモックフレームワークやスパイ機能を持たない、trait を本物のコードで実装したテスト専用の型であり、命名だけが問題である。命名を変更し、コメントで意図を明確化する。

## 優先度根拠

High。命名がプロジェクト規約 (AGENTS.md) の文面と衝突しており、Mock/Stub という名称が実態と乖離している。正式リリース前に是正しないと、新規参入者がコードを読み誤る可能性がある。

## 現状

該当箇所 (宣言 / impl / 使用位置は 2026-07-24 の実測値):

- `src/video_codec_capability.rs`: 宣言 + impl は `:178-222`、使用は `:235` — `StubVideoEncoder` / `StubVideoDecoder` / `MockCapability`
- `src/video_codec.rs`: 宣言 + impl は `:434-623` (Stub 系 `:434,437,440`、`MockCapability` の struct と impl は `:449-623`)、使用は `:640,645,668,702,713,738,759,893` (`:602,618` は impl ブロック内の使用のため上記範囲に含まれる) — `StubVideoEncoder` / `StubVideoDecoder` / `StubVideoEncoderWithInfoName` / `MockCapability`
- `src/video_codec_preference.rs`: 宣言 + impl は `:369-470` (Stub 系 `:369,372`、`MockVideoCodecCapability` の struct と impl は `:375-469`)、使用は `:449,465,481,510,719,724` — `StubVideoEncoder` / `StubVideoDecoder` / `MockVideoCodecCapability`

これらの型の実体はモックフレームワークやスパイ機能を一切使っておらず、`VideoCodecCapability` / `VideoEncoderHandler` / `VideoDecoderHandler` の trait を本物のコードで実装したテスト専用の型である。`StubVideoEncoder` / `StubVideoDecoder` は空の trait 実装、`StubVideoEncoderWithInfoName` は `get_encoder_info` で実装名を返す。`MockCapability` (video_codec.rs) は `resolve_sdp_format` を含む本格的なロジックを持つ。

## 設計方針

命名を変更し、コメントでテスト専用の本物の trait 実装であることを明示する。実装内容は変更しない。

### リネーム

| ファイル | 現状の型名 | リネーム先 |
|---|---|---|
| `video_codec_capability.rs` | `StubVideoEncoder` | `NoopVideoEncoder` |
| `video_codec_capability.rs` | `StubVideoDecoder` | `NoopVideoDecoder` |
| `video_codec_capability.rs` | `MockCapability` | `TestVideoCodecCapability` |
| `video_codec.rs` | `StubVideoEncoder` | `NoopVideoEncoder` |
| `video_codec.rs` | `StubVideoDecoder` | `NoopVideoDecoder` |
| `video_codec.rs` | `StubVideoEncoderWithInfoName` | `NoopVideoEncoderWithInfoName` |
| `video_codec.rs` | `MockCapability` | `TestVideoCodecCapability` |
| `video_codec_preference.rs` | `StubVideoEncoder` | `NoopVideoEncoder` |
| `video_codec_preference.rs` | `StubVideoDecoder` | `NoopVideoDecoder` |
| `video_codec_preference.rs` | `MockVideoCodecCapability` | `TestVideoCodecCapability` |

ファイルをまたいで同名の `TestVideoCodecCapability` / `NoopVideoEncoder` / `NoopVideoDecoder` が存在するが、いずれも `#[cfg(test)]` モジュール内の private な型であり競合しない。

### 関連する文字列の更新

`video_codec.rs:444` の `info.set_implementation_name("StubEncoder")` を `"NoopEncoder"` に置き換える。`video_codec.rs:909` の `implementation_name.contains("StubEncoder")` を `.contains("NoopEncoder")` に置き換える。

### コメント

各型の宣言部に、テスト専用の本物の trait 実装であることを日本語コメントで明示する。

- `NoopVideoEncoder` / `NoopVideoDecoder` / `NoopVideoEncoderWithInfoName`: 「`VideoEncoderHandler` / `VideoDecoderHandler` を最小限に実装したテスト専用の型。モックやスタブではない」
- `TestVideoCodecCapability`: 「`VideoCodecCapability` を本物のコードで実装したテスト専用の型。モックやスタブではない」

### やらないこと

- 本 issue では命名変更とコメント追加のみを行う。テストコードの実装差し替え（元・案 B）は行わない
- AGENTS.md の改訂（元・案 C）は行わない

## 完了条件

- `Mock` / `Stub` プレフィックスの型名が SDK の `#[cfg(test)]` から除去されている。
- 新しい命名の意図が日本語コメントで明示されている。
- テストのカバレッジと意味が変わっていない (テストが緑のまま通る)。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
