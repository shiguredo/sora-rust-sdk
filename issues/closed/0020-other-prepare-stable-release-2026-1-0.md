# 正式リリース 2026.1.0 に向けたタスク管理（親 issue）

- Priority: High
- Created: 2026-06-22
- Completed: 2026-07-23
- Model: Opus 4.7
- Branch: develop
- Polished: {YYYY-MM-DD}
- Updated: 2026-07-23

## 目的

`sora_sdk` を `2026.1.0-canary.14` から **最初の正式版 `2026.1.0`** として crates.io に公開するための準備を集約する親 issue。
個別の修正は本 issue から派生 issue を切って対応する。正式リリース後は SemVer 互換を守る必要があるため、互換破壊につながる公開 API 変更は本 issue 配下で完結させる。

本 issue 自体は実装作業を伴わないタスク管理 issue である。チェックボックス更新や派生 issue 起票のための issue ファイル単体コミットは `develop` ブランチで直接行う（`shiguredo-issues` 規約の「コード変更を伴わない操作は develop で直接作業する」に従う）。実装作業は派生 issue 側でそれぞれ作業ブランチを切る（1 issue = 1 branch）。

## 優先度根拠

- 正式リリースは時雨堂 OSS の事業上の最優先マイルストーン
- 公開後は SemVer 制約により破壊変更が打ちづらくなるため、canary のうちに公開 API を整える必要がある
- `/review-code` で 6 観点並列レビューを 1 周回した結果、致命的 11 件・重要多数を確認しており、未対応のまま正式版を出すと利用者へ強い不利益が生じる

## 現状

2026-07-23 時点:

- バージョン: `2026.1.0-canary.14`（`Cargo.toml:3`）。正式版 `2026.1.0` は未公開
- 時雨堂依存クレートは `[workspace.dependencies]` 内 10 件すべて正式版の `~X.Y` tilde requirement に統一済み（#0021 closed）。canary 依存・完全 pin なし。`shiguredo_webrtc = "~0.150"`（`Cargo.lock` は `0.150.3`）
- Must（M2 / M10 欠番を除く M1, M3〜M9, M11）はすべて完了済み（チェック済み）
- 公開 API: `SoraConnectionCommand` は `pub(crate)` 化済み（#0038）。`#[non_exhaustive]` は規約により付与しない方針のまま（M3）。rustdoc は #0041 で拡充済み
- `release.yml` は #0028 で「対応不要」と判断して closed。ワークフロー本体は `cargo publish` 中心の現状維持
- `CHANGES.md` は `213eba4` で本文を削除済み。現在は凡例 + `## develop` に UPDATE 1 件。リリース時に `## 2026.1.0` へのリネーム（または M10 方針に沿った削除）が残作業
- 日本語ログメッセージは #0022 で英語化済み（`src/` のログマクロに日本語は残っていない）。`examples/sumomo` の日本語 `expect` は S7 側の残作業
- Should: S3（#0029〜#0037）はすべて closed。S2 の派生 #0024〜#0027 は closed。S2 残は #0047〜#0049 を起票済み。S1 / S4 / S5 / S6 / S7 は #0046 / #0050〜#0053 を起票済み
- 本 issue 以外の open issue は `#0023`（AMF simulcast SIGSEGV）と Should 派生 #0046〜#0053。`#0023` は本 issue の派生一覧には含めていない（親リンクも無し）
- Should から切り出さなかった軽微項目: `examples/sumomo` の日本語 `expect` 3 箇所、`amf.rs` の SAFETY コメント、ホットパス `.expect("encoder should exist")`、`issues/pending/0003` / `0007` の旧名更新。必要なら直接修正する

`/review-code` の指摘詳細は本リポジトリのレビューレポートを参照。本 issue では派生 issue として個別に追跡する。

## 設計方針

### 派生 issue 切り出しルール

- 本 issue を親として、以下の Must（致命的）10 件 + Should（重要・改善）7 グループ群から個別 issue を起票する
- 派生 issue は `shiguredo-issues` 規約に従う（1 issue = 1 目的 1 カテゴリ、命名規則、メタデータ）
- 派生 issue 起票時は本 issue のチェックボックスに issue 番号を追記して進捗管理する
- Must 10 件（正式リリースのブロッカー）は起票・対応済み
- Should の派生 issue は起票済み（#0046〜#0053）。対応は正式リリース後でも可
- M2 は対応しない（メタデータ整備は不要との判断）。番号は欠番として残す（M3 以降の参照を維持するため）

### 派生 issue 一覧（Must）

正式リリース前に必ず終わらせる。**SemVer 互換の観点から正式版リリース後では取り戻せない項目**。

- [x] M1. 時雨堂依存クレートを正式版に切り替える（`shiguredo_amf` / `shiguredo_libcamera` / `shiguredo_nvcodec` / `shiguredo_v4l2` / `shiguredo_vpl` / `shiguredo_webrtc` の canary・完全 pin 解消）（#0021）
  - 対象を含む時雨堂依存 10 クレートを `~X.Y` tilde requirement に統一済み（#0021 closed）
- M2. （欠番。対応しないと判断）
- [x] M3. 公開 API への `#[non_exhaustive]` 一斉付与（`shiguredo-rust` 規約の `#[non_exhaustive]` 禁止により対応しない）
- [x] M4. `SoraConnectionCommand` を `pub(crate)` 化（#0038）
- [x] M5. `shiguredo_webrtc` の公開 API 型を `pub use` で再エクスポート（`shiguredo-rust` 規約の re-export 禁止により対応しない）
- [x] M6. 公開 API の rustdoc 拡充（`lib.rs` クレートドキュメント・`types.rs` / `error.rs` / `video_codec*.rs` / `video_codecs/*.rs` のモジュールドキュメントと項目ドキュメント）（#0041）
- [x] M7. `Mp4Error` を `Error` 系へ統合（`From<Mp4Error> for Error` または `Error::Mp4` バリアント追加 + `Mp4Error::Display` 日本語化）（#0039）
- [x] M8. 日本語ログメッセージの英語化（`src/connection.rs` 26 件、`examples/sumomo/src/main.rs` 17 件）（#0022）
- [x] M9. `release.yml` の整備（ビルド依存 `apt-get install` ステップ、`timeout-minutes`、`concurrency`、prerelease 判定の SemVer 対応、`cargo publish --dry-run` 別ジョブ、`-p sora_sdk` 明示、OpenH264 ダウンロード）（#0028）
  - #0028 で各項目を対応不要と判断して closed。ワークフロー本体は意図的に現状維持
- M10. （欠番。CHANGES.md はリリース時に全削除するため対応不要）
- [x] M11. `SKILL.md` のバージョン追従ルールと iOS 対応の確定（`canary.X` 手動同期の自動化 or 粗い表記化 / `InternalAppleVideoCodecCapability` の iOS gate 維持か削除か）（#0042）

### 派生 issue 一覧（Should）

技術的負債だが正式リリース後でも段階対応可能。Must 完了後に順次起票する。

- [ ] S1. テスト戦略強化（#0046）
- [ ] S2. video codec 層の残課題
  - 完了・対応不要で closed: AMF ホットパス `assert_eq!`（#0024） / `libcamera.rs` の `acquire()` 後リソースリーク（#0025） / `v4l2.rs` の callback と encoder 同居デッドロック懸念（#0026） / `mp4.rs` の `lengthSizeMinusOne` 無視（#0027）
  - 残の派生: V4L2 stride バッファ不整合（#0047） / MP4 停止遅延（#0048） / codec ヘルパー重複（#0049）
  - 切り出し対象外（軽微・直接対応可）: ホットパス `.expect("encoder should exist")` / `amf.rs` の SAFETY コメント不在
- [x] S3. 公開 API 設計の追加修正（`SoraConnection` / `TimerManager` の `Drop` 実装（#0029） / `SecureRandom` 毎フレーム初期化と panic 経路（#0030） / `now()` の panic 経路（#0031） / URL シャッフルの modulo bias（#0032） / `TlsConfig` の二重インターフェース（#0033） / `Result<T>` エイリアスの扱い（#0034） / `ParsedProxyInfo` のフィールド可視性整合（#0035） / `tokio` の `rt-multi-thread` 削除（#0036） / 公開構造体の `Debug` 手書き実装（#0037））
  - 派生 #0029〜#0037 はすべて closed
- [ ] S4. CI ワークフロー強化（#0050。MSRV / `rust-toolchain.toml` も含む）
- [ ] S5. README / docs の整備（#0051。`CODEBASE.md` も含む）
- [ ] S6. リリース前掃除（#0052）
  - 完了済み: `CHANGES.md` の `### misc` セクション削除 / `src/` 内 `#[allow(dead_code)]` は 0 件（当該 TODO は解消）
- [ ] S7. 規約遵守（`src/` の `.unwrap()` → `.expect` は #0053。MSRV は #0050、`CODEBASE.md` は #0051）
  - 完了済み: `.cargo/config.toml.example` の UTF-8 BOM 削除
  - 切り出し対象外（軽微・直接対応可）: sumomo 日本語 `expect` 3 箇所 / `issues/pending/0003` / `0007` の旧名更新

## 完了条件

- 上記 Must のチェックボックス（M2 と M10 を除く M1 と M3〜M9、M11）がすべて完了している（達成済み）
- `Cargo.toml` の依存が canary を含まず、`shiguredo_webrtc` の完全 pin が外れている（#0021 で達成済み。`~0.150`）
- `release.yml` が新規タグ push で正式版を crates.io に公開できる状態になっている（#0028 で現状のまま公開可能と判断済み）
- `CHANGES.md` のリリース見出し整備（`## develop` を `## 2026.1.0` にリネームするか、M10 方針に沿ってリリース時に本文を削除する）
- 派生 issue がすべて closed または明示的に「正式リリース 2026.1.0 後に対応する」として pending に分けられている
- `sora_sdk 2026.1.0` 正式版が crates.io に公開され、docs.rs にビルドされている

S1〜S7 の Should グループは正式版リリース後でも段階対応可能なため、本 issue の完了条件から外す（個別 issue で追跡する）。

## 解決方法

1. 本 issue 作成と同時に、Must の M1 と M3〜M11（M2 と M10 を除く）を派生 issue として順次起票する
2. 各派生 issue は `shiguredo-issues` / `shiguredo-git` / `shiguredo-changelog` 規約に従い、1 issue 1 目的 1 ブランチで対応する
3. 派生 issue を起票するたび、本 issue 内のチェックボックスに issue 番号を追記する（例: `- [ ] M1. ... (#XXXX)`）
4. 派生 issue が closed になるたび、本 issue 内のチェックボックスにチェックを入れて closed コミットに本 issue 番号を含める
5. Must は完了済み。Should 派生は #0046〜#0053 を起票済み。正式リリース後でも段階対応可能
6. 完了条件をすべて満たした時点で本 issue を closed にする。リリース作業はタグ push と `release.yml`（#0028 で現状維持と判断）で行う。Should が残っていても、明示的に「正式リリース後対応」として open / pending のまま親を closed にしてよい

Must（M1, M3〜M9, M11）はすべて完了し、Should（S1〜S7）は #0046〜#0053 に切り出し済みである。正式版 `2026.1.0` のタグ push / crates.io 公開 / `CHANGES.md` 見出し整備はメンテナーが別途行うため、本 issue での追跡は不要と判断した。タスク管理の親としての役目は終わったので、本 issue を closed にする。

## 派生 issue の起票方針

- カテゴリは各派生 issue の本質に合わせる
  - M1, M9: `other`（リリース準備・CI・依存管理）
  - M3, M4, M5, M7: `change`（公開 API の互換破壊変更）
  - M6: `doc`
  - M8: `fix`（規約違反の修正なので `fix` がより適切。`change` でも可）
   - M10: （欠番）
   - M11: `doc`
  - S1: `test`
  - S2: `bug`（致命的バグ修正）
  - S3: `change` か `refactor`
  - S4: `other`
  - S5: `doc`
  - S6: `refactor`
  - S7: `refactor`（規約遵守のための一律置換）
- 必要に応じて派生 issue の中で複数ファイルにまたがる対応を許容する
- 派生 issue 内に「親 issue: #0020」と明記する
