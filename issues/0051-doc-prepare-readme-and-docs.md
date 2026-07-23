# README / docs を正式リリース向けに整備する

- Priority: Medium
- Created: 2026-07-23
- Completed: {YYYY-MM-DD}
- Model: Composer
- Branch: feature/doc-prepare-readme-and-docs
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S5。S7 の `CODEBASE.md` 整備も本 issue に含める。

## 目的

利用者が正式版を導入するときに迷わないよう、README・付属 docs・`CODEBASE.md`・sumomo ドキュメントを現状の実装と CI に揃える。

## 優先度根拠

Medium。

- crates.io / GitHub の第一印象が README
- ビルド依存や対応プラットフォームの誤りは導入失敗に直結する
- コード変更を伴わないためリリース後でも対応可能だが、正式版と同時にあるのが望ましい

## 現状

確認済みのずれ・残作業:

- `README.md` の対応 OS 記述と `.github/workflows/ci.yml` の matrix が完全一致していない
- 「優先実装」セクション（`README.md:461` 付近）の整理が必要
- Copyright 表記の見直し
- 構成図に `pbt/` `docs/` が未反映の可能性
- Sora 対応上限の明示が不足
- Linux ビルド依存の網羅が不足しうる
- `docs/SORA_CPP_SDK.md` に古い記述が残っていないか要確認
- `examples/sumomo/README.md` に `libssl-dev` 記述が残存（`examples/sumomo/README.md:10`）
- `examples/sumomo/Cargo.toml` のパッケージメタデータが薄い（`version = "0.0.0"`、description 等なし）
- sumomo の CLI オプション一覧が不足しうる
- `CODEBASE.md` はタイトルのみ（本文未整備）

サンプル URL の `example.com` 化は README 上は既に概ね実施済み。残があれば合わせる。

## 設計方針

- 実装と CI を正とし、ドキュメントを追従させる
- `CODEBASE.md` はリポジトリ固有規約・ディレクトリ役割が分かる最小限の本文にする
- sumomo は「動かすための最小情報」を README / Cargo.toml に集約する
- 本 issue ではコードの機能変更をしない

## 完了条件

- README の対応プラットフォーム・ビルド依存・構成が現状と矛盾しない
- `docs/SORA_CPP_SDK.md` の明らかな古い記述が解消されている
- sumomo README の誤記が消え、CLI の使い方が追える
- `CODEBASE.md` に実内容がある
- ドキュメントのみの変更としてレビュー可能な粒度になっている

## 解決方法

1. README / CI / `Cargo.toml` を突き合わせて差分表を作る
2. README・docs・sumomo README / Cargo.toml・`CODEBASE.md` を更新する
3. 不要になった「優先実装」や古い記述を削るまたは現状に合わせて直す
