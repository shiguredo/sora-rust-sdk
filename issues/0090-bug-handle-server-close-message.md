# サーバーの Close メッセージで接続を終了する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-server-close-message
- Polished: {YYYY-MM-DD}

## 目的

サーバーから正式な Close メッセージを受信したときに、シグナリング経路にかかわらず `SoraConnection::run` を終了する。

## 優先度根拠

High。特に WebSocket 切断後に DataChannel シグナリングを継続する構成では、Close を無視すると接続 task が残留する。

## 現状

WebSocket 経路の `IncomingMessageData::Close` は内側の event poll loop だけを抜ける。
`SoraConnection::handle_datachannel_message` は DataChannel 経由の Close を unsupported message として扱う。

## 設計方針

- WebSocket と DataChannel で共通の Close 処理を使用する
- 外側の接続 loop を終了させる状態を明示する
- Close callback、DataChannel close、WebSocket close の順序を一貫させる

## 完了条件

- WebSocket 経由の Close で `run` が終了する
- DataChannel 経由の Close でも `run` が終了する
- WebSocket を無視して DataChannel シグナリングを継続する構成を実接続で検証する
- Close callback が重複して呼ばれない
