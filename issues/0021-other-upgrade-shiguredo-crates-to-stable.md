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

`Cargo.toml` の `[workspace.dependencies]` 内の指定（canary または完全 pin）:

- `shiguredo_amf = "2026.3.0-canary.0"`（Cargo.toml:23）
- ~~`shiguredo_libcamera = "2026.1.0-canary.1"`~~ → `shiguredo_libcamera = "2026.1"`（Cargo.toml:27。2026-06-23 に正式版へ切り替え済み）
- ~~`shiguredo_nvcodec = "=2026.2.0-canary.2"`~~ → `shiguredo_nvcodec = "=2026.2.0"`（Cargo.toml:31。2026-06-23 に正式版へ切り替え済み。完全 pin は維持）
- ~~`shiguredo_v4l2 = "2026.1.0-canary.4"`~~ → `shiguredo_v4l2 = "2026.1"`（Cargo.toml:35。2026-06-23 に正式版へ切り替え済み）
- `shiguredo_vpl = "2026.3.0-canary.0"`（Cargo.toml:37）
- ~~`shiguredo_webrtc = "=0.150.1"`~~ → `shiguredo_webrtc = "0.150.2"`（Cargo.toml:39。2026-06-23 に正式版へ切り替え済み）

サブクレートでも個別に依存している時雨堂クレートがあれば追従が必要:

- `e2e-tests/Cargo.toml`
- `examples/sumomo/Cargo.toml`
- `pbt/Cargo.toml`

## 設計方針

1. 依存先 6 クレートの正式版リリース完了を待つ（本 issue の前提条件）
2. 正式版リリース後、`Cargo.toml` の指定をマイナーバージョン指定（例: `"2026.3"`）に切り替える
3. 完全 pin（`=` 指定）はすべて外し、マイナー指定に揃える
4. プロジェクト方針「依存ライブラリのバージョン指定はマイナーまで」に合わせる
5. サブクレート（`e2e-tests` / `examples/sumomo` / `pbt`）に直接記載されている時雨堂クレートも同時に更新する
6. 全 feature 組合せ（default / `amf` / `libcamera` / `nvcodec` / `openh264` / `v4l2` / `vpl`）で `cargo build --workspace` と `cargo test --workspace` が通ることを確認する
7. `CHANGES.md` の `## develop` に最終バージョンを反映した UPDATE エントリを記載する（既存の中間バージョンエントリは整理して 1 件に集約する）

## 完了条件

- `Cargo.toml` の時雨堂クレート 6 件がすべて正式版（canary サフィックスなし）を指している
- 完全 pin（`=` 指定）が外れており、マイナーバージョン指定になっている
- サブクレートの時雨堂クレート参照も整合している
- 全 feature の組合せで `cargo build --workspace` と `cargo test --workspace` が通る
- `CHANGES.md` の `## develop` に各 UPDATE エントリが追加・整理されている
- `Cargo.lock` が更新されている

## 解決方法

1. 依存先 6 クレートの正式版リリース状況を `cargo search` などで確認する
2. すべての正式版がリリースされていれば、`Cargo.toml` の `[workspace.dependencies]` をマイナーバージョン指定で書き換える
3. サブクレート（`e2e-tests` / `examples/sumomo` / `pbt`）の時雨堂クレート参照も合わせて更新する
4. `cargo update` を実行して `Cargo.lock` を更新する
5. CI 同等の環境で `cargo build --workspace` / `cargo test --workspace` を全 feature 組合せで実行する
6. `CHANGES.md` の `## develop` に UPDATE エントリを集約して記載する

## 進捗

- 2026-06-23: `shiguredo_libcamera` と `shiguredo_v4l2` を 2026.1 安定版に切り替え、`Cargo.lock` と `CHANGES.md` を更新済み
- 2026-06-23: `shiguredo_webrtc` を 0.150.2 に上げ、`Cargo.lock` と `CHANGES.md` を更新済み
- 2026-06-23: `shiguredo_nvcodec` を `=2026.2.0` 正式版に切り替え、`Cargo.lock` と `CHANGES.md` を更新済み（完全 pin は維持）

## 依存待ち

依存先 6 クレートのうち、残り 2 クレート（`shiguredo_amf` / `shiguredo_vpl`）の正式版リリース完了が前提条件。リリーススケジュールが大きく遅れる場合は本 issue を `issues/pending/` に移動する選択肢がある。
