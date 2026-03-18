# DataChannel 経由の ping で stats 付き ping が無視されている

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
