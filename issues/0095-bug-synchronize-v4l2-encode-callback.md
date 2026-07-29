# V4L2 encode callback の再登録を同期する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-v4l2-encode-callback-race
- Polished: {YYYY-MM-DD}

## 目的

V4L2 の非同期 encode callback と callback 再登録を同期し、寿命が終了した callback pointer の呼び出しを防ぐ。

## 優先度根拠

High。callback の寿命契約に反する raw pointer 呼び出しが成立し、並行再登録時に不正メモリアクセスへつながり得る。

## 現状

`handle_v4l2_encode_callback` は shared state の lock 内で callback pointer をコピーし、lock を解放してから呼び出す。
`V4L2Encoder::register_encode_complete_callback` は、進行中 callback の完了を待たずに pointer を置き換える。

## 設計方針

- callback の世代と in-flight 呼び出し数を管理する
- 旧 callback の呼び出し完了前に、その寿命を終了させない
- callback 呼び出し中の再入で deadlock しない
- release と callback 再登録の同期方針を共通化する

## 完了条件

- callback 呼び出しと再登録を並行させても旧 pointer を使用しない
- release 後に callback が呼ばれない
- callback から再入しても deadlock しない
- 実際の encoder callback を使った競合テストがある
