# リリース前の掃除リファクタを行う

- Priority: Medium
- Created: 2026-07-23
- Completed: {YYYY-MM-DD}
- Model: Composer
- Branch: feature/refactor-pre-release-cleanup
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S6。

## 目的

公開面に不要な API・デッドコード・装飾・配置のずれを消し、正式版以降の保守コストを下げる。

## 優先度根拠

Medium。

- 公開 API の余分な `pub` は SemVer 負債になる
- 掃除自体は互換に配慮すればリリース後でも可能だが、canary のうちに閉じたい項目が多い
- 1 ブランチでまとめてよい粒度のリファクタ群

## 現状

未対応（2026-07-23 確認）:

- `examples/sumomo/src/tests.rs` が crate 内 `src/` 配下のまま
- `VideoCodecPreference` の `find` / `find_mut` / `get_or_add` など、外部不要な可能性のある API が `pub`（`src/video_codec_preference.rs`）
- `Makefile` に `fuzzing` / `fuzzing-list` ターゲットが残存
- `e2e-tests/src/lib.rs:184` の `wait_task_finished` が残存
- `CodecDirection::as_label`（`src/video_codec_capability.rs:68`）が残存
- `src/zlib.rs` が独立モジュールのまま（`connection.rs` から利用）
- `#[expect(unused_variables)]` が複数箇所（例: `openh264.rs` / `amf.rs` / `nvcodec.rs` / `v4l2.rs` / `vpl.rs` / `video_codec_capability.rs`）
- `// ----` 系の装飾コメントが `connection.rs` / `types.rs` / `signaling_types.rs` 等に残存
- `src/connection.rs:765` が `let client = Self { ... }` のまま

既に対応済みのため本 issue の対象外:

- `CHANGES.md` の `### misc` 削除
- `src/` 内 `#[allow(dead_code)]` 解消

`DataChannelConfig::direction` は `signaling_types.rs` で `pub(crate)` のフィールドとして現存する。削除してよいかはシグナリング必須フィールドか再確認してから判断する（安易に消さない）。

## 設計方針

- 公開 API を狭める変更は `pub` → `pub(crate)` を基本とし、外部利用が無いことを確認する
- テスト配置は sumomo / e2e の既存慣例に合わせる
- `zlib.rs` 統合は可読性が落ちない範囲で `connection` 近傍へ寄せるか、現状維持の判断を明示する
- バグ修正や機能追加は混ぜない

## 完了条件

- 上記未対応リストが解消されるか、残す理由がコメント / 本 issue に書かれている
- 公開 API を狭めた項目について、crate 外からの参照が無い
- `cargo test --workspace` / `clippy` が通る

## 解決方法

1. 公開 API の利用有無を crate 外（examples / e2e / docs）から確認し、不要なら `pub(crate)` 化する
2. sumomo テスト配置、Makefile、装飾コメント、変数名、`#[expect]` を片付ける
3. `wait_task_finished` / `as_label` / `zlib` の要否を個別に判断して削除または統合する
4. `DataChannelConfig::direction` はワイヤ必須なら削除せず対象外とする
