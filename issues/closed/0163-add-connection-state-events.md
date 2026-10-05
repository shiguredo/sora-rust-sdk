# 接続状態の変化イベントを追加する

- Created: 2026-10-04
- Completed: 2026-10-05
- Branch: feature/add-connection-state-events
- Polished: {YYYY-MM-DD}

## 目的

接続の確立・切断を型付きイベントで通知し、利用者が notify の JSON 文字列をパースしなくても接続状態を扱えるようにする。各 SDK が notify の文字列判定を重複実装している状態を解消する。

## 現状

`SoraConnectionEventHandler` には `on_switched` と `on_websocket_close` はあるが、接続確立・切断の状態変化を表すイベントが無い。

ブリッジ側では `on_notify` に届く `connection.created` を文字列比較すれば接続確立を判定できる。パース自体は容易で、通知は接続ライフサイクル時にしか届かないため性能上の問題もない。暫定対応としてブリッジ側での文字列判定を許容する。

問題は、コアが接続状態を内部で保持しているのに利用者に再導出させていることにある。この方法は Sora の通知仕様 (`event_type` の値) に依存するため、通知の追加・変更で壊れやすい。切断理由も `on_websocket_close` のコードと理由からしか取得できない。iOS / Android の両ブリッジで同じ判定を重複して実装する必要があり、実際に両ブリッジで文字列比較を行っている。本 issue はこの重複と仕様依存を解消するための恒久対応である。

## 設計方針

`SoraConnectionEventHandler` に接続状態の変化を受け取るメソッドを追加する。

- 状態は列挙型 (connecting / connected / disconnected など) で表現する
- 切断理由も取得できるようにする
- 既存の notify 通知と `on_switched` / `on_websocket_close` はそのまま残す
- 公開 API に shiguredo_webrtc の型は出さない

## 完了条件

- 接続確立と切断が型付きイベントで通知される
- notify の文字列パースなしで状態を扱える
- 切断理由が取得できる

## 変更対象

- `src/connection_event_handler.rs` の `SoraConnectionEventHandler`
- `src/connection.rs` の `SoraConnection` (状態遷移の通知)
- `skills/sora-rust-sdk/SKILL.md` (イベント一覧の更新)

## 解決方法

この issue は扱わないことにした。接続の状態変化を 1 つのイベント API にまとめていたが、観測 (状態変化の通知) と判断 (接続の終了条件と切断理由) を分ける必要があったため、目的ごとに分割して別 issue に移した。

- 接続の確立を表すイベントは追加しない。接続の確立は WebRTC の状態変化から分かる
- 切断は必ず `run` の終了として観測されるため、切断用のコールバックは追加せず、`run` の戻り値で理由を返す
- WebRTC の状態変化の通知は別 issue で扱う
- 接続を終了する条件と切断理由は別 issue で扱う

notify の `event_type` の文字列比較をコアに移す案は採用しない。`event_type` の値は Sora の仕様で固定されているため比較自体が壊れやすいわけではなく、コアに移しても新しい情報は増えない。
