# DataChannel 受信イベントキューを有界化する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-bound-datachannel-event-queue
- Polished: {YYYY-MM-DD}

## 目的

DataChannel 受信イベントの滞留量を制限し、受信速度が処理速度を上回った場合のメモリ枯渇を防ぐ。

## 優先度根拠

High。リモートがメッセージを連続送信すると、所有された受信データが無制限に蓄積し、プロセス停止につながる。

## 現状

`SoraConnection::new` はイベント配送に unbounded channel を使用する。
`DcObsHandler::on_message` は受信データを `Vec<u8>` に複製してからキューへ投入するため、キューの要素数と総バイト量に上限がない。

## 設計方針

- イベント配送を有界化する
- メッセージ数だけでなく、保持中の総バイト量を制限する
- 上限超過時の drop、エラー通知、DataChannel 切断のいずれを採用するか明示する
- callback スレッドを無期限に block しない

## 完了条件

- 受信処理を上回る速度で送信しても、キューのメモリ使用量が設定上限内に収まる
- 上限超過時の動作が API とログから判別できる
- 大量の実データを送受信するテストで上限動作が検証されている
- 通常の DataChannel メッセージ配送に回帰がない
