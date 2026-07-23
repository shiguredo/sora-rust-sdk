# video codec ヘルパーの重複を解消する

- Priority: Low
- Created: 2026-07-23
- Completed: {YYYY-MM-DD}
- Model: Composer
- Branch: feature/refactor-dedupe-video-codec-helpers
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 残項目。

## 目的

各ハードウェア / ソフトウェアコーデック実装にコピーされた小さなヘルパーを共通化し、挙動差分と修正漏れを減らす。

## 優先度根拠

Low。

- 動作バグそのものではないが、同種修正を複数ファイルへ漏らす温床になっている
- 正式リリース後でも段階対応可能
- 共通化の境界を誤ると feature gate が複雑になるため、慎重に進める

## 現状

同名・同役割の関数が複数ファイルに存在する:

| ヘルパー | 存在箇所 |
|---|---|
| `requested_frame_type` | `v4l2.rs` / `vpl.rs` / `amf.rs` / `nvcodec.rs` / `openh264.rs` |
| `supported_formats_for_codec` | `vpl.rs` / `amf.rs` / `nvcodec.rs` |
| `encoder_codec_config` | `vpl.rs` / `amf.rs` / `nvcodec.rs` |
| `decoder_codec` | `vpl.rs` / `amf.rs` / `nvcodec.rs` |
| `target_kbps_from_bps` | `vpl.rs` / `amf.rs`（戻り型が `u16` と `u32` で不一致） |
| `frame_type_from_*` | 各バックエンド固有（完全共通化はしない） |

対象ファイル:

- `src/video_codecs/v4l2.rs`
- `src/video_codecs/vpl.rs`
- `src/video_codecs/amf.rs`
- `src/video_codecs/nvcodec.rs`
- `src/video_codecs/openh264.rs`

本 issue の対象外:

- ホットパスの `.expect("encoder should exist")` 等（別判断）
- `amf.rs` の SAFETY コメント追加（別判断）
- V4L2 stride 不整合（#0047）
- MP4 停止遅延（#0048）

## 設計方針

- バックエンド非依存で同一のものを `video_codecs` 配下の共通モジュールへ移す
- バックエンド固有の変換（`frame_type_from_amf` 等）は残す
- `target_kbps_from_bps` の戻り型差は、呼び出し側の要求に合わせて意図をコメントで残すか型を統一する
- feature ごとのコンパイルが壊れないように共通モジュールの依存を整理する

## 完了条件

- 上記の共通可能なヘルパーが単一実装になっている
- 各コーデック実装がそれを利用している
- 既存テストが通る

## 解決方法

1. 共通化候補を diff して、完全一致 / 近似一致を分類する
2. 共通モジュール（例: `src/video_codecs/helpers.rs`）を追加する
3. 各ファイルから重複定義を削除して差し替える
4. feature 組み合わせで `cargo check` / `clippy` を確認する
