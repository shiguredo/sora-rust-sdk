# vpl feature のターゲット OS を制限する

- Priority: Medium
- Created: 2026-08-10
- Completed: 2026-08-13
- Model: deepseek-v4-flash
- Branch: feature/fix-vpl-feature-gating
- Polished: {YYYY-MM-DD}

## 目的

`README.md` の VPL の対応プラットフォーム表記が実態と一致していない問題を解消する。`shiguredo_vpl` の `supported_codecs` は `#[cfg(target_os = "linux")]` で Linux 専用に定義されているにもかかわらず、README は VPL を「Windows / Linux」と記載しており、Windows で利用可能であると誤認させる。

## 現状

`sora_sdk` の `vpl` feature (`Cargo.toml`) にはターゲット OS のゲートがないが、`shiguredo_vpl` の `supported_codecs` は `#[cfg(target_os = "linux")]` で Linux 専用に定義されている。`src/video_codecs/vpl.rs` が `shiguredo_vpl::supported_codecs` をターゲットゲートなしで use しているため、macOS で `cargo check --features vpl` を実行すると E0432 (unresolved import) でビルドが失敗する。Windows でも同様に失敗する。

一方 README の対応表は VPL を「Windows / Linux」と記載しており、文書化されたプラットフォームで feature がビルドできない。CI は Windows ランナーでデフォルト feature のみをビルドし `--all-features` を実行しないため検出されていない。

## 設計方針

- `README.md` の VPL の対応プラットフォームを `Linux` のみに修正する
- コード側（`Cargo.toml` の feature ゲート等）は変更しない
- 非対応 OS で `--features vpl` を有効化した場合のビルド失敗は仕様として許容する

## 完了条件

- README の対応表が実態と一致する（VPL の対応プラットフォームが `Linux` のみになる）

## 変更対象

- `README.md`

## 解決方法

1. `README.md` の VPL の対応プラットフォームを `Windows / Linux` から `Linux` に修正した
   - 機能一覧の箇条書き（`- Intel VPL によるハードウェアエンコード/デコード対応`）
   - 対応コーデック表の対応プラットフォーム列
2. コード側の OS ゲートは実施しない。理由は以下の通り
   - 他の HW feature（`amf` / `nvcodec` / `v4l2` / `libcamera`）もターゲット OS のゲートを持たず、プラットフォーム前提の運用で成立している。`vpl` だけ OS ゲートを付けるのは不整合
   - `--all-features` が全プラットフォームでビルドできる必要はない。非対応 OS で `--features vpl` を有効化した場合のビルド失敗（E0432）は仕様として許容する
   - 本 issue の実在する欠陥は README の事実誤認（Windows で利用可能と誤認させる表記）であり、これを修正すれば解決する
3. `CHANGES.md` への追記は不要。`shiguredo-changelog` 規約により `.md` ファイルの変更は変更履歴に反映しない
4. ユーザーの指示により、作業ブランチは切らず `develop` ブランチに直接コミットした
