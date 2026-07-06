# SoraConnection のコールバックをトレイト化する

- Priority: Medium
- Created: 2026-07-06
- Completed: YYYY-MM-DD
- Model: DeepSeek V4 Pro
- Branch: feature/change-callback-to-trait
- Polished: YYYY-MM-DD

## 目的

現状 `SoraConnection` のコールバックは 12 個の独立した `Box<dyn Fn(...) + Send>` として定義されている。
実際の利用ではコールバック間で状態を共有する必要があるが、クロージャ間での共有には `Arc` や mpsc チャネルを経由する必要があり冗長。
コールバック群を単一のトレイト `SoraConnectionEventHandler` に統合することで、ユーザーが自身の struct に状態を持たせ、
`&mut self` による自然な状態共有を実現する。

## 現状

`src/connection.rs` で 12 個のコールバック型エイリアスが定義され（68-79 行目）、
`SoraConnectionBuilder` が 12 個の `Option<Box<dyn Fn(...) + Send>>` フィールドと対応する setter メソッドを持つ（85-330 行目）。

| コールバック | シグネチャ |
|---|---|
| `on_signaling_message` | `Fn(SignalingType, SignalingDirection, &str)` |
| `on_notify` | `Fn(&str)` |
| `on_push` | `Fn(&str)` |
| `on_track` | `Fn(RtpTransceiver)` |
| `on_remove_track` | `Fn(RtpReceiver)` |
| `on_switched` | `Fn()` |
| `on_websocket_close` | `Fn(Option<u16>, &str)` |
| `on_message` | `Fn(&str, &[u8])` |
| `on_data_channel` | `Fn(&str)` |
| `on_data_channel_open` | `Fn(&str)` |
| `on_data_channel_message` | `Fn(&str, &[u8])` |
| `on_data_channel_close` | `Fn(&str)` |

`SoraConnection::run()` 内では各コールバックを `config.on_xxx.take()` で取り出してローカル変数に保持し、
`DataChannelMessageCallbacks` 構造体（828-834 行目）で一部コールバックを束ねている。
`on_track` と `on_remove_track` は `PeerConnectionObserver` (`PcObserverHandler`) に格納される。

## 設計方針

- 新しいトレイト `SoraConnectionEventHandler` を定義する
  - 全 12 メソッドにデフォルトの空実装を提供し、ユーザーが必要なものだけオーバーライドできるようにする
  - 各メソッドのシグネチャは `fn on_xxx(&mut self, ...)` とする
  - `SoraConnectionEventHandler` は `Send` を要求する（現状の `Box<dyn Fn + Send>` と同等）
- `SoraConnectionBuilder` に `.event_handler(impl SoraConnectionEventHandler + Send + 'static)` を追加する
  - 内部では `Box<dyn SoraConnectionEventHandler + Send>` として保持する
  - 既存の 12 個の個別コールバックフィールドと setter は削除する
- `SoraConnection` は `event_handler: Box<dyn SoraConnectionEventHandler + Send>` を保持する
- `SoraConnection::run()` 内のコールバック呼び出しを `self.event_handler.on_xxx(...)` に置き換える
  - `DataChannelMessageCallbacks` は `&mut dyn SoraConnectionEventHandler` 参照に置き換える
  - `on_track` / `on_remove_track` 向けの `PcObserverHandler` には event_handler の参照を渡す
- 破壊的変更として、既存の個別コールバック setter はすべて削除する

## 完了条件

- コールバック用の型エイリアス 12 個が削除されている
- `SoraConnectionEventHandler` トレイトが定義され、全メソッドにデフォルトの空実装がある
- `SoraConnectionBuilder` の 12 個のコールバックフィールドと対応 setter が削除されている
- `SoraConnectionBuilder` に `.event_handler()` メソッドが追加されている
- `DataChannelMessageCallbacks` 構造体が削除またはトレイト参照に置き換えられている
- sumomo 例が新しい API で動作している
- e2e テストが新しい API で動作し、全テストが成功する
- `CHANGES.md` に破壊的変更として記載されている

## 解決方法

（実装時に記述）
