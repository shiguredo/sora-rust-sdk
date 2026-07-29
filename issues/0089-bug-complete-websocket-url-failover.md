# WebSocket 接続完了まで URL failover を継続する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-websocket-url-failover
- Polished: {YYYY-MM-DD}

## 目的

複数のシグナリング URL を指定した場合に、WebSocket Upgrade まで成功した URL を選択し、設定した接続 timeout 内で正常な代替 URL へ failover できるようにする。

## 優先度根拠

High。TCP または TLS だけ成功する URL が先着すると、正常な代替 URL を破棄した後で Upgrade に失敗し、接続不能になる。

## 現状

`connect_signaling_urls` は TCP、proxy、TLS が完了した時点で他の接続試行を中止する。
WebSocket Upgrade は URL 選択後に `SoraConnection::run` で実行される。
DNS、proxy CONNECT、WebSocket Upgrade は共通 deadline の対象になっていない。

## 設計方針

- HTTP 101 応答までを URL ごとの接続成功条件に含める
- DNS、proxy CONNECT、TCP、TLS、Upgrade を単一 deadline で管理する
- 失敗した URL を除外し、deadline 内は残りの URL を継続する
- 選択 URL と接続済み stream の対応を崩さない

## 完了条件

- 最速 URL の Upgrade が失敗しても、正常な別 URL へ接続できる
- 各接続段階の無応答が設定 timeout 内で終了する
- 実際の TCP listener と WebSocket handshake を使った failover テストがある
- redirect 後の再接続にも同じ保証が適用される
