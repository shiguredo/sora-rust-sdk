# video codec ヘルパーの重複を解消する

- Priority: Low
- Created: 2026-07-23
- Completed: 2026-08-16
- Model: Composer
- Branch: feature/refactor-dedupe-video-codec-helpers
- Polished: 2026-07-29

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./closed/0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 残項目。

## 目的

各ハードウェア / ソフトウェアコーデック実装にコピーされた小さなヘルパーを共通化し、挙動差分と修正漏れを減らす。

## 優先度根拠

Low。

- 動作バグそのものではないが、同種修正を複数ファイルへ漏らす温床になっている
- 正式リリース後でも段階対応可能
- 共通化の境界を誤ると feature gate が複雑になるため、慎重に進める

## 現状

同名・同役割の関数が複数ファイルに存在するが、共通化の難易度には大きな差がある:

### 完全一致（容易に共通化可能）

| ヘルパー | 存在箇所 | 備考 |
|---|---|---|
| `requested_frame_type` | `v4l2.rs` / `vpl.rs` / `amf.rs` / `nvcodec.rs` / `openh264.rs` | 全ファイルで実装が完全に同一 |

### 近似一致（共通化には工夫が必要）

| ヘルパー | 存在箇所 | 備考 |
|---|---|---|
| `supported_formats_for_codec` | `vpl.rs` / `amf.rs` / `nvcodec.rs` | 各バックエンドでサポートするコーデックが異なるが、同一 codec_type に対する SdpVideoFormat の内容は同一。バックエンド間で VP9 の profile-id 有無にも差異あり |
| `target_kbps_from_bps` | `vpl.rs` / `amf.rs` | 計算式は同一だが戻り型が `u16`（vpl）と `u32`（amf）で不一致。呼び出し元の API 要求型に依存 |

### バックエンド依存（単純な共通化は不可能）

| ヘルパー | 存在箇所 | 非共通化の理由 |
|---|---|---|
| `encoder_codec_config` | `vpl.rs` / `amf.rs` / `nvcodec.rs` | 各関数が返す型が異なるクレート（`shiguredo_vpl::CodecConfig` / `shiguredo_amf::CodecConfig` / `shiguredo_nvcodec::CodecConfig`）に依存。さらに nvcodec の H264/H265/AV1 は `idr_period: None` フィールドを持ち他と構造が異なる。トレイト・マクロの新規作成は AGENTS.md で禁止されているため、これらの関数は本 issue では共通化しない |
| `decoder_codec` | `vpl.rs` / `amf.rs` / `nvcodec.rs` | 同上。異なるクレート型（`shiguredo_vpl::DecoderCodec` 等）を返すため単純共通化不可 |
| `frame_type_from_*` | 各バックエンド固有 | バックエンドごとにフレームタイプのマッピングが異なり、共通化の対象外 |

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

- **完全一致**: `requested_frame_type` を `src/video_codecs/helpers.rs` へ移し、全バックエンドから利用する
- **近似一致**: `supported_formats_for_codec` と `target_kbps_from_bps` は共通化するか判断する
  - `supported_formats_for_codec`: 全 codec をカバーする単一の関数に統合し、呼び出し側で使わない arm はデッドコード最適化に任せる。または codec_type ごとの SdpVideoFormat 生成だけを共通関数として切り出す。VP9 の profile-id 差異は `profile-id=0` を付与する方向で統一する（SDP で明示的に profile を指定する方が安全なため）
  - `target_kbps_from_bps`: AMF 側が `u32` を要求するため `u32` に統一し、VPL 側は呼び出し元で `u16::try_from` する。逆に `u16` に統一すると VPL の API 上限を超えるビットレートでクリップされサイレントに壊れるリスクがあるため
- **バックエンド依存**: `encoder_codec_config` / `decoder_codec` / `frame_type_from_*` は本 issue では共通化しない。各クレートの型に依存するため、トレイトやマクロを作らずに共通化する手段がない
- feature ごとのコンパイルが壊れないように共通モジュールの依存を整理する。`shiguredo_webrtc` への依存（`VideoFrameType` / `VideoFrameTypeVectorRef` 等）は `lib.rs` で re-export せず、各ファイルで直接 import する（AGENTS.md の re-export 禁止に従う）

## 完了条件

- `requested_frame_type` が単一実装になり、全バックエンドがそれを利用している
- `supported_formats_for_codec` と `target_kbps_from_bps` について共通化の要否を判断し、共通化する場合は単一実装にする
- 共通化しないもの（`encoder_codec_config` / `decoder_codec`）は各バックエンドにそのまま残し、なぜ共通化しないかのコメントを付ける
- 各 feature を有効にした状態で `cargo check` が通る（確認パターンは解決方法を参照）
- `cargo clippy` が通る
- 既存テストが通る

## 解決方法

1. `requested_frame_type` を共通モジュール `src/video_codecs/helpers.rs` に移す
2. 各ファイル（`v4l2.rs` / `vpl.rs` / `amf.rs` / `nvcodec.rs` / `openh264.rs`）の `requested_frame_type` を削除し、`helpers::requested_frame_type` を import して差し替える
3. 各ファイルの `#[cfg(test)]` にある `requested_frame_type` のテストを `helpers.rs` の `#[cfg(test)]` に移す
4. `supported_formats_for_codec` と `target_kbps_from_bps` の差分を評価し、共通化の要否を判断する。共通化する場合は `helpers.rs` に追加する。共通化しない場合は理由をコメントで残す
5. `encoder_codec_config` / `decoder_codec` は共通化不可能であるため、各ファイルにそのまま残し、なぜ共通化しないかのコメントを付ける
6. `src/video_codecs/mod.rs` に `pub mod helpers;` を追加する。helpers モジュールの cfg 条件は `any(feature = "v4l2", feature = "vpl", feature = "amf", feature = "nvcodec", feature = "openh264")` とする
7. 各 feature パターンで `cargo check` と `cargo clippy` を確認する。確認パターン:
   - `--no-default-features`
   - `--features v4l2`
   - `--features vpl`
   - `--features amf`
   - `--features nvcodec`
    - `--features openh264`

## 解決方法

本 issue は `#0146`（リリース前の非破壊掃除を一括で行う）に統合した。

旧 `#0049`・旧 `#0054`・旧 `#0130` はいずれも「重複の解消と SemVer 非影響の掃除」を目的とする同一カテゴリの issue であり、生成元が別々だったために 3 本に分裂していた。重複したカテゴリの issue が独立に残ると、実装時に対象ファイル・完了条件の検証が分断されるため、3 つを 1 つの `#0146` に統合して一括対応する。

本 issue の内容（video codec ヘルパーの共通化）は `#0146` の「現状 1. video codec ヘルパーの重複」および「設計方針」「完了条件」「変更対象」「解決方法」に引き継がれている。
