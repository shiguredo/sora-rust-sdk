# DataChannel 経由の ping で stats 付き ping が無視されている

Created: 2026-03-18
Completed: 2026-04-07
Model: GPT-5.4

## 概要

WebSocket 経由の ping では `stats: true` の場合に統計情報付き pong を返しているが、
DataChannel 経由の ping では常に空の pong を返している。動作に差異がある。

## 該当箇所

- `src/client.rs:1704`

## 再現手順

1. DataChannel シグナリングに切り替わった状態にする
2. Sora サーバーから `stats: true` 付きの ping を DataChannel 経由で受信する
3. 統計情報なしの空の pong が返される（WebSocket 経由では統計情報付き pong が返される）

## 優先度

低

## 解決方法

`handle_datachannel_message` の `IncomingMessageData::Ping` 処理を修正し、
`stats: true` の場合は `get_stats()` の取得結果を `pong.stats` に含めるようにした。
`get_stats()` に失敗した場合は、WebSocket 経由と同じく空の `pong` にフォールバックする。
