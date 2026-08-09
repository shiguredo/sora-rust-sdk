# vpl feature のターゲット OS を制限する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-vpl-feature-gating
- Polished: {YYYY-MM-DD}

## 目的

`sora_sdk` の `vpl` feature を Linux でのみビルド可能にし、macOS / Windows でのビルド失敗と README の対応表との矛盾を解消する。

## 現状

`sora_sdk` の `vpl` feature (`Cargo.toml`) にはターゲット OS のゲートがないが、`shiguredo_vpl` の `supported_codecs` は `#[cfg(target_os = "linux")]` で Linux 専用に定義されている。`src/video_codecs/vpl.rs` が `shiguredo_vpl::supported_codecs` をターゲットゲートなしで use しているため、macOS で `cargo check --features vpl` を実行すると E0432 (unresolved import) でビルドが失敗する。Windows でも同様に失敗する。

一方 README の対応表は VPL を「Windows / Linux」と記載しており、文書化されたプラットフォームで feature がビルドできない。CI は Windows ランナーでデフォルト feature のみをビルドし `--all-features` を実行しないため検出されていない。

## 設計方針

- `vpl` feature の依存を `#[cfg(target_os = "linux")]` でゲートする
- `src/video_codecs/mod.rs` と `src/lib.rs` の `vpl` モジュール公開もターゲット OS でゲートする
- README の対応表を実際のビルド可能範囲 (Linux) に合わせて修正する
- 各 HW feature のビルド可否を CI で検証する (任意。既存の self-hosted matrix で確認可能)

## 完了条件

- macOS / Windows で `cargo check --features vpl` が成功する (または明示的にビルド対象外になる)
- Linux で `vpl` feature のビルド・テストが従来どおり成功する
- README の対応表が実態と一致する
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `Cargo.toml`
- `src/video_codecs/vpl.rs`
- `src/video_codecs/mod.rs`
- `src/lib.rs`
- `README.md`
- `CHANGES.md`
