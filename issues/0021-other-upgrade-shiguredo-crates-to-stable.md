# 時雨堂依存クレートを正式版に切り替える

- Priority: High
- Created: 2026-06-22
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/update-shiguredo-crates-to-stable
- Polished: {YYYY-MM-DD}

## 目的

`sora_sdk 2026.1.0` を最初の正式版として crates.io に公開するため、依存先の時雨堂クレートを正式版に切り替える。
現状は canary 版または完全 pin が混在しており、正式版 sora_sdk が canary に依存する状態は crates.io 上のセマンティクスとして許容できない（利用者が意図せず prerelease に固定される）。

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Must 派生 issue M1。

## 優先度根拠

- 正式リリース 2026.1.0 の物理的ブロッカー（依存先が canary のままだと正式版を公開できない）
- 公開後は依存バージョンの破壊変更が利用者へ波及するため、canary のうちに正式版依存へ揃える必要がある

## 現状

2026-06-23 時点で、`Cargo.toml` の `[workspace.dependencies]` 内の時雨堂依存クレート 10 件をすべて正式版に切り替え、`~X.Y` 形式（マイナーまで固定、パッチは自動追従）の tilde requirement に統一済み:

- `shiguredo_amf = "~2026.3"`
- `shiguredo_http11 = "~2026.6"`
- `shiguredo_libcamera = "~2026.1"`
- `shiguredo_mp4 = "~2026.3"`
- `shiguredo_nvcodec = "~2026.2"`
- `shiguredo_openh264 = "~2026.1"`
- `shiguredo_v4l2 = "~2026.1"`
- `shiguredo_vpl = "~2026.3"`
- `shiguredo_webrtc = "~0.150"`
- `shiguredo_websocket = "~2026.3"`

サブクレート（`e2e-tests` / `examples/sumomo` / `pbt`）で直接記載されているのは `shiguredo_audio_device` / `shiguredo_video_device` のみ（workspace.dependencies 経由ではない）。これらは本 issue のスコープ外として現状維持。

## 設計方針

1. 依存先の時雨堂クレートが正式版（canary サフィックスなし）に揃っていること
2. 時雨堂依存クレート（workspace.dependencies の 10 件）は `~X.Y` 形式の tilde requirement に統一する（メジャー・マイナーを固定し、パッチは crates.io の最新を自動追従。`=` なしの `^` 解釈によるマイナー上振れを排除しつつ、バグ修正パッチは取り込む）
3. 全 feature 組合せ（default / `amf` / `libcamera` / `nvcodec` / `openh264` / `v4l2` / `vpl`）で `cargo build --workspace` と `cargo test --workspace` が通ることを確認する
4. `CHANGES.md` の `## develop` に最終バージョンを反映した UPDATE エントリを記載する（中間バージョンの個別エントリは残しつつ、完全 pin 化の集約エントリを追加する）

## 完了条件

- `Cargo.toml` の時雨堂依存クレート 10 件がすべて正式版を指している
- すべての時雨堂依存クレートが `~X.Y` 形式の tilde requirement になっている
- 全 feature の組合せで `cargo build --workspace` と `cargo test --workspace` が通る
- `CHANGES.md` の `## develop` に各 UPDATE エントリと完全 pin 化エントリが追加されている
- `Cargo.lock` が更新されている

## 解決方法

1. 依存先 10 クレートの crates.io 上の最新パッチバージョンを `cargo search` で確認する
2. `Cargo.toml` の `[workspace.dependencies]` を `~X.Y` 形式の tilde requirement で書き換える
3. `cargo update` で `Cargo.lock` を整合させる（shiguredo 以外の外部クレート更新は別コミットに分離する）
4. CI 同等の環境で `cargo build --workspace` / `cargo test --workspace` を全 feature 組合せで実行する
5. `CHANGES.md` の `## develop` に各 UPDATE エントリと完全 pin 化の集約エントリを記載する

## 進捗

- 2026-06-23: `shiguredo_libcamera` と `shiguredo_v4l2` を 2026.1 安定版に切り替え、`Cargo.lock` と `CHANGES.md` を更新済み
- 2026-06-23: `shiguredo_webrtc` を 0.150.2 に上げ、`Cargo.lock` と `CHANGES.md` を更新済み
- 2026-06-23: `shiguredo_nvcodec` を `=2026.2.0` 正式版に切り替え、`Cargo.lock` と `CHANGES.md` を更新済み（完全 pin は維持）
- 2026-06-23: `shiguredo_amf` と `shiguredo_vpl` を `2026.3` 正式版に切り替え、`Cargo.lock` と `CHANGES.md` を更新済み。依存先 6 クレートはすべて正式版になった
- 2026-06-23: 方針転換。時雨堂依存クレートはマイナー指定ではなく `= "X.Y.Z"` 形式の完全 pin に統一する方針へ変更。workspace.dependencies の 10 件すべてを完全 pin に書き換え（http11=`=2026.6.1`, webrtc=`=0.150.2` はパッチ込み、他は `X.Y.0`）。`Cargo.lock` は解決値が変わらないため差分なし。`CHANGES.md` に集約エントリを追加済み
- 2026-06-23: 方針再調整。完全 pin だとバグ修正パッチを取り込めないため、`~X.Y` 形式（メジャー・マイナー固定、パッチ自動追従）の tilde requirement に変更。workspace.dependencies の 10 件すべてを `~X.Y` に書き換え。`Cargo.lock` は解決値が変わらないため差分なし。`CHANGES.md` の集約エントリも書き換え済み

## 依存待ち

依存先の時雨堂クレート 10 件はすべて正式版がリリース済み・`~X.Y` 形式の tilde requirement で統一済み。本 issue の残作業は全 feature 組合せでの `cargo build --workspace` / `cargo test --workspace` 確認のみ。
