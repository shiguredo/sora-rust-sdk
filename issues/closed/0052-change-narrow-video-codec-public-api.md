# `VideoCodecPreference` / `CodecDirection` の公開 API を狭める

- Priority: High
- Created: 2026-07-23
- Completed: 2026-07-23
- Model: Composer
- Branch: feature/change-narrow-video-codec-public-api
- Polished: 2026-07-23

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./closed/0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S6 のうち、公開 API 縮小側。
非破壊掃除は [`0054-refactor-pre-release-nonbreaking-cleanup.md`](./0054-refactor-pre-release-nonbreaking-cleanup.md) で扱う。

親 0020 は Should 一括を「正式リリース後でも可」としているが、公開 API の可視性低下については本 issue を正とし、`2026.1.0` タグ前 (canary 中) に実施する。closed 親への追記は行わない。

## 目的

クレート外から不要な公開メソッドを隠し、正式版以降の SemVer 負債を防ぐ。

## 優先度根拠

High。

- 公開 API の縮小は破壊的変更であり、正式版タグ後は次メジャーまで不可 (先例 `#0038` と同型)
- 余分な `pub` は SemVer 負債になる

## 現状

調査日: 2026-07-23。

`VideoCodecPreference` (`src/video_codec_preference.rs`、`src/lib.rs` で再エクスポート):

| API | 現状 | crate 外 | crate 内 | 方針 |
|---|---|---|---|---|
| `find` | `pub` | 利用あり (`examples/sumomo/src/video_codec_list.rs:410-415`、同 `tests.rs:574`) | `video_codec.rs` / `connection_context.rs` | **`pub` 維持** |
| `codecs` | `pub` | 利用あり (`examples/sumomo/src/tests.rs:384`) | `video_codec.rs:113`、同ファイルの `validate_video_codec_preference` | **`pub` 維持** |
| `merge` | `pub` | 利用あり (`examples/sumomo/src/main.rs:99`、`video_codec_list.rs:403`) | `connection_context.rs` | **`pub` 維持** |
| `find_mut` | `pub` | なし | 同一モジュールの `merge` のみ | **非 `pub`** |
| `get_or_add` | `pub` | なし | 同ファイル単体テストのみ | **非 `pub`** |
| `has_implementation` | `pub` | なし | 同ファイル単体テストのみ | **非 `pub`** |

`PreferenceCodec` (同ファイル、`src/lib.rs` で再エクスポート):

| API | 現状 | crate 外 | crate 内 | 方針 |
|---|---|---|---|---|
| `new` / `direction` / `codec_type` / `implementation` | `pub` | 利用あり (構築・参照) | validate / `video_codec.rs` / sumomo | **`pub` 維持** |
| `set_implementation` | `pub` (`:57`) | なし | 同ファイル単体テストのみ (`:773`) | **非 `pub`** (`merge` はフィールド直代入) |

`CodecDirection` (`src/video_codec_capability.rs`):

| API | 現状 | crate 外 | crate 内 | 方針 |
|---|---|---|---|---|
| `as_label` | `pub` (`:68`) | なし | `video_codec_preference.rs:233,267` (エラー文言。単体テストが `"duplicate H264 encoder"` 等に依存) | **`pub(crate)`**。削除しない |
| `as_str` | `pub` (`:60`) | なし | `video_codec_preference.rs:166` (DisplayJson) | **`pub(crate)`** |

`new` / `new_from_capability` / `Default` は crate 外利用ありのため `pub` 維持 (表では省略)。
`src/lib.rs` の `pub use` は型のみ。本 issue で `lib.rs` は触らない。同ファイル `#[cfg(test)]` からの非 `pub` 呼び出しはそのまま通る。

### 対象外

| 項目 | 理由 |
|---|---|
| 非破壊掃除 (デッドコード・装飾コメント・`client` リネーム等) | `#0054` 側 |
| `examples/sumomo` の `find` / `codecs` / `merge` 呼び出しの書き換え | これらは `pub` 維持のため不要 |

## 設計方針

- 同一モジュール専用なら非 `pub`、跨モジュールなら `pub(crate)`
- エラー文言・単体テスト期待値は変更しない
- `CHANGES.md` に `[CHANGE]` を追記する (`shiguredo-changelog` に従う)
- バグ修正や機能追加は混ぜない

## 完了条件

- 上表どおり可視性が更新されている
- crate 外 (`examples/` / `e2e-tests/`) から狭めた API を参照していない
- `skills/sora-rust-sdk/SKILL.md` の公開表から狭めた API (`find_mut` / `get_or_add` / `has_implementation` / `set_implementation` / `as_label` / `as_str`) が削除され、`find` / `codecs` / `merge` / `PreferenceCodec` の公開ゲッターが残っている
- `CHANGES.md` の `## develop` に次を追記済み (既存 `[UPDATE]` より前。`@` は実装者名に置換):

```text
- [CHANGE] `VideoCodecPreference` の `find_mut` / `get_or_add` / `has_implementation`、`PreferenceCodec` の `set_implementation`、`CodecDirection` の `as_label` / `as_str` をクレート外から呼べなくする
  - @implementer
```

- 検証:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo test -p sora_sdk`
  - `cargo test -p sumomo`
  - `rg 'pub fn (find_mut|get_or_add|has_implementation|set_implementation)' src/` がヒットしない
  - `rg 'pub\(crate\) fn (as_label|as_str)' src/video_codec_capability.rs` がヒットする
  - `rg 'pub fn (find|codecs|merge)\b' src/video_codec_preference.rs` がヒットする

## 解決方法

対応する価値が無いため closed にする。PR `#42` も取り下げた。

再評価の結果:

1. `#0038` (`SoraConnectionCommand`) や `#0033` (`TlsConfig` フィールド) と異なり、ユーザーが触るべきでない内部型の露出・構築経路の二重化・バグ是正ではない
2. crate 外 (examples / e2e) からの参照は狭め候補メソッドについて 0 件だが、それは「今すぐ `pub` を外す必然」にはならない。preference 構築ヘルパーとして `pub` のまま残す判断も妥当
3. 親 `#0020` S6 は Should の予防的掃除であり、Must の SemVer ブロッカーではない
4. 実施しても実害の除去にならず、破壊的変更 (`[CHANGE]`) だけが増える

結果として可視性変更は行わず、コードは現状維持とする。非破壊掃除は `#0054` で継続する。
