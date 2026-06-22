# 正式リリース 2026.1.0 に向けたタスク管理（親 issue）

- Priority: High
- Created: 2026-06-22
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: develop
- Polished: {YYYY-MM-DD}

## 目的

`sora_sdk` を `2026.1.0-canary.10` から **最初の正式版 `2026.1.0`** として crates.io に公開するための準備を集約する親 issue。
個別の修正は本 issue から派生 issue を切って対応する。正式リリース後は SemVer 互換を守る必要があるため、互換破壊につながる公開 API 変更は本 issue 配下で完結させる。

本 issue 自体は実装作業を伴わないタスク管理 issue である。チェックボックス更新や派生 issue 起票のための issue ファイル単体コミットは `develop` ブランチで直接行う（`shiguredo-issues` 規約の「コード変更を伴わない操作は develop で直接作業する」に従う）。実装作業は派生 issue 側でそれぞれ作業ブランチを切る（1 issue = 1 branch）。

## 優先度根拠

- 正式リリースは時雨堂 OSS の事業上の最優先マイルストーン
- 公開後は SemVer 制約により破壊変更が打ちづらくなるため、canary のうちに公開 API を整える必要がある
- `/review-code` で 6 観点並列レビューを 1 周回した結果、致命的 11 件・重要多数を確認しており、未対応のまま正式版を出すと利用者へ強い不利益が生じる

## 現状

- バージョン: `2026.1.0-canary.10`（`Cargo.toml:3`）
- 時雨堂依存クレートが canary 版または完全 pin のまま
- 公開 API に `#[non_exhaustive]` ゼロ件、rustdoc 大幅不足、`SoraConnectionCommand` 等の内部実装が公開されている
- `release.yml` にビルド依存インストールが無く、`cargo publish` の verify が失敗するリスク
- `CHANGES.md` の本文と実態が乖離、エントリ順序も規約違反
- `src/connection.rs` 等で日本語ログメッセージが 43 件超残存（AGENTS.md 違反）

`/review-code` の指摘詳細は本リポジトリのレビューレポートを参照。本 issue では派生 issue として個別に追跡する。

## 設計方針

### 派生 issue 切り出しルール

- 本 issue を親として、以下の Must（致命的）11 件 + Should（重要・改善）7 グループ群から個別 issue を起票する
- 派生 issue は `shiguredo-issues` 規約に従う（1 issue = 1 目的 1 カテゴリ、命名規則、メタデータ）
- 派生 issue 起票時は本 issue のチェックボックスに issue 番号を追記して進捗管理する
- まず Must 11 件（正式リリースのブロッカー）を起票する
- Should は Must の対応状況を見ながら順次起票する

### 派生 issue 一覧（Must）

正式リリース前に必ず終わらせる。**SemVer 互換の観点から正式版リリース後では取り戻せない項目**。

- [ ] M1. 時雨堂依存クレートを正式版に切り替える（`shiguredo_amf` / `shiguredo_libcamera` / `shiguredo_nvcodec` / `shiguredo_v4l2` / `shiguredo_vpl` / `shiguredo_webrtc` の canary・完全 pin 解消）
- [ ] M2. `Cargo.toml` のリリースメタデータ整備（`description` 改善 / `keywords` / `categories` / `documentation` / `[package.metadata.docs.rs]` / `package.include` に `THIRD_PARTY_LICENSES.md` と `CHANGES.md` 追加）
- [ ] M3. 公開 API への `#[non_exhaustive]` 一斉付与
- [ ] M4. `SoraConnectionCommand` を `pub(crate)` 化
- [ ] M5. `shiguredo_webrtc` の公開 API 型を `pub use` で再エクスポート
- [ ] M6. 公開 API の rustdoc 拡充（`lib.rs` クレートドキュメント・`types.rs` / `error.rs` / `video_codec*.rs` / `video_codecs/*.rs` のモジュールドキュメントと項目ドキュメント）
- [ ] M7. `Mp4Error` を `Error` 系へ統合（`From<Mp4Error> for Error` または `Error::Mp4` バリアント追加 + `Mp4Error::Display` 日本語化）
- [ ] M8. 日本語ログメッセージの英語化（`src/connection.rs` 26 件、`examples/sumomo/src/main.rs` 17 件）
- [ ] M9. `release.yml` の整備（ビルド依存 `apt-get install` ステップ、`timeout-minutes`、`concurrency`、prerelease 判定の SemVer 対応、`cargo publish --dry-run` 別ジョブ、`-p sora_sdk` 明示）
- [ ] M10. `CHANGES.md` の整合化とリリース手順整備（エントリ順序、`shiguredo_webrtc` / `shiguredo_nvcodec` / `prek` 等の本文の実態追従、`[ADD] ParsedProxyInfo` の本体移動、表記揺れ、`### misc` 扱いの確定、`## develop` → `## 2026.1.0` リネーム手順の明文化）
- [ ] M11. `SKILL.md` のバージョン追従ルールと iOS 対応の確定（`canary.X` 手動同期の自動化 or 粗い表記化 / `InternalAppleVideoCodecCapability` の iOS gate 維持か削除か）

### 派生 issue 一覧（Should）

技術的負債だが正式リリース後でも段階対応可能。Must 完了後に順次起票する。

- [ ] S1. テスト戦略強化（PBT 追加 / `IncomingMessage::parse` `RpcResponse::parse` 単体テスト追加 / `parse_stats_lossy` 誤合格修正 / `redirect.rs` 環境変数欠落時の return 修正 / TURN-TLS / `client_cert` / `spotlight` / `forwarding_filters` の e2e 追加）
- [ ] S2. video codec 層の致命的バグ修正（`v4l2.rs` の callback と encoder 同居デッドロック懸念 / `libcamera.rs` の `acquire()` 後リソースリーク / `v4l2.rs` の stride バッファ計算 / `mp4.rs` の `lengthSizeMinusOne` 無視 / `mp4.rs` の停止応答遅延 / `amf.rs` のホットパス `assert_eq!` / `find_capability` 重複定義）
- [ ] S3. 公開 API 設計の追加修正（`SoraConnection` / `TimerManager` の `Drop` 実装 / `Result<T>` エイリアスの扱い / `tokio` の `rt-multi-thread` 削除 / `SecureRandom` 毎フレーム初期化と panic 経路 / `now()` の panic 経路 / URL シャッフルの modulo bias / `TlsConfig` の二重インターフェース / 公開構造体の `Debug` 手書き実装 / `ParsedProxyInfo` のフィールド可視性整合）
- [ ] S4. CI ワークフロー強化（`macos-15` / `macos-26` マトリクス追加 / MSRV 1.88 検証 / `clippy --all-targets` 追加 / `cargo doc -D warnings` 追加 / nightly 確認 / self-hosted の `--skip` をテスト側 `#[ignore]` へ移動 / `cp .cargo/config.toml.ci` の用途明確化）
- [ ] S5. README / docs の整備（`README.md` のプレースホルダ修正 / Linux ビルド依存の網羅 / 構成図に `pbt/` `docs/` 追加 / 対応プラットフォームと CI matrix の整合 / Sora 対応上限明示 / 「優先実装」セクション整理 / Copyright 表記見直し / `docs/SORA_CPP_SDK.md` の `HTTP Proxy 未実装` 等の古い記述修正 / `sumomo` README の `libssl-dev` 誤記削除 / CLI オプション一覧追加 / サンプル URL を `example.com` 化 / `examples/sumomo/Cargo.toml` メタデータ追加）
- [ ] S6. リリース前掃除（`examples/sumomo/src/tests.rs` を `tests/` へ移動 / `VideoCodecPreference` の `find_mut` 等 4 公開 API を `pub(crate)` 化 / `CHANGES.md` の `### misc` セクション削除 / `Makefile` の `fuzzing` / `fuzzing-list` ターゲット削除 / `wait_task_finished` 削除 / `DataChannelConfig::direction` 削除 / `CodecDirection::as_label` 削除 / `src/zlib.rs` 統合 / `#[expect(unused_variables)]` 8 箇所 と各 `#[allow(dead_code)]` の `_` プレフィックス化 / `// ----` 装飾コメント削除 / `connection.rs:728,746` の変数名 `client` を `connection` にリネーム）
- [ ] S7. 規約遵守（`src/` 内 `.unwrap()` 47 件を `.expect("MESSAGE")` 化 / `.cargo/config.toml.example` の UTF-8 BOM 削除 / `examples/sumomo/src/main.rs:543,701` の `expect("BUG: external_adm が None です")` 英語化 / `rust-toolchain.toml` を `1.88.0` 固定 or MSRV 検証 CI ジョブ追加 / `AGENTS.md:20` が指す `CODEBASE.md` の作成 or 条件付き記述化 / `issues/pending/0003` 本文の `SoraClient` 系旧名を新名に更新 / `issues/pending/0007:23` の `client.rs:1031-1045` を `connection.rs:` の新参照に更新）

## 完了条件

- 上記 M1〜M11 のチェックボックスがすべて完了している
- `cargo publish --dry-run` が CI 上で通る
- `Cargo.toml` の依存が canary を含まず、`shiguredo_webrtc` の完全 pin が外れている
- `CHANGES.md` の `## develop` が `## 2026.1.0` にリネームされ、リリース日が記載されている
- `release.yml` が新規タグ push で正式版を crates.io に公開できる状態になっている
- 派生 issue がすべて closed または明示的に「正式リリース 2026.1.0 後に対応する」として pending に分けられている
- `sora_sdk 2026.1.0` 正式版が crates.io に公開され、docs.rs にビルドされ、全 feature 付き API がドキュメントから参照可能になっている

S1〜S7 の Should グループは正式版リリース後でも段階対応可能なため、本 issue の完了条件から外す（個別 issue で追跡する）。

## 解決方法

1. 本 issue 作成と同時に、Must の M1〜M11 を派生 issue として順次起票する
2. 各派生 issue は `shiguredo-issues` / `shiguredo-git` / `shiguredo-changelog` 規約に従い、1 issue 1 目的 1 ブランチで対応する
3. 派生 issue を起票するたび、本 issue 内のチェックボックスに issue 番号を追記する（例: `- [ ] M1. ... (#XXXX)`）
4. 派生 issue が closed になるたび、本 issue 内のチェックボックスにチェックを入れて closed コミットに本 issue 番号を含める
5. Must がすべて完了したら Should の派生 issue 起票を開始する
6. 完了条件をすべて満たした時点で本 issue を closed にする。リリース作業（`canary.py` の正式版モード対応 or 別 release スクリプトの実装、`## develop` → `## 2026.1.0` リネーム、タグ作成、`cargo publish`）も M9 / M10 の派生 issue の中で完結させる

## 派生 issue の起票方針

- カテゴリは各派生 issue の本質に合わせる
  - M1, M2, M9: `other`（リリース準備・CI・依存管理）
  - M3, M4, M5, M7: `change`（公開 API の互換破壊変更）
  - M6: `doc`
  - M8: `fix`（規約違反の修正なので `fix` がより適切。`change` でも可）
  - M10, M11: `doc` または `other`
  - S1: `test`
  - S2: `bug`（致命的バグ修正）
  - S3: `change` か `refactor`
  - S4: `other`
  - S5: `doc`
  - S6: `refactor`
  - S7: `refactor`（規約遵守のための一律置換）
- 必要に応じて派生 issue の中で複数ファイルにまたがる対応を許容する
- 派生 issue 内に「親 issue: #0020」と明記する
