# sumomo の終了要求と失敗結果を正しく処理する

- Priority: Medium
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-sumomo-termination
- Polished: {YYYY-MM-DD}

## 目的

指定時間経過時に sumomo を確実に切断し、非同期処理や描画 thread の失敗を成功終了として扱わない。

## 優先度根拠

Medium。正式サンプルの CLI 契約と終了 status が実動作と一致せず、自動実行で hang や障害の見落としが発生する。

## 現状

通常表示経路は duration 経過時に event loop を抜けるだけで、connection handle から disconnect せず、同じ `run` future を再び待つ。
raw-player 経路は async block の `Result` と worker thread の panic を破棄して `Ok(())` を返す。

## 設計方針

- duration 経過時に connection handle から disconnect する
- `run` の終了を timeout 付きで待つ
- async block の error と thread panic を main の終了結果へ伝播する
- renderer と capture resource をすべての終了経路で停止する

## 完了条件

- `--duration` 指定時間後に process が正常終了する
- setup、connection、renderer の失敗が非 0 終了になる
- worker panic が成功終了として隠れない
- 通常表示と raw-player の両経路を実行するテストがある
