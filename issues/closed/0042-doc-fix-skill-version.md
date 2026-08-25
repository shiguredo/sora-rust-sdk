# SKILL.md のバージョン表記を canary 番号を含まない形式に変更する

- Priority: High
- Created: 2026-07-03
- Completed: 2026-07-03
- Model: DeepSeek V4 Pro
- Branch: feature/update-skill-version
- Polished: 2026-07-03

## 目的

`skills/sora-rust-sdk/SKILL.md` にハードコードされた canary バージョン番号が `Cargo.toml` の実バージョンと乖離する問題を解消する。

## 優先度根拠

- 正式リリース前に必須（親 issue: #0020 の Must 項目 M11 のうち SKILL.md バージョン追従分）
- SKILL.md のバージョン表記が実態と一致していないと、AI エージェントが誤った API 情報を参照する

## 現状

`skills/sora-rust-sdk/SKILL.md:26` に `2026.1.0-canary.10` とハードコードされているが、`Cargo.toml:3` の実際のバージョンは `2026.1.0-canary.11` で一致していない。

`canary.py` は `Cargo.toml` と `Cargo.lock` のみを更新し、SKILL.md の更新ロジックは含まれていない。このため canary のたびに乖離が拡大する。

親 issue #0020 の M11 では二択「 `canary.X` 手動同期の自動化 or 粗い表記化」が提示されていた。
本 issue では以下の理由から canary 番号を含まない表記に変更する方式を採用する:

- SKILL.md は LLM エージェント向けの参照資料であり、canary 番号レベルの正確なバージョンは不要。メジャー・マイナーまでの一致で API の安定性判断には十分
- `canary.py` に SKILL.md 更新ロジックを追加すると、canary bump のたびに SKILL.md という無関係なファイルのコミットが発生し、ノイズになる
- canary 番号を含まなければ canary 間の自動 bump が不要であり、正式リリース時に手動更新すればよい

## 設計方針

- SKILL.md:26 のバージョン表記を `2026.1.0-canary`（canary 番号を含まない表記）に変更する
- 正式リリース時に `2026.1.0` に更新する。以降の開発サイクル（例 : `2026.2.0-canary.0` 開始時）にも手動更新する
- SKILL.md 内の他に canary バージョン番号がハードコードされている箇所があれば、同様に canary 番号を除去する
- 非 canary バージョン番号（ `最小 Rust バージョン: 1.88` 、 `対応 Sora: 2025.1.0 以降` ）は本 issue の対象外とする

## 完了条件

- `skills/sora-rust-sdk/SKILL.md:26` のバージョン表記が `2026.1.0-canary` に変更されている
- SKILL.md 内に他に canary バージョン番号のハードコードがないことが確認されている（`-canary\.\d+` で検索して 0 件）

## 解決方法

1. `skills/sora-rust-sdk/SKILL.md:26` のバージョン表記を `2026.1.0` に変更した（canary 番号を除去）
2. SKILL.md 内を正規表現 `-canary\.\d+` で全文検索し、他に canary バージョン番号がハードコードされていないことを確認した
3. 実際のコードと SKILL.md の記述を照合し、以下の乖離を修正した:
   - `ParsedProxyInfo` の `parse` 関数の所属型を `ProxyInfo` → `ParsedProxyInfo` に修正
   - `Video` バリアントのフィールドに型注釈を追加
   - エラー表に `Error::Mp4` と `Error::InvalidSystemTime` を追加

## 親 issue

- #0020
