# ログメッセージを英語に統一する

- Priority: High
- Created: 2026-06-22
- Completed: 2026-06-22
- Model: Opus 4.7
- Branch: feature/fix-translate-log-messages-to-english
- Polished: {YYYY-MM-DD}

## 目的

AGENTS.md「ログメッセージは全て英語にすること」に違反している日本語のログメッセージを全て英語化する。
プロジェクト規約違反の修正であり、`/review-code` の致命的指摘の一つ。

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Must 派生 issue M8。

## 優先度根拠

- AGENTS.md の明文化された規約違反
- 正式リリース 2026.1.0 の前に直さないと、リリース後も「割れ窓」として残り続ける
- 機械的修正であり、互換性への影響もない（ログ文字列の言語が変わるだけ）

## 現状

`rtc_log_info!` / `rtc_log_warning!` / `rtc_log_error!` 等のフォーマット文字列に日本語が混入している。
検出: `grep -rn "rtc_log_" src examples/sumomo/src | grep -E '"[^"]*[ぁ-んァ-ヶー一-龯]'`

合計 43 件:

- `src/connection.rs`: 26 件
  - 該当行: 656, 874, 911, 930, 933, 995, 1028, 1032, 1111, 1116, 1128, 1131, 1134, 1139, 1142, 1260, 1296, 1300, 1653, 1660, 1688, 1809, 1815, 1829, 2443, 2485
  - 例: `rtc_log_info!("接続が閉じられました")` / `rtc_log_warning!("JSON メッセージの解析に失敗しました")` / `rtc_log_info!("DataChannel '{}' を登録しました", label)` / `rtc_log_info!("Sora サーバーから切断されました")` / `rtc_log_info!("Ping を受信しました")` 等
- `examples/sumomo/src/main.rs`: 17 件
  - 該当行: 440, 446, 457, 461, 476, 485, 492, 547, 561, 577, 585, 588, 705, 726, 737, 746, 749
  - 例: シグナリング送受信のログ、接続状態のログ、コーデック選択のログ等

他のソースファイル（`src/libcamera.rs` / `src/video_codecs/*` 等）にもログマクロは存在するが、本 issue 起票時点でフォーマット文字列に日本語は含まれていない（grep で確認済み）。

## 対象外

- `Error` の `Display` 実装の日本語メッセージ（規約上、エラーメッセージは利用者向けで日本語）
- `expect("BUG: external_adm が None です")` などの panic メッセージ（親 issue の S7「規約遵守」で別途対応する）
- コメント・docstring の日本語（規約上日本語）
- テストのログメッセージ（規約上日本語）

## 設計方針

1. 既存の英語ログメッセージのスタイル（時制・大文字小文字・句読点）を踏襲する
2. 「何が起きたか」「どの label/url/state が対象か」を簡潔に表現する
3. ログレベル（info / warning / error）の使い分けは現状を維持し、文言だけ翻訳する
4. プレースホルダ引数（`{}` / `{:?}` / `{label}` 等）の構造は維持する
5. 過度な意訳をせず、原文の情報量を保つ
6. 文章を読みやすくするための英文補足（冠詞等）の追加は許容する

## 完了条件

- `grep -rn "rtc_log_" src examples/sumomo/src | grep -cE '"[^"]*[ぁ-んァ-ヶー一-龯]'` の結果が `0`
- `cargo build --workspace` / `cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` が通る
- `CHANGES.md` の `## develop` に FIX エントリ追加
- e2e-tests / pbt のテストログ（日本語）は対象外なので変更されていない

## 解決方法

PR #33 で対応。

- `src/connection.rs` の `rtc_log_*` マクロ 26 件を英語化
- `examples/sumomo/src/main.rs` の `rtc_log_*` マクロ 17 件を英語化
- 完了確認: `grep -rn "rtc_log_" src examples/sumomo/src | grep -cE '"[^"]*[ぁ-んァ-ヶー一-龯]'` の結果が `0`
- `cargo build --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` 全通過
- `CHANGES.md` の `## develop` に `[FIX] ログメッセージを英語に統一する` を追加
- GitHub Actions の CI（GitHub-hosted 5 ジョブ + self-hosted 5 ジョブ + slack_notify）全 pass で squash merge
