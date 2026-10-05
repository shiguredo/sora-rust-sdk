# run() の戻り値で切断理由を返す

- Created: 2026-10-05
- Completed: {YYYY-MM-DD}
- Branch: feature/change-return-disconnect-reason
- Polished: {YYYY-MM-DD}

## 目的

切断理由を型付きで取得できるようにする。切断は必ず `run` の終了として観測されるため、切断用のコールバックを追加せず戻り値で返す。

## 現状

`SoraConnection::run` は `Result<()>` を返す。`Ok(())` か `Err(Error)` かしか分からず、次を区別できない。

- クライアントからの切断
- サーバーの `close` メッセージによる終了
- シグナリングエラー (Sora はシグナリング失敗時に WebSocket を 4490 で閉じる)
- WebSocket の切断
- DataChannel が閉じられたことによる終了
- PeerConnection の失敗

サーバーの `close` メッセージの code と reason は `IncomingMessageData::Close` としてパースしているが、`run` のループでは破棄している。

`on_websocket_close` は WebSocket の Close フレームを受信したときだけ呼ばれ、Close フレームを伴わない TCP の切断では呼ばれない。

## 設計方針

`DisconnectReason` を新設し、`run` の戻り値を `Result<DisconnectReason, Error>` にする。

- 終了理由を 1 つの変数で持ち、`run` の終了時に 1 回だけ返す。`run` のループ本体を `Result` を返す内部関数に切り出し、外側で理由を確定する
- 理由は少なくとも次を区別する
  - クライアントからの切断
  - サーバーの `close` (code と reason を含む)
  - シグナリングエラー (4490 と reason)
  - WebSocket の切断 (code と reason)
  - DataChannel の close (ラベル)
  - PeerConnection の失敗
- 異常終了は従来どおり `Err(Error)` で返す。理由の分類のために `Error` を包み直さない
- 切断用のコールバックは追加しない。`on_websocket_close` は WebSocket レベルの通知として残し、接続終了の正本は `run` の戻り値であることを doc に書く
- `run` の future を破棄または abort した場合は理由を取得できないことを doc に書く
- 理由の種類は、DataChannel の close と PeerConnection の失敗の対応が入ってから確定する

## 完了条件

- 正常切断 / サーバーの `close` / シグナリングエラー (4490) / WebSocket の切断 / DataChannel の close / クライアントからの切断 / PeerConnection の失敗が、それぞれ対応する理由で返る
- サーバーの `close` の code と reason が取得できる
- `connection.run().await?;` がそのままコンパイルできる
- `run` の終了条件は変わらない

## 変更対象

- `src/connection.rs` の `run` と `DisconnectReason`
- `src/lib.rs` の例
- `README.md` と `skills/sora-rust-sdk/SKILL.md`
