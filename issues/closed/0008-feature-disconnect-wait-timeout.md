# 切断待機タイムアウト

## 概要

切断処理の待機タイムアウトを設定できるようにする。

## 背景

C++ SDK では `disconnect_wait_timeout` (デフォルト 5 秒) で切断時の DataChannel クローズ待機タイムアウトを設定できる。
Rust SDK では未実装。

## 調査結果

### C++ SDK

- フィールド: `disconnect_wait_timeout` (`int`, デフォルト 5 秒)
- DataChannel の `Close()` メソッドに渡される
- すべての DataChannel がクローズされるか、タイムアウトするまで待機
- DataChannel + WebSocket 両方接続時、DataChannel のみ接続時で使用

### Rust SDK 現状

- Disconnect コマンド受信時、即座に ACK を返して break する (client.rs:774-782)
- DataChannel のクローズ待機はない
- オープン中の DataChannel に対してコールバックを呼ぶのみ

## 方針

- `SoraClientBuilder` に `disconnect_wait_timeout: Duration` を追加する (デフォルト 5 秒、C++ SDK と合わせる)
- Disconnect 処理時に type: disconnect の DataChannel メッセージを送信し、DataChannel がクローズされるまで待機する
- タイムアウト時は強制的に切断する
- `docs/SORA_CPP_SDK.md` の対応表を更新する

## 解決方法

- `SoraClientBuilder` に `disconnect_wait_timeout: Duration` を追加した (デフォルト 5 秒)
- メインループ脱出後、DataChannel が使用されている場合にクローズ完了を待機するようにした
- タイムアウト時はログを出力して強制切断する
