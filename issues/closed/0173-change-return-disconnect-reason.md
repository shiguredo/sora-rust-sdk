# run() の戻り値で切断理由を返す

- Created: 2026-10-05
- Completed: 2026-10-09
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

## 解決方法

`DisconnectReason` を新設し、`SoraConnection::run` の戻り値を `Result<DisconnectReason>` に変更した。`run` のループは終了理由を `Option<DisconnectReason>` で持ち、`set_disconnect_reason` で最初に確定した理由だけを残して、ループを抜けた時点で返す。戻り値に `#[must_use]` は付けていないため `connection.run().await?;` はそのままコンパイルできる。

理由を確定する箇所は次のとおり。

- クライアントからの切断: `SoraConnectionCommand::Disconnect` の処理
- サーバーの `close`: `IncomingMessageData::Close` の code と reason
- シグナリングエラー: WebSocket の Close code 4490
- WebSocket の切断: Close フレームを受信していればその code と reason、受信していなければ code 無し
- DataChannel の close: 閉じた DataChannel のラベル
- PeerConnection の失敗: `Failed` になった場合と `Disconnected` のまま猶予期間を超えた場合

設計方針では `run` のループ本体を `Result` を返す内部関数に切り出すとしていたが、ループが共有するローカル変数が多いため、単一の理由変数と各終了経路での設定に置き換えた。`run` の終了条件は変えていない。

Close フレームを伴わない WebSocket の切断では、ソケットが死んでいるため終了処理の close handshake が I/O エラーになる。切断理由が確定済みの場合はこのエラーを警告に落とし、`WebSocketClosed` を返せるようにした。

`e2e-tests/tests/disconnect_reason.rs` を追加し、実 Sora に接続してクライアントからの切断 (WebSocket シグナリングと DataChannel シグナリング)、server Close、シグナリングエラー、Close フレームを伴わない WebSocket の切断で、返る理由を確認している。既存の E2E テストでも期待する理由を確認するようにした。テスト用の TCP プロキシは `e2e-tests/src/proxy.rs` に切り出した。

DataChannelClosed と PeerConnectionFailed は、実接続で確定的に再現する手段が無いため専用のテストを追加していない。
