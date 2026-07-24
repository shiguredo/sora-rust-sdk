# テストコード内の `Mock` / `Stub` 命名を解消し、AGENTS.md 規約に沿わせる

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-rename-mock-stub-in-test-code
- Polished: {YYYY-MM-DD}
- Updated: 2026-07-24

## 目的

AGENTS.md「モックやスタブは絶対に利用しないこと」に反し、`#[cfg(test)]` 内で `MockCapability` / `MockVideoCodecCapability` / `StubVideoEncoder` / `StubVideoDecoder` / `StubVideoEncoderWithInfoName` の名前で実装差し替え型テストダブルが 3 ファイルで多用されている。命名変更と、可能なら実装アプローチの見直しを行う。

## 優先度根拠

High。プロジェクト規約 (AGENTS.md) の明示的違反。正式リリース前に是正しないと、リポジトリ全体の規約遵守レベルが下がる。

## 現状

該当箇所 (宣言 / impl / 使用位置は 2026-07-24 の実測値):

- `src/video_codec_capability.rs`: 宣言 + impl は `:178-222`、使用は `:235` — `StubVideoEncoder` / `StubVideoDecoder` / `MockCapability`
- `src/video_codec.rs`: 宣言 + impl は `:434-599` (Stub 系 `:434,437,440`、`MockCapability` の struct と impl は `:449-599`)、使用は `:602,618,640,645,668,702,713,738,759,893` — `StubVideoEncoder` / `StubVideoDecoder` / `StubVideoEncoderWithInfoName` / `MockCapability`
- `src/video_codec_preference.rs`: 宣言 + impl は `:369-470` (Stub 系 `:369,372`、`MockVideoCodecCapability` の struct と impl は `:375-469`)、使用は `:449,465,481,510,719,724` — `StubVideoEncoder` / `StubVideoDecoder` / `MockVideoCodecCapability`

いずれも `#[cfg(test)]` 内で `VideoCodecCapability` / `VideoEncoderHandler` / `VideoDecoderHandler` の trait を実装し、テスト用の最小限のダミー実装として利用している。

## 設計方針

以下のいずれかを選択:

- **案 A (推奨)**: 命名を変更する (`Mock` / `Stub` プレフィックスを外し、`TestOnlyVideoCodecCapability`, `NoopVideoEncoder`, `NoopVideoDecoder` 等にリネーム)。実装内容は「trait を最小限に実装した本物」であることを日本語コメントで明示する。
- **案 B**: 実装を差し替え、実際の `InternalVideoCodecCapability` などを使ってテストを組み直す。テスト意図が「trait のインターフェース検証」だけなら不可能ではないが、テストのカバレッジが変わる可能性がある。
- **案 C**: AGENTS.md 側で例外を明文化する (「テスト専用の trait 実装は許容する」)。ただしプロジェクト方針を緩めることになる。

**案 A を第一候補とする**。ただし命名変更だけでは規約の精神には反するため、コメントで「本物の trait 実装であり、実装を差し替えていない」ことを日本語で明確に説明する。将来的に案 B に移行できるならさらに望ましい。

## 完了条件

- `Mock` / `Stub` プレフィックスの型名が SDK の `#[cfg(test)]` から除去されている。
- 新しい命名の意図が日本語コメントで明示されている。
- テストのカバレッジと意味が変わっていない (テストが緑のまま通る)。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
