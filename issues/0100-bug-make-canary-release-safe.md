# canary リリース操作を事前検証して安全にする

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-canary-release
- Polished: {YYYY-MM-DD}

## 目的

canary release script が不正な branch や dirty worktree から部分的な commit、push、tag 公開を行わないようにする。

## 優先度根拠

High。現在の順序では branch push 後に tag push が失敗するなど、公開操作が部分適用され、再実行も安全に行えない。

## 現状

canary release script は version file を先に更新し、dependency update、commit、tag、branch push、tag push を順に実行する。
branch、worktree、既定ブランチ包含、既存 tag、build、test、package を事前確認しない。

## 設計方針

- Python 3.12、3.13、3.14 を対象とし、全関数へ型を付ける
- branch、clean 状態、remote 同期、tag 重複を変更前に検査する
- fmt、test、package を commit と tag の前に実行する
- 中間失敗時の状態と復旧手順を明確にする
- dry-run が実行予定の変更と検証を正確に表示する
- script のテストにモックやスタブを使わない

## 完了条件

- 不正 branch、dirty worktree、既存 tag ではファイルを変更する前に停止する
- build または package 失敗時に commit と tag が作られない
- 同じ引数で再実行しても意図しない version 増加や push が発生しない
- Python の format、lint、型検査、テストが設定されている
