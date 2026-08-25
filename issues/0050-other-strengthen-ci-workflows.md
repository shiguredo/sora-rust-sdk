# CI ワークフローを強化する

- Priority: Medium
- Created: 2026-07-23
- Completed: {YYYY-MM-DD}
- Model: Composer
- Branch: feature/fix-strengthen-ci-workflows
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S4。S7 のうち MSRV / toolchain 検証も本 issue に含める。

## 目的

対応プラットフォームと MSRV を CI で継続検証し、ドキュメント・`Cargo.toml` の宣言と実装のズレを早く検知する。

## 優先度根拠

Medium。

- README は Ubuntu / macOS / Windows の対応を謳うが、GitHub-hosted の macOS 行列が無い
- `Cargo.toml` の `rust-version = "1.88"` に対し、`rust-toolchain.toml` は `stable` のまま
- 正式リリース後の回帰防止に効くが、リリースそのもののブロッカーではない

## 現状

`.github/workflows/ci.yml`:

- GitHub-hosted は Ubuntu 系 + `windows-2025-vs2026`
- macOS は self-hosted のみ
- `cargo clippy --workspace` はあるが `--all-targets` ではない
- `cargo doc -D warnings` が無い
- MSRV 1.88 固定検証ジョブが無い
- `cp .cargo/config.toml.ci .cargo/config.toml` の意図がワークフロー内で説明されていない

`rust-toolchain.toml` は `channel = "stable"`。

## 設計方針

- `macos-15` / `macos-26`（利用可能なら）を GitHub-hosted 行列へ追加する。ランナー提供状況に合わせて現実的な集合にする
- MSRV は `rust-version` に合わせて検証ジョブを追加するか、`rust-toolchain.toml` を `1.88.0` に固定する（どちらか一方ではなく、方針を issue 内で確定して実装する）
- `clippy --all-targets` と `cargo doc -- -D warnings` を追加する
- self-hosted の `--skip` があれば、可能ならテスト側 `#[ignore]` へ移す
- `.cargo/config.toml.ci` の用途をコメントまたは docs で明示する

## 完了条件

- CI が README の対応 OS 方針と矛盾しない macOS 検証を含む（または意図的除外理由が文書化されている）
- MSRV 1.88 が CI または toolchain 固定で保証されている
- `clippy --all-targets` と `cargo doc -D warnings` が CI にある
- 既存の必須ジョブが壊れない

## 解決方法

1. `ci.yml` の matrix / ジョブを拡張する
2. MSRV 方針を決めて toolchain か専用ジョブを入れる
3. clippy / doc ステップを追加する
4. self-hosted skip と config.toml.ci の説明を整理する
