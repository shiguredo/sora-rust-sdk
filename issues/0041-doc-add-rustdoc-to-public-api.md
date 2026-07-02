# 公開 API の rustdoc 拡充

- Priority: High
- Created: 2026-07-03
- Completed: {YYYY-MM-DD}
- Model: DeepSeek V4 Pro
- Branch: feature/doc-add-rustdoc-to-public-api
- Polished: {YYYY-MM-DD}

## 目的

公開 API に rustdoc を追加し、`cargo doc` で完全な API リファレンスを生成できるようにする。
現在 94 件の公開アイテムのうち 19 件（約 20%）しかドキュメントコメントが存在しない。

## 優先度根拠

- 正式リリース前に必須（親 issue: #0020 の Must 項目 M6）
- rustdoc がないと利用者が公開 API の使い方を理解できない
- 公開後はドキュメントの不備がそのまま利用者への不利益になる

## 現状

`cargo doc` を実行しても、ほとんどの公開アイテムにドキュメントコメントがないため、API リファレンスとして機能しない。

| ファイル | モジュール `//!` | 公開アイテム数 | ドキュメント済み |
|---|---|---|---|
| `src/lib.rs` | 不十分（1 行のみ） | - (re-export のみ) | 0 |
| `src/types.rs` | あり | 26 | 0 |
| `src/error.rs` | あり | 2 | 1 (Result のみ、Error enum 未記載) |
| `src/video_codec.rs` | なし | 10 | 1 |
| `src/video_codec_capability.rs` | なし | 11 | 7 (trait は充実、構造体は未記載) |
| `src/video_codec_preference.rs` | なし | 16 | 0 |
| `src/video_codecs/internal.rs` | なし | 2 | 0 |
| `src/video_codecs/internal_apple.rs` | なし | 2 | 0 |
| `src/video_codecs/amf.rs` | なし | 2 | 0 |
| `src/video_codecs/nvcodec.rs` | なし | 3 | 0 |
| `src/video_codecs/openh264.rs` | なし | 2 | 0 |
| `src/video_codecs/v4l2.rs` | なし | 2 | 0 |
| `src/video_codecs/vpl.rs` | なし | 2 | 0 |
| `src/video_codecs/mp4.rs` | あり（充実） | 16 | 10 |

## 設計方針

- 全公開アイテムに `///` ドキュメントコメントを追加する
- 全モジュールに `//!` モジュールドキュメントを追加する
- `src/lib.rs` にクレート全体の説明を `//!` で追加する
- ドキュメントは日本語で書く（AGENTS.md の規約に従う）
- `SKILL.md` の内容と重複しないよう、rustdoc には簡潔な説明を書き、詳細な使い方は `SKILL.md` を参照するよう案内する
- 型の説明・メソッドの説明・パニック条件・エラー条件を明記する
- `#[cfg]` で conditionally compiled なアイテムにはその旨を記載する

## 完了条件

- 上記の全ファイルで全ての公開アイテムに `///` が付与されている
- 全ファイルにモジュールレベルの `//!` が付与されている
- `src/lib.rs` にクレートレベルの `//!` が付与されている
- `cargo doc` が警告なく実行できる

## 解決方法

1. `src/lib.rs` にクレート全体の説明を `//!` で追加する
2. `src/types.rs` の全公開型・全公開メソッドに `///` を追加する
3. `src/error.rs` の `Error` enum と全バリアントに `///` を追加する
4. `src/video_codec.rs` の全公開型・全公開関数に `///` と `//!` を追加する
5. `src/video_codec_capability.rs` の `VideoCodecImplementation`, `CodecDirection` に `///` を追加する
6. `src/video_codec_preference.rs` の全公開型・全公開メソッド・全公開関数に `///` と `//!` を追加する
7. `src/video_codecs/` 以下の各ファイルに `//!` と `///` を追加する

## 親 issue

- #0020
