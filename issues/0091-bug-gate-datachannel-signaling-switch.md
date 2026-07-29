# switched 受信前の DataChannel シグナリング切替を防ぐ

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-datachannel-signaling-switch
- Polished: {YYYY-MM-DD}

## 目的

正式な `switched` メッセージを受信する前に、送信シグナリング経路が DataChannel へ切り替わる競合を防ぐ。

## 優先度根拠

High。正常な DataChannel シグナリング構成でも、Open event と `switched` の到着順によって誤った経路へメッセージを送信し得る。

## 現状

`SoraConnection::handle_datachannel_state` は、opened DataChannel 数が設定数と一致しただけで DataChannel シグナリングを有効にする。
`switched_received` は別に管理されており、切替判定へ渡されない。

## 設計方針

- `switched_received` と必須内部 DataChannel の Open を両方満たした場合だけ切り替える
- DataChannel の個数ではなく、必要な label 集合の一致を検査する
- WebSocket error を吸収する条件にも、DataChannel シグナリングの利用可能性を含める

## 完了条件

- `switched` 前の送信が WebSocket を使用する
- `switched` と必須 DataChannel Open の両方が成立した後だけ切り替わる
- event の到着順を入れ替えたテストがある
- ユーザー定義 DataChannel の有無で判定が変わらない
