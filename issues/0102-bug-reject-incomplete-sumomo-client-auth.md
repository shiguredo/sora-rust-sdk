# sumomo の不完全な client 認証設定を拒否する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-sumomo-client-auth
- Polished: {YYYY-MM-DD}

## 目的

client certificate と private key の片側だけが指定された場合に起動を拒否し、利用者の意図に反する認証なし接続を防ぐ。

## 優先度根拠

High。認証を設定したつもりの利用者が警告なしで認証なし接続へ移行し、接続先の設定次第では意図しない接続が成立する。

## 現状

sumomo の argument parser は certificate と key を独立して受理する。
argument validation は pair 条件を検査せず、builder 設定は両方が存在する場合だけ適用される。

## 設計方針

- certificate と key の両方指定または両方未指定だけを許可する
- 片側指定時は具体的な英語 error message を返す
- SDK 本体の client authentication validation と同じ契約に揃える
- error とログに鍵本文を含めない

## 完了条件

- certificate だけ、key だけの指定が起動前に失敗する
- 両方指定と両方未指定は従来どおり動作する
- error 表示に certificate や key の内容が含まれない
- argument validation のテストがある
