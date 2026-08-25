# on_websocket_close コールバックの追加

## 概要

WebSocket 切断時にユーザーが close code と reason を取得できるコールバックを追加する。

## 背景

WebSocket が切断された際に、切断理由（close code と reason）をアプリケーション側で取得する手段がない。
デバッグや切断理由に応じたリトライ制御のために、コールバックが必要。

## 対応内容

- `SoraClientBuilder` に `on_websocket_close: Arc<dyn Fn(Option<u16>, &str) + Send + Sync>` フィールドを追加する
- Builder メソッド `on_websocket_close` を追加する
- `ConnectionEvent::Close` ハンドラーでコールバックを呼び出す

## 解決方法

`src/client.rs` に以下の変更を加えた:

- `SoraClientBuilder` 構造体に `on_websocket_close` フィールドを追加
- デフォルト値として空のクロージャを設定
- `on_websocket_close` Builder メソッドを追加（シグネチャ: `Fn(Option<u16>, &str)`）
- `run()` メソッド内でコールバックを clone し、`ConnectionEvent::Close` ハンドラーで呼び出し
- `CloseCode` が `Option<CloseCode>` であるため、`code.map(|c| c.0)` で `Option<u16>` に変換
