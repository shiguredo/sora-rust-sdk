# リリース前の非破壊掃除を行う

- Priority: Medium
- Created: 2026-07-23
- Completed: {YYYY-MM-DD}
- Model: Composer
- Branch: feature/refactor-pre-release-nonbreaking-cleanup
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./closed/0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S6 のうち、非破壊掃除側。
公開 API の可視性縮小 (`#0052`) は対応不要として closed にした。本 issue は SemVer 非影響の掃除のみ。

## 目的

デッドコード・装飾コメント・命名のずれ・未使用引数属性を消し、保守コストを下げる。

## 優先度根拠

Medium。

- 動作・公開 API を変えない掃除であり、正式版タグ後でも対応可能
- 親 0020 の Should「正式リリース後でも可」を適用する

## 現状

調査日: 2026-07-23。

| 項目 | 根拠 | 方針 |
|---|---|---|
| `e2e-tests/src/lib.rs:184` の `wait_task_finished` | 定義のみ。呼び出し 0 | 削除 |
| `Makefile` の `fuzzing` / `fuzzing-list` | `fuzz/` が無く死んだターゲット | ターゲットと `.PHONY` から削除。`prek.toml` は触らない |
| `src/connection.rs:765` の `let client = Self { ... }` と `:783` の `Ok((client, handle))` | `#0016` リネーム取りこぼし | 両方を `connection` にリネーム |
| `// -------------------------` 系の装飾コメント | `src/connection.rs` / `src/types.rs` / `src/signaling_types.rs` のみ | 全削除 |
| `#[expect(unused_variables)]` | `src/video_codec_capability.rs:130,143` は未使用の `env` / `format`。`src/video_codecs/openh264.rs:135` は `settings`、`:537` は `env` / `format`。`src/video_codecs/{amf,nvcodec,v4l2,vpl}.rs` の `init_encode` は `settings`。先例 `src/video_codecs/mp4.rs` の `_settings` | 未使用引数を `_env` / `_format` / `_settings` にし属性を外す |

`amf` / `nvcodec` / `v4l2` / `vpl` / `libcamera` は optional。default は `openh264` のみ。`v4l2` 検証時は `libcamera` も同時に有効にする（`src/video_codecs/v4l2.rs` に `#[cfg(feature = "libcamera")]` あり）。

### 対象外

| 項目 | 理由 |
|---|---|
| 公開 API の可視性変更 | `#0052` で対応不要と判断済み |
| `src/zlib.rs` の統合 | 既に `pub(crate)`。`connection.rs` からのみ利用 (10 行)。独立のまま維持 |
| `examples/sumomo/src/tests.rs` の配置変更 | binary-only の private 試験。`src/` 内 `#[cfg(test)]` が正しい |
| `DataChannelConfig::direction` | ワイヤ必須を `.required()` パース済み。`#[expect(dead_code)]` で保持 |
| `#0049` / `#0053` / `#0046` | 別目的。`wait_task_finished` 以外の e2e 変更なし |

## 設計方針

- SemVer 非影響の変更に限定する
- `CHANGES.md` に書かない (`[CHANGE]` も `### misc` の復活もしない)
- バグ修正や機能追加は混ぜない

## 完了条件

- 上表の削除・リネーム・装飾コメント削除が完了している
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
  - `cargo test -p sora_sdk`
  - `cargo test -p sumomo`
  - `Makefile` に `fuzzing` / `fuzzing-list` が無い
  - `rg '// -{5,}' src/connection.rs src/types.rs src/signaling_types.rs` がヒットしない

## 解決方法

1. `e2e-tests/src/lib.rs` から `wait_task_finished` を削除する
2. `Makefile` から `fuzzing` / `fuzzing-list` と `.PHONY` の該当名を削除する
3. `src/connection.rs:765` と `:783` の `client` を `connection` にリネームする
4. 装飾コメント 3 ファイルを削除する
5. `src/video_codec_capability.rs` と `src/video_codecs/{openh264,amf,nvcodec,v4l2,vpl}.rs` の未使用引数を `_` 接頭辞化し、`#[expect(unused_variables)]` を外す
6. 完了条件のコマンドを実行する
