# 接続情報の型付きアクセサを追加する

- Created: 2026-10-04
- Completed: 2026-10-18
- Branch: feature/add-connection-info-accessors
- Polished: {YYYY-MM-DD}

## 目的

connection_id / session_id などの接続情報を API から取得できるようにする。通知のパースを各 SDK からなくす。

## 現状

connection_id / session_id は notify (`connection.created`) の JSON にのみ含まれ、`SoraConnectionHandle` から取得できない。

ブリッジ側では notify の JSON をパースすれば回避できる。パース自体は容易で、通知は接続ライフサイクル時にしか届かないため性能上の問題もない。暫定対応としてブリッジ側でのパースを許容する。

問題は、コアが保持している接続情報を利用者に再導出させていることにある。通知仕様への依存は残り、iOS / Android の両ブリッジで同じパースを重複実装することになる。本 issue はこの重複と仕様依存を解消するための恒久対応である。

## 設計方針

`SoraConnectionHandle` に接続情報を返すアクセサを追加する。

- connection_id / session_id を返す
- 接続確立前は `None` を返す
- 公開 API に shiguredo_webrtc の型は出さない

## 完了条件

- ハンドル経由で connection_id / session_id を取得できる
- notify のパースが不要になる

## 変更対象

- `src/connection.rs` の `SoraConnectionHandle` (接続情報の保持とアクセサ)
- `skills/sora-rust-sdk/SKILL.md` (API 一覧の更新)

## 解決方法

対応しない。connection_id / session_id は利用者が notify (`connection.created`) をパースすれば取得でき、SDK にアクセサを追加する必要がないと判断した。

- 現状の記述にある「コアが保持している接続情報を利用者に再導出させている」は事実と異なる。`IncomingMessageData::Notify` は種別のみを判別してペイロードを保持せず、SDK は connection_id / session_id を一切パースしていない。SDK と利用者が同じ JSON を別々に読んでいる状態であり、再導出させているわけではない
- SDK が持つ動機が無い。`selected_signaling_url` / `connected_signaling_url` は SDK 自身が接続処理で使う値を保持しているのに対し、connection_id / session_id は接続処理に使わないため、アクセサのために保存することになる
- SDK に寄せても利用者が管理する項目は減らない。値を保持して返すだけであり、どちらに置いても同じ値を持つ
- 追加する基準が SDK の中に無い。`connection.created` には channel_id / client_id / bundle_id / metadata なども含まれ、その一部だけをアクセサにする判断は利用者側の事情に依存する
- パースの重複は解消しない。`connection.created` の他のフィールドを読む利用者は結局 notify をパースする
- ブリッジ間の実装重複はブリッジ側で共通化する話であり、SDK の公開 API を増やす根拠にはならない
