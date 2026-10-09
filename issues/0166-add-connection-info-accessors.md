# 接続情報の型付きアクセサを追加する

- Created: 2026-10-04
- Completed: {YYYY-MM-DD}
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
