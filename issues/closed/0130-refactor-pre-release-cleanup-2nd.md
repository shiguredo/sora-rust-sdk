# リリース前の非破壊掃除を追加で行う

- Priority: Low
- Created: 2026-08-10
- Completed: 2026-08-16
- Model: deepseek-v4-flash
- Branch: feature/refactor-pre-release-cleanup-2nd
- Polished: {YYYY-MM-DD}

## 目的

コードベース全体のレビューで検出されたデッドコード・重複・過剰なログ・軽微な可読性の問題を、SemVer 非影響の範囲で掃除する。

## 現状

既存の `0054` で対応予定の項目を除き、以下が残っている。

### デッドコード・到達不能コード

- `src/error.rs` の `Error::PeerConnectionMissing` バリアント: 構築箇所が存在しない (Display と source の実装のみ)
- `src/connection.rs` の `request_stats_pong`: 常に `Ok` を返すため、呼び出し側の `is_err()` フォールバック分岐 (`send_pong`) が到達不能
- `examples/sumomo/src/main.rs` の `apply_video_options` の `_` アーム: `args.rs` で codec 値がバリデート済みのため到達不能

### 過剰なログ

- `src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder::encode` 内のフレーム単位 `rtc_log_info!` (ホットパスで毎フレーム出力)
- `src/connection.rs` の DataChannel メッセージ送受信の 1 メッセージごとの `rtc_log_info!` (送信 2 箇所 + 受信 1 箇所。うち 2 箇所は重複)

### 重複ロジック・重複定義

- テスト用ヘルパー型の 3 ファイル重複 (`NoopVideoEncoder` / `NoopVideoDecoder` / `TestVideoCodecCapability` が `src/video_codec.rs` / `src/video_codec_capability.rs` / `src/video_codec_preference.rs` の各テストモジュールに定義)
- `find_capability` 関数の 2 ファイル重複 (`src/video_codec.rs` / `src/video_codec_preference.rs`)
- `parse_ice_servers` / `parse_ice_servers_optional` の統合余地 (`src/signaling_types.rs`)
- `examples/sumomo/src/args.rs` の Args 構造体リテラルの 3 重複

### 軽微な可読性・設計

- `examples/sumomo/src/fake.rs` の `tick_once` の `_fps` 死に引数
- `src/video_codec.rs` の `align_down` の行単位の自明なコメント
- `src/connection.rs` の `TlsConfig`: 公開されているが全フィールド `pub(crate)` で生成・参照手段のないデッドな公開型
- `src/connection.rs` の `TimerManager::set_timer`: 同一 id の実行中タイマーを abort せず上書きする
- `src/connection.rs` の `handle_offer`: 同じ形の Observer 構造体 3 連発と 5 秒ハードコード 3 箇所
- `src/connection.rs` の URL シャッフル: `u64 % n` のモジュロバイアス
- `src/error.rs` の Display 実装の言語混在 (`Error::LibcameraMessage` 等の feature ゲート系のみ英語) と `Error::InvalidRole` の「--role」CLI 固有表記
- `src/lib.rs` の crate ドキュメントのサンプルが `ignore` でコンパイル確認されていない

### 対象外

- 公開 API の可視性変更・後方互換のない変更 (別 issue の範囲)
- e2e-tests の未使用 API の削除 (`SoraTestConnection::send_rpc_request` 等) は、テスト追加とセットで扱うべきため本 issue の対象外 (テスト戦略の issue で扱う)
- `DataChannelConfig::direction` の削除は `0054` で「維持」判断済みのため対象外
- バグ修正・機能追加は混ぜない

## 設計方針

- SemVer 非影響の変更に限定する
- `CHANGES.md` に書かない
- バグ修正や機能追加は混ぜない
- 各変更はモックやスタブを使わないテストで確認する

## 完了条件

- 上記のデッドコード・重複・過剰ログが削除・整理されている
- `TlsConfig` の公開境界が決定され、不要なら非公開にする
- 検証:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo test --workspace`
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/error.rs`
- `src/connection.rs`
- `src/signaling_types.rs`
- `src/video_codec.rs`
- `src/video_codec_capability.rs`
- `src/video_codec_preference.rs`
- `src/video_codecs/mp4.rs`
- `src/lib.rs`
- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/args.rs`
- `examples/sumomo/src/fake.rs`

## 解決方法

本 issue は `#0146`（リリース前の非破壊掃除を一括で行う）に統合した。

旧 `#0049`・旧 `#0054`・旧 `#0130` はいずれも「重複の解消と SemVer 非影響の掃除」を目的とする同一カテゴリの issue であり、生成元（親 `#0020` の S2 / S6 と、コードベース全体レビュー）が別々だったために分裂していた。重複したカテゴリの issue が独立に残ると、実装時に対象ファイル・完了条件の検証が分断されるため、3 つを 1 つの `#0146` に統合して一括対応する。

本 issue の内容（デッドコード削除・過剰なログ削減・重複ロジック整理・軽微な可読性改善）は `#0146` の「現状 2・4・5・6」および「設計方針」「完了条件」「変更対象」「解決方法」に引き継がれている。
