# リリース前の非破壊掃除を一括で行う（公開 API の破壊的変更を一部含む）

- Priority: Medium
- Created: 2026-08-16
- Completed: 2026-08-17
- Model: deepseek-v4-flash
- Branch: feature/refactor-pre-release-cleanup
- Polished: {YYYY-MM-DD}

旧 `#0049`（video codec ヘルパーの重複解消）・旧 `#0054`（リリース前の非破壊掃除 1 回目）・旧 `#0130`（同 2 回目）を統合した issue。

いずれも「重複の解消とリリース前の掃除」を目的とする同一カテゴリの issue であり、生成元（親 `#0020` の S2 / S6 と、コードベース全体レビュー）が別々だったために 3 本に分裂していた。本 issue で一括して対応し、旧 3 issue は closed にした。

リリース前の掃除として、公開 API の破壊的変更（`Error::PeerConnectionMissing` の削除、`TlsConfig` の非公開化）と、実行時動作を変える設計改善（`TimerManager::set_timer` の同一 id 上書き修正、VP9 の profile-id 統一）も対象に含める。

## 目的

コードベース全体のデッドコード・重複定義・過剰なログのレベル調整・可読性の問題を一括して解消し、保守コストを下げる。リリース前の掃除のため、後方互換のない変更（`Error::PeerConnectionMissing` の削除、`TlsConfig` の非公開化）と実行時動作を変える設計改善（`TimerManager::set_timer` の同一 id 上書き修正、VP9 の profile-id 統一）も対象に含める。

## 現状

対象は以下の 6 グループ。

### 1. video codec ヘルパーの重複

同名・同役割の関数が複数ファイルに存在し、挙動差分と修正漏れの温床になっている。

| ヘルパー | 存在箇所 | 備考 |
|---|---|---|
| `requested_frame_type` | `v4l2.rs` / `vpl.rs` / `amf.rs` / `nvcodec.rs` / `openh264.rs` | 全ファイルで実装が完全に同一 |
| `supported_formats_for_codec` | `vpl.rs` / `amf.rs` / `nvcodec.rs` | 各バックエンドでサポートするコーデックが異なる。H264 / H265 / AV1 の SdpVideoFormat は同一だが、VP9 は vpl のみ `profile-id=0` 付き、nvcodec は profile-id なし、amf は未サポート。VP8 は nvcodec のみサポート |
| `target_kbps_from_bps` | `vpl.rs` / `amf.rs` | 計算式は同一だが戻り型が `u16`（vpl）と `u32`（amf）で不一致。呼び出し元の API 要求型に依存 |

`encoder_codec_config` / `decoder_codec` / `frame_type_from_*` は各クレート固有の型（`shiguredo_vpl::CodecConfig` / `shiguredo_amf::CodecConfig` / `shiguredo_nvcodec::CodecConfig` 等）に依存し、トレイト・マクロの新規作成は shiguredo-rust スキルで禁止されているため共通化しない。

### 2. デッドコード・到達不能コード

- `e2e-tests/src/lib.rs` の `wait_task_finished`: 定義のみで呼び出し 0
- `src/error.rs` の `Error::PeerConnectionMissing` バリアント: 構築箇所が存在しない（Display の実装のみ。`source()` には明示的な arm がなく `_ => None` に落ちる）

### 3. 命名のずれ

- `src/connection.rs` の `let client = Self { ... }` と `Ok((client, handle))`: リネーム取りこぼし。両方を `connection` にリネーム

### 4. 過剰なログ

ホットパスで毎メッセージ・毎フレーム出力される `rtc_log_info!` は、通常運用では不要なノイズになるため、削除せず `rtc_log_verbose!` にレベルを落とす。

- `src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder::encode` 内のフレーム単位 `rtc_log_info!`（ホットパスで毎フレーム出力）→ `rtc_log_verbose!` に変更
- `src/connection.rs` の DataChannel メッセージ送受信の 1 メッセージごとの `rtc_log_info!`（`send_data_channel_message` の送信 1 箇所 + `handle_data_channel_message` の受信 1 箇所）→ `rtc_log_verbose!` に変更

### 5. 重複ロジック・重複定義

- テスト用ヘルパー型の 3 ファイル重複（`NoopVideoEncoder` / `NoopVideoDecoder` / `TestVideoCodecCapability` が `src/video_codec.rs` / `src/video_codec_capability.rs` / `src/video_codec_preference.rs` の各テストモジュールに定義）
- `find_capability` 関数の 2 ファイル重複（`src/video_codec.rs` / `src/video_codec_preference.rs`）
- `parse_ice_servers` / `parse_ice_servers_optional` の統合（`src/signaling_types.rs`）: 「オプショナルなメンバーを取得して配列をパースする」処理を共通ヘルパー `parse_optional` に抽出し、`parse_ice_servers` に統合する。Sora ドキュメント「シグナリングの型定義」では offer の `config`（`SignalingOfferMessage`）はオプショナルのため、`config` / `iceServers` が無い場合は空リストとして受理する。re-offer（`SignalingReOfferMessage`）は `config` を持たないため、`ReOffer` から `ice_servers` を削除してパースしない。旧 offer 側の必須パースは仕様に対して過剰に厳しかった。挙動はテストで固定する
- `examples/sumomo/src/args.rs` の Args 構造体リテラルの 3 重複

### 6. 軽微な可読性・設計

- `examples/sumomo/src/fake.rs` の `tick_once` の `_fps` 死に引数
- `src/connection.rs` の `TlsConfig`: 公開 API だが全フィールド `pub(crate)` で、外部から生成・参照する手段のないデッドな公開型（内部では builder の `insecure()` / `ca_cert()` / `client_cert()` と `build_tls_client_config` で利用）
- `src/connection.rs` の `TimerManager::set_timer`: 同一 id の実行中タイマーを abort せず上書きする
- `src/connection.rs` の `handle_offer`: 同じ形の Observer 構造体 3 連発と 5 秒ハードコード 3 箇所
- `src/error.rs` の Display 実装の言語混在（`Error::LibcameraMessage` 等の feature ゲート系のみ英語）と `Error::InvalidRole` の「--role」CLI 固有表記
- `src/lib.rs` の crate ドキュメントのサンプルが `ignore` でコンパイル確認されていない

### 対象外

| 項目 | 理由 |
|---|---|
| e2e-tests の未使用 API の削除（`SoraTestConnection::send_rpc_request` 等） | テスト追加とセットで扱うべきため対象外（テスト戦略の issue で扱う） |
| `DataChannelConfig::direction` | ワイヤ必須を `.required()` パース済み。`#[expect(dead_code)]` で維持 |
| ホットパスの `.expect("encoder should exist")` 等 | 別判断 |
| `amf.rs` の SAFETY コメント追加 | 別判断 |
| V4L2 stride 不整合 | 旧 `#0047`（open）で別途対応 |
| MP4 停止遅延 | 旧 `#0048` で対応済み |
| バグ修正・機能追加 | 混ぜない。ただし `TimerManager::set_timer` の abort 追加は、実行時動作の正確性を高める設計改善として本 issue に含める |
| URL シャッフルのモジュロバイアス | 旧 `#0032`（closed）で「対応不要」と判断済み。URL 数が高々 100 以下では偏りが無視できるため本 issue でも対応しない |

## 設計方針

- 原則 SemVer 非影響の変更に限定する
- 例外として、後方互換のない変更を 2 件だけ含める（`Error::PeerConnectionMissing` の削除、`TlsConfig` の非公開化。いずれも構築・参照手段がなく、リリース前の掃除として実施する）
- `CHANGES.md` は変更しない
- バグ修正や機能追加は混ぜない（例外: `TimerManager::set_timer` の abort 追加は、実行時動作の正確性を高める設計改善として本 issue に含める）
- 各変更はモックやスタブを使わないテストで確認する
- video codec ヘルパーの共通化方針:
  - `requested_frame_type` を `src/video_codecs/helpers.rs` へ移し、全バックエンドから利用する
  - `supported_formats_for_codec` と `target_kbps_from_bps` は共通化するか判断する
    - `supported_formats_for_codec`: 全 codec をカバーする単一の関数に統合し、呼び出し側で使わない arm はデッドコード最適化に任せる。または codec_type ごとの SdpVideoFormat 生成だけを共通関数として切り出す。VP9 の profile-id 差異は `profile-id=0` を付与する方向で統一する（SDP で明示的に profile を指定する方が安全なため。vpl 側は現状どおり、nvcodec 側に profile-id を追加する動作変更を含む）
    - `target_kbps_from_bps`: 戻り型を `u32` に統一する。AMF 側は `u32` を要求するためそのまま使える。VPL 側の受け口（`config.target_kbps` / `ReconfigureParams::target_kbps`）は `u16` のままなので、呼び出し元で `u16::try_from` して渡す。`u16` に統一すると AMF 側で `u32::from` するだけなので実害はないが、共通関数は呼び出し元の API 要求型から独立させ、変換は呼び出し元に置くのが責務の分離として適切。`u16::try_from` が失敗する（65,535 kbps 超）場合の挙動は、現行の `unwrap_or(u16::MAX)` によるクリップを維持する（サイレントに壊れるリスクは VPL の API 上限自体に由来するため）
  - 共通化しないもの（`encoder_codec_config` / `decoder_codec` / `frame_type_from_*`）は各バックエンドにそのまま残す（各クレート固有の型 `shiguredo_vpl::CodecConfig` / `shiguredo_amf::CodecConfig` / `shiguredo_nvcodec::CodecConfig` 等に依存し、トレイト・マクロの新規作成は shiguredo-rust スキルで禁止されているため）
- feature ごとのコンパイルが壊れないように共通モジュールの依存を整理する。`shiguredo_webrtc` への依存（`VideoFrameType` / `VideoFrameTypeVectorRef` 等）は `lib.rs` で re-export せず、各ファイルで直接 import する（shiguredo-rust スキルの re-export 禁止に従う）

## 完了条件

- 上記のデッドコード・到達不能コード・重複定義が削除・整理されている
- 過剰ログは削除せず `rtc_log_verbose!` にレベル変更されている
- 現状 6（軽微な可読性・設計）の各項目（`tick_once` の `_fps`、`TlsConfig`、`TimerManager::set_timer`、`handle_offer`、`error.rs` の Display、`lib.rs` のサンプル）が対応済みである
- `requested_frame_type` が単一実装になり、全バックエンドがそれを利用している
- `supported_formats_for_codec` と `target_kbps_from_bps` について共通化の要否を判断し、共通化する場合は単一実装にしている
- 共通化しないもの（`encoder_codec_config` / `decoder_codec` / `frame_type_from_*`）は各バックエンドにそのまま残している
- `TlsConfig` が公開 API から除外されている（`lib.rs` の `pub use` から削除し、`pub(crate)` にしている）
- `skills/sora-rust-sdk/SKILL.md` の公開型一覧（接続ビルダーの表）から `TlsConfig` が除去されている
- `Error::PeerConnectionMissing` が `Error` enum から削除されている
- `skills/sora-rust-sdk/SKILL.md` のエラー型テーブルから `PeerConnectionMissing` が除去されている
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
  - `cargo check --no-default-features --features v4l2`
  - `cargo check --no-default-features --features vpl`
  - `cargo check --no-default-features --features amf`
  - `cargo check --no-default-features --features nvcodec`
  - `cargo check --no-default-features --features openh264`
  - `cargo test -p sora_sdk`
  - `cargo test -p sumomo`
  - `cargo test --workspace`
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
- `src/types.rs`
- `src/signaling_types.rs`
- `src/error.rs`
- `src/lib.rs`
- `e2e-tests/src/lib.rs`
- `examples/sumomo/src/args.rs`
- `examples/sumomo/src/fake.rs`
- `skills/sora-rust-sdk/SKILL.md`（公開型一覧の `TlsConfig` 行の削除、エラー型テーブルの `PeerConnectionMissing` 除去）
- `Makefile`（`fuzzing` / `fuzzing-list` ターゲットの削除。`#0081` は pending（Low）で、`fuzz/` ディレクトリが存在せず実質的に実行不能なため）

## 解決方法

1. `requested_frame_type` を共通モジュール `src/video_codecs/helpers.rs` に移し、各ファイル（`v4l2.rs` / `vpl.rs` / `amf.rs` / `nvcodec.rs` / `openh264.rs`）の定義を削除して import に差し替える。各ファイルの `#[cfg(test)]` にあるテストも `helpers.rs` の `#[cfg(test)]` に移す
2. `supported_formats_for_codec` と `target_kbps_from_bps` の差分を評価し、共通化の要否を判断する。共通化する場合は `helpers.rs` に追加する。共通化しない場合は理由をコメントで残す
3. `encoder_codec_config` / `decoder_codec` / `frame_type_from_*` は共通化不可能であるため、各ファイルにそのまま残す
4. `src/video_codecs/mod.rs` に `pub mod helpers;` を追加する。helpers モジュールの cfg 条件は `any(feature = "v4l2", feature = "vpl", feature = "amf", feature = "nvcodec", feature = "openh264")` とする
5. `e2e-tests/src/lib.rs` から `wait_task_finished` を削除する
6. `src/connection.rs` の `client` を `connection` にリネームする
7. 過剰なログを削除せず `rtc_log_verbose!` に変更する
8. 重複ロジック・重複定義を整理する
9. 軽微な可読性・設計の問題を修正する（`TlsConfig` の非公開化を除く。`TlsConfig` は step 10 で扱う）
10. `TlsConfig` を `lib.rs` の `pub use` から外し、`pub(crate)` に変更し、`skills/sora-rust-sdk/SKILL.md` の公開型一覧から `TlsConfig` を除去する
11. `Error::PeerConnectionMissing` を `src/error.rs` の `Error` enum から削除し、`skills/sora-rust-sdk/SKILL.md` のエラー型テーブルを更新する
12. 各 feature パターンで `cargo check` と `cargo clippy` を確認する
13. 完了条件のコマンドを実行して確認する

上記の全ステップを実施し、完了条件に記載したコマンド（`cargo fmt` / `cargo clippy` / 各 feature の `cargo check` / `cargo test --workspace` 等）がすべて通過したため closed にする。
