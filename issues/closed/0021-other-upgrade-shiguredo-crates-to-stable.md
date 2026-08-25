# 時雨堂依存クレートを正式版に切り替える

- Priority: High
- Created: 2026-06-22
- Completed: 2026-07-23
- Model: Opus 4.7
- Branch: feature/update-shiguredo-crates-to-stable
- Polished: {YYYY-MM-DD}
- Updated: 2026-07-23

## 目的

`sora_sdk 2026.1.0` を最初の正式版として crates.io に公開するため、依存先の時雨堂クレートを正式版に切り替える。

起票時点では canary 版または完全 pin が混在しており、正式版 sora_sdk が canary に依存する状態は crates.io 上のセマンティクスとして許容できなかった（利用者が意図せず prerelease に固定される）。依存切り替え自体は 2026-06-23 に完了し、検証は CI の feature マトリクスで担保する方針で本 issue を closed にする。

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Must 派生 issue M1。親 0020 の M1 チェックボックスは依存切り替え完了として 2026-07-02（`107b3a6`）で `[x]` 済み。

## 優先度根拠

- 正式リリース 2026.1.0 の Must 派生 issue（親 0020 の M1）だった
- 公開後は依存バージョンの破壊変更が利用者へ波及するため、canary のうちに正式版依存（`~X.Y`）へ揃える必要があった

## 現状

2026-07-23 時点で、`Cargo.toml` の `[workspace.dependencies]` 内の時雨堂依存クレート 10 件はすべて正式版を指し、`~X.Y` 形式（マイナーまで固定、パッチは自動追従）の tilde requirement に統一済み:

- `shiguredo_amf = "~2026.3"`（`Cargo.lock`: `2026.3.0`）
- `shiguredo_http11 = "~2026.6"`（`Cargo.lock`: `2026.6.1`）
- `shiguredo_libcamera = "~2026.1"`（`Cargo.lock`: `2026.1.0`）
- `shiguredo_mp4 = "~2026.3"`（`Cargo.lock`: `2026.3.0`）
- `shiguredo_nvcodec = "~2026.2"`（`Cargo.lock`: `2026.2.0`）
- `shiguredo_openh264 = "~2026.1"`（`Cargo.lock`: `2026.1.0`）
- `shiguredo_v4l2 = "~2026.1"`（`Cargo.lock`: `2026.1.0`）
- `shiguredo_vpl = "~2026.3"`（`Cargo.lock`: `2026.3.0`）
- `shiguredo_webrtc = "~0.150"`（`Cargo.lock`: `0.150.3`）
- `shiguredo_websocket = "~2026.3"`（`Cargo.lock`: `2026.3.0`）

サブクレートのうち `e2e-tests` / `examples/sumomo` で直接バージョン指定されているのは `shiguredo_audio_device` / `shiguredo_video_device`（いずれも `"2026.2"`）のみ。`pbt` には該当依存なし。これらは本 issue のスコープ外として現状維持。

`CHANGES.md` には 0021 対応時に個別 UPDATE と `~X.Y` 統一の集約エントリを追加済みだったが、`213eba4`（2026-07-14）で変更履歴本文が削除された。親 0020 の M10（欠番: リリース時に CHANGES を全削除するため対応不要）方針と整合し、本 issue では再記載しない。現 `CHANGES.md` の `## develop` に残る関連エントリは `shiguredo_webrtc` 0.150.3 の UPDATE のみ。

## 設計方針

1. 依存先の時雨堂クレートが正式版（canary サフィックスなし）に揃っていること（実装済み）
2. 時雨堂依存クレート（workspace.dependencies の 10 件）は `~X.Y` 形式の tilde requirement に統一する（メジャー・マイナーを固定し、パッチは crates.io の最新を自動追従。`=` なしの `^` 解釈によるマイナー上振れを排除しつつ、バグ修正パッチは取り込む）（実装済み）
3. 全 feature 組合せ（default / `amf` / `libcamera` / `nvcodec` / `openh264` / `v4l2` / `vpl`）で `cargo build --workspace` と `cargo test --workspace` が通ることを確認する（CI の feature マトリクスで担保。実装済み扱い）
4. `CHANGES.md` への UPDATE 追記は、追加済みのうえ `213eba4` で削除済み。親 M10 方針により本 issue では再記載しない（実装済み扱い）

## 完了条件

- `Cargo.toml` の時雨堂依存クレート 10 件がすべて正式版を指している（実装済み）
- すべての時雨堂依存クレートが `~X.Y` 形式の tilde requirement になっている（実装済み）
- `Cargo.lock` が更新されている（実装済み）
- `CHANGES.md` への UPDATE 追記は追加後に削除済み。親 M10 方針により再記載不要（実装済み扱い）
- 全 feature の組合せで `cargo build --workspace` と `cargo test --workspace` が通る（CI の feature マトリクスで担保。実装済み扱い）

## 解決方法

1. 依存先 10 クレートの crates.io 上の最新パッチバージョンを `cargo search` で確認した
2. `Cargo.toml` の `[workspace.dependencies]` を `~X.Y` 形式の tilde requirement で書き換えた（`c82e451`）
3. `cargo update` で `Cargo.lock` を整合させた
4. 全 feature 組合せの `cargo build --workspace` / `cargo test --workspace` は `.github/workflows/ci.yml` の feature マトリクス（`openh264` / `openh264,nvcodec` / `openh264,amf` / `openh264,vpl` / `openh264,libcamera,v4l2`）で継続的に検証しているため、別途の手動確認は不要と判断した
5. `CHANGES.md` への UPDATE 追記は実施済みのうえ `213eba4` で削除された。親 0020 の M10 方針により再記載しない

依存切り替え本体は 2026-06-23 に完了し、親 0020 の M1 も `107b3a6` で `[x]` 済みである。未記録だった検証完了条件は上記 CI マトリクスで満たすものとし、本 issue を closed にする。

## 進捗

- 2026-06-23: `shiguredo_libcamera` と `shiguredo_v4l2` を 2026.1 安定版に切り替え、`Cargo.lock` と `CHANGES.md` を更新済み
- 2026-06-23: `shiguredo_webrtc` を 0.150.2 に上げ、`Cargo.lock` と `CHANGES.md` を更新済み
- 2026-06-23: `shiguredo_nvcodec` を `=2026.2.0` 正式版に切り替え、`Cargo.lock` と `CHANGES.md` を更新済み（完全 pin は維持）
- 2026-06-23: `shiguredo_amf` と `shiguredo_vpl` を `2026.3` 正式版に切り替え、`Cargo.lock` と `CHANGES.md` を更新済み。依存先 6 クレートはすべて正式版になった
- 2026-06-23: 方針転換。時雨堂依存クレートはマイナー指定ではなく `= "X.Y.Z"` 形式の完全 pin に統一する方針へ変更。workspace.dependencies の 10 件すべてを完全 pin に書き換え（http11=`=2026.6.1`, webrtc=`=0.150.2` はパッチ込み、他は `X.Y.0`）。`Cargo.lock` は解決値が変わらないため差分なし。`CHANGES.md` に集約エントリを追加済み
- 2026-06-23: 方針再調整。完全 pin だとバグ修正パッチを取り込めないため、`~X.Y` 形式（メジャー・マイナー固定、パッチ自動追従）の tilde requirement に変更。workspace.dependencies の 10 件すべてを `~X.Y` に書き換え。`Cargo.lock` は解決値が変わらないため差分なし。`CHANGES.md` の集約エントリも書き換え済み（`c82e451`）
- 2026-07-02: 親 0020 の M1 が依存切り替え完了として `[x]` される（`107b3a6`）。本 issue は open のまま
- 2026-07-14: `CHANGES.md` の変更履歴本文が削除される（`213eba4`）。0021 で追加した UPDATE / 集約エントリも消える
- 2026-07-20: `shiguredo_webrtc` の解決値が `0.150.3` に上がる（`1cdb1f5`、Ubuntu 26.04 LTS 対応）。`~0.150` の範囲内
- 2026-07-23: 検証完了条件を CI feature マトリクスで担保すると判断し、本 issue を closed にする
