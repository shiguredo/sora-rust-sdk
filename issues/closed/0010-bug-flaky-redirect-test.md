# redirect テストがフレーキーに失敗する

## 概要

`test_redirect` がサーバーの redirect 応答に依存しており、redirect が発生しない場合にタイムアウトで失敗する。

## 再現手順

1. CI で `cargo test --workspace` を実行する
2. `test_redirect` がタイムアウトで失敗する場合がある

## 再現時のエラーメッセージ

```
redirect テストがタイムアウトしました (redirect_received=false, redirect_connect_sent=false, offer_received=true)
```

## 原因

サーバーが `cluster_affinity: true` で接続しても、クラスタの状態によっては redirect を返さず直接 offer を返すことがある。テストにリトライ機構がないため、redirect が発生しなかった場合にそのまま失敗する。

## 解決方法

2 つ目のクライアント接続を最大 3 回リトライするように修正した。redirect が発生しなかった場合は切断してから再試行する。
