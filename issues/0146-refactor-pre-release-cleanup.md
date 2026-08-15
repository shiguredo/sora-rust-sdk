# リリース前の非破壊掃除を一括で行う

- Priority: Medium
- Created: 2026-08-16
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/refactor-pre-release-cleanup
- Polished: {YYYY-MM-DD}

旧 `#0049`（video codec ヘルパーの重複解消）・旧 `#0054`（リリース前の非破壊掃除 1 回目）・旧 `#0130`（同 2 回目）を統合した issue。

いずれも「重複の解消と SemVer 非影響の掃除」を目的とする同一カテゴリの issue であり、生成元（親 `#0020` の S2 / S6 と、コードベース全体レビュー）が別々だったために 3 本に分裂していた。本 issue で一括して対応し、旧 3 issue は closed にした。

## 目的

コードベース全体のデッドコード・重複定義・過剰なログ・装飾コメント・可読性の問題を、公開 API と動作を変えない範囲で一括して解消し、保守コストを下げる。

## 現状

対象は以下の 6 グループ。

### 1. video codec ヘルパーの重複

同名・同役割の関数が複数ファイルに存在し、挙動差分と修正漏れの温床になっている。

| ヘルパー | 存在箇所 | 備考 |
|---|---|---|
| `requested_frame_type` | `v4l2.rs` / `vpl.rs` / `amf.rs` / `nvcodec.rs` / `openh264.rs` | 全ファイルで実装が完全に同一 |
| `supported_formats_for_codec` | `vpl.rs` / `amf.rs` / `nvcodec.rs` | 各バックエンドでサポートするコーデックが異なるが、同一 codec_type に対する SdpVideoFormat の内容は同一。バックエンド間で VP9 の profile-id 有無にも差異あり |
| `target_kbps_from_bps` | `vpl.rs` / `amf.rs` | 計算式は同一だが戻り型が `u16`（vpl）と `u32`（amf）で不一致。呼び出し元の API 要求型に依存 |

`encoder_codec_config` / `decoder_codec` / `frame_type_from_*` は各クレート固有の型（`shiguredo_vpl::CodecConfig` / `shiguredo_amf::CodecConfig` / `shiguredo_nvcodec::CodecConfig` 等）に依存し、トレイト・マクロの新規作成は AGENTS.md で禁止されているため共通化しない。

### 2. デッドコード・到達不能コード

- `e2e-tests/src/lib.rs` の `wait_task_finished`: 定義のみで呼び出し 0
- `src/error.rs` の `Error::PeerConnectionMissing` バリアント: 構築箇所が存在しない（Display と source の実装のみ）
- `src/connection.rs` の `request_stats_pong`: 常に `Ok` を返すため、呼び出し側の `is_err()` フォールバック分岐（`send_pong`）が到達不能
- `examples/sumomo/src/main.rs` の `apply_video_options` の `_` アーム: `args.rs` で codec 値がバリデート済みのため到達不能

### 3. 装飾コメント・命名のずれ・未使用引数

- `src/connection.rs` の `let client = Self { ... }` と `Ok((client, handle))`: リネーム取りこぼし。両方を `connection` にリネーム
- `// -------------------------` 系の装飾コメント: `src/connection.rs` / `src/types.rs` / `src/signaling_types.rs` のみ。全削除
- `#[expect(unused_variables)]`: `src/video_codec_capability.rs` は未使用の `env` / `format`。`src/video_codecs/openh264.rs` は `settings` と `env` / `format`。`src/video_codecs/{amf,nvcodec,v4l2,vpl}.rs` の `init_encode` は `settings`。未使用引数を `_env` / `_format` / `_settings` にし属性を外す

### 4. 過剰なログ

- `src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder::encode` 内のフレーム単位 `rtc_log_info!`（ホットパスで毎フレーム出力）
- `src/connection.rs` の DataChannel メッセージ送受信の 1 メッセージごとの `rtc_log_info!`（送信 2 箇所 + 受信 1 箇所。うち 2 箇所は重複）

### 5. 重複ロジック・重複定義

- テスト用ヘルパー型の 3 ファイル重複（`NoopVideoEncoder` / `NoopVideoDecoder` / `TestVideoCodecCapability` が `src/video_codec.rs` / `src/video_codec_capability.rs` / `src/video_codec_preference.rs` の各テストモジュールに定義）
- `find_capability` 関数の 2 ファイル重複（`src/video_codec.rs` / `src/video_codec_preference.rs`）
- `parse_ice_servers` / `parse_ice_servers_optional` の統合余地（`src/signaling_types.rs`）
- `examples/sumomo/src/args.rs` の Args 構造体リテラルの 3 重複

### 6. 軽微な可読性・設計

- `examples/sumomo/src/fake.rs` の `tick_once` の `_fps` 死に引数
- `src/video_codec.rs` の `align_down` の行単位の自明なコメント
- `src/connection.rs` の `TlsConfig`: 公開されているが全フィールド `pub(crate)` で生成・参照手段のないデッドな公開型
- `src/connection.rs` の `TimerManager::set_timer`: 同一 id の実行中タイマーを abort せず上書きする
- `src/connection.rs` の `handle_offer`: 同じ形の Observer 構造体 3 連発と 5 秒ハードコード 3 箇所
- `src/connection.rs` の URL シャッフル: `u64 % n` のモジュロバイアス
- `src/error.rs` の Display 実装の言語混在（`Error::LibcameraMessage` 等の feature ゲート系のみ英語）と `Error::InvalidRole` の「--role」CLI 固有表記
- `src/lib.rs` の crate ドキュメントのサンプルが `ignore` でコンパイル確認されていない

### 対象外

| 項目 | 理由 |
|---|---|
| 公開 API の可視性変更・後方互換のない変更 | 別 issue の範囲 |
| e2e-tests の未使用 API の削除（`SoraTestConnection::send_rpc_request` 等） | テスト追加とセットで扱うべきため対象外（テスト戦略の issue で扱う） |
| `DataChannelConfig::direction` | ワイヤ必須を `.required()` パース済み。`#[expect(dead_code)]` で維持 |
| `Makefile` の `fuzzing` / `fuzzing-list` | `#0081` は pending（Low）。空ターゲットは本 issue で削除してよい |
| ホットパスの `.expect("encoder should exist")` 等 | 別判断 |
| `amf.rs` の SAFETY コメント追加 | 別判断 |
| V4L2 stride 不整合 | 旧 `#0047` で対応済み |
| MP4 停止遅延 | 旧 `#0048` で対応済み |
| バグ修正・機能追加 | 混ぜない |

## 設計方針

- SemVer 非影響の変更に限定する
- `CHANGES.md` に書かない（`[CHANGE]` も `### misc` の復活もしない）
- バグ修正や機能追加は混ぜない
- 各変更はモックやスタブを使わないテストで確認する
- video codec ヘルパーの共通化方針:
  - `requested_frame_type` を `src/video_codecs/helpers.rs` へ移し、全バックエンドから利用する
  - `supported_formats_for_codec` と `target_kbps_from_bps` は共通化するか判断する
    - `supported_formats_for_codec`: 全 codec をカバーする単一の関数に統合し、呼び出し側で使わない arm はデッドコード最適化に任せる。または codec_type ごとの SdpVideoFormat 生成だけを共通関数として切り出す。VP9 の profile-id 差異は `profile-id=0` を付与する方向で統一する（SDP で明示的に profile を指定する方が安全なため）
    - `target_kbps_from_bps`: AMF 側が `u32` を要求するため `u32` に統一し、VPL 側は呼び出し元で `u16::try_from` する。逆に `u16` に統一すると VPL の API 上限を超えるビットレートでクリップされサイレントに壊れるリスクがあるため
  - 共通化しないもの（`encoder_codec_config` / `decoder_codec` / `frame_type_from_*`）は各バックエンドにそのまま残し、なぜ共通化しないかのコメントを付ける
- feature ごとのコンパイルが壊れないように共通モジュールの依存を整理する。`shiguredo_webrtc` への依存（`VideoFrameType` / `VideoFrameTypeVectorRef` 等）は `lib.rs` で re-export せず、各ファイルで直接 import する（AGENTS.md の re-export 禁止に従う）

## 完了条件

- 上記のデッドコード・到達不能コード・装飾コメント・未使用引数属性・過剰ログ・重複定義が削除・整理されている
- `requested_frame_type` が単一実装になり、全バックエンドがそれを利用している
- `supported_formats_for_codec` と `target_kbps_from_bps` について共通化の要否を判断し、共通化する場合は単一実装にしている
- 共通化しないもの（`encoder_codec_config` / `decoder_codec`）は各バックエンドにそのまま残し、なぜ共通化しないかのコメントを付けている
- `TlsConfig` の公開境界が決定され、不要なら非公開にしている
- 対象外表の項目を実施していない
- 検証:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace -- -D warnings`
  - feature 付きは `.github/workflows/ci.yml` の `ci-self-hosted` に合わせる:
    - `cargo clippy --workspace --features openh264,amf -- -D warnings`
    - `cargo clippy --workspace --features openh264,nvcodec -- -D warnings`
    - `cargo clippy --workspace --features openh264,vpl -- -D warnings`
    - `cargo clippy --workspace --features openh264,libcamera,v4l2 -- -D warnings`
    - ローカルでビルドできない組み合わせはスキップしてよいが、そのセルは `ci-self-hosted` の通過を完了条件に含める
  - `cargo check --no-default-features`
  - `cargo check`（`v4l2` / `vpl` / `amf` / `nvcodec` / `openh264` の各 feature 単独）
  - `cargo test -p sora_sdk`
  - `cargo test -p sumomo`
  - `cargo test --workspace`
  - `rg '// -{5,}' src/connection.rs src/types.rs src/signaling_types.rs` がヒットしない
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/video_codecs/helpers.rs`（新規）
- `src/video_codecs/mod.rs`
- `src/video_codecs/v4l2.rs`
- `src/video_codecs/vpl.rs`
- `src/video_codecs/amf.rs`
- `src/video_codecs/nvcodec.rs`
- `src/video_codecs/openh264.rs`
- `src/video_codecs/mp4.rs`
- `src/video_codec.rs`
- `src/video_codec_capability.rs`
- `src/video_codec_preference.rs`
- `src/connection.rs`
- `src/signaling_types.rs`
- `src/error.rs`
- `src/lib.rs`
- `e2e-tests/src/lib.rs`
- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/args.rs`
- `examples/sumomo/src/fake.rs`
- `Makefile`（`fuzzing` / `fuzzing-list` の空ターゲットを削除する場合のみ）

## 解決方法

1. `requested_frame_type` を共通モジュール `src/video_codecs/helpers.rs` に移し、各ファイル（`v4l2.rs` / `vpl.rs` / `amf.rs` / `nvcodec.rs` / `openh264.rs`）の定義を削除して import に差し替える。各ファイルの `#[cfg(test)]` にあるテストも `helpers.rs` の `#[cfg(test)]` に移す
2. `supported_formats_for_codec` と `target_kbps_from_bps` の差分を評価し、共通化の要否を判断する。共通化する場合は `helpers.rs` に追加する。共通化しない場合は理由をコメントで残す
3. `encoder_codec_config` / `decoder_codec` は共通化不可能であるため、各ファイルにそのまま残し、なぜ共通化しないかのコメントを付ける
4. `src/video_codecs/mod.rs` に `pub mod helpers;` を追加する。helpers モジュールの cfg 条件は `any(feature = "v4l2", feature = "vpl", feature = "amf", feature = "nvcodec", feature = "openh264")` とする
5. デッドコード・到達不能コードを削除する（`wait_task_finished` / `Error::PeerConnectionMissing` / `request_stats_pong` のフォールバック分岐 / `apply_video_options` の `_` アーム）
6. `src/connection.rs` の `client` を `connection` にリネームし、装飾コメントを 3 ファイルから削除する
7. 未使用引数を `_` 接頭辞化し、`#[expect(unused_variables)]` を外す
8. 過剰なログを削減する
9. 重複ロジック・重複定義を整理する
10. 軽微な可読性・設計の問題を修正する
11. 各 feature パターンで `cargo check` と `cargo clippy` を確認する
12. 完了条件のコマンドを実行する
