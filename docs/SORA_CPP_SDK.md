# Sora C++ SDK との比較

Sora C++ SDK に対する Sora Rust SDK の実装状況をまとめる。

## connect メッセージのフィールド

| フィールド | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `channel_id` | o | o | |
| `role` | o | o | |
| `client_id` | o | o | |
| `bundle_id` | o | o | |
| `metadata` | o | o | |
| `audio` | o | o | bool またはオブジェクト (codec_type, bit_rate, opus_params) |
| `video` | o | o | bool またはオブジェクト (codec_type, bit_rate, vp9/av1/h264/h265 params) |
| `simulcast` | o | o | |
| `data_channel_signaling` | o | o | |
| `ignore_disconnect_websocket` | o | o | |
| `sora_client` | o | o | クライアント識別文字列 |
| `libwebrtc` | o | o | libwebrtc バージョン |
| `environment` | o | o | 実行環境情報 |
| `multistream` | o | 対象外 | Sora で廃止済みのため実装しない |
| `simulcast_rid` | o | 対象外 | Sora で廃止済みのため実装しない |
| `simulcast_request_rid` | o | o | |
| `spotlight` | o | o | |
| `spotlight_number` | o | 対象外 | Sora で非推奨のため実装しない |
| `spotlight_focus_rid` | o | o | |
| `spotlight_unfocus_rid` | o | o | |
| `signaling_notify_metadata` | o | o | |
| `audio_streaming_language_code` | o | 未実装 | |
| `data_channels` | o | o | ユーザー定義の DataChannel |
| `forwarding_filter` | o | 対象外 | Sora で廃止済みのため実装しない |
| `forwarding_filters` | o | o | |
| `redirect` | o | o | リダイレクト対応 |

## シグナリング機能

| 機能 | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| WebSocket シグナリング | o | o | |
| DataChannel シグナリング | o | o | |
| zlib 圧縮 | o | o | DataChannel メッセージの圧縮・展開 |
| offer / answer | o | o | |
| re-offer / re-answer | o | o | |
| ping / pong | o | o | |
| stats 応答 | o | o | req-stats に対する統計情報応答 |
| notify コールバック | o | o | |
| push コールバック | o | o | |
| on_track コールバック | o | o | |
| on_remove_track コールバック | o | o | |
| switched コールバック | o | o | |
| DataChannel メッセージコールバック | o | o | |
| disconnect | o | o | |
| get_stats | o | o | |
| simulcast encodings 適用 | o | o | サーバーからの encodings 設定を適用 |
| 複数シグナリング URL | o | o | 並列接続・ランダマイズ |
| DataChannel からのユーザーメッセージ送信 | o | o | SendDataChannel |
| 接続状態の取得 | o | 一部実装 | GetSelectedSignalingURL, GetConnectedSignalingURL は実装済み / GetConnectionID は未実装 |
| Rpc 送信 | o | o | C++ SDK: Rpc (コールバック方式) / Rust SDK: send_rpc_request (async/await 方式) |
| OnSignalingMessage コールバック | o | o | シグナリングメッセージ監視 (後述) |
| OnWsClose コールバック | o | o | WebSocket Close 受信時の通知 (後述) |
| OnDataChannel コールバック | o | o | C++ SDK は DataChannel の状態をまとめて通知 |
| OnDataChannelOpen コールバック | - | o | Rust SDK は open を個別通知 |
| OnDataChannelClose コールバック | - | o | Rust SDK は close を個別通知 |
| candidate 対応 | o | 未実装 | ICE candidate の送受信 |

## 複数シグナリング URL

C++ SDK の複数シグナリング URL に準拠した設計。

### 動作

- `SoraClient::builder()` の第 2 引数に `Vec<String>` で複数の URL を指定する
- 接続時に URL リストをランダムにシャッフルして負荷分散する
- `tokio::task::JoinSet` で全 URL に同時に TCP/TLS 接続を試みる
- 最初に接続が成功した URL を採用し、残りの接続試行はキャンセルする
- 全 URL への接続が失敗した場合は `Error::AllSignalingUrlsFailed` を返す
- URL が空の場合は `Error::SignalingUrlsEmpty` を返す

### 接続状態の取得

| メソッド | 説明 |
|---|---|
| `SoraClientHandle::selected_signaling_url()` | 最初に接続が成功した URL を返す |
| `SoraClientHandle::connected_signaling_url()` | 現在接続中の URL を返す (リダイレクト後はリダイレクト先) |

### C++ SDK との比較

| 項目 | C++ SDK | Rust SDK |
|---|---|---|
| URL 指定 | `config.signaling_urls` (`vector<string>`) | `SoraClient::builder()` の第 2 引数 (`Vec<String>`) |
| ランダム化 | デフォルト有効 / `disable_signaling_url_randomization` で無効化可能 | デフォルト有効 |
| 並列接続 | Boost.Asio の非同期接続 | `tokio::task::JoinSet` |
| 残接続のキャンセル | `ws->Close()` で明示的にクローズ | `JoinSet::abort_all()` でタスクキャンセル |
| URL 追跡 | `GetSelectedSignalingURL()` / `GetConnectedSignalingURL()` | `selected_signaling_url()` / `connected_signaling_url()` |
| ランダム化無効化 | o | 未実装 |

## OnWsClose コールバック

WebSocket の Close フレーム受信時に呼び出されるコールバック。

- コールバックのシグネチャ: `Fn(Option<u16>, &str)`
  - 第 1 引数: close code (`Option<u16>`)。サーバーが close code を送信しなかった場合は `None`
  - 第 2 引数: reason (`&str`)
- `ConnectionEvent::Close` の受信時、WebSocket ループを抜ける前に呼び出される

### C++ SDK との比較

| 項目 | C++ SDK | Rust SDK |
|---|---|---|
| コールバック名 | `OnWsClose` | `on_websocket_close` |
| close code の型 | `int` | `Option<u16>` |
| reason の型 | `std::string` | `&str` |

## OnSignalingMessage コールバック

C++ SDK の OnSignalingMessage に準拠した設計。

- 受信: offer, re-offer, redirect (WebSocket)、signaling ラベルのみ (DataChannel)
- 送信: 全送信メッセージ
- notify, push, ping/pong, stats 等は対象外 (専用コールバックで処理)
- `SignalingType` で経路 (WebSocket / DataChannel) を区別する
- `SignalingDirection` で方向 (Sent / Received) を区別する

## OnDataChannel 系コールバック

C++ SDK は `OnDataChannel` コールバックで、DataChannel の状態をまとめて通知する。
個別のイベントごとに別コールバックがある設計ではない。

Rust SDK は DataChannel のライフサイクルを 3 つのコールバックに分離している。

| コールバック | C++ SDK | Rust SDK | タイミング |
|---|---|---|---|
| `on_data_channel` | o | o | C++ SDK では DataChannel の状態通知、Rust SDK では DataChannel が作成された時 |
| `on_data_channel_open` | - | o | DataChannel が開いた時 |
| `on_data_channel_close` | - | o | DataChannel が閉じた時 |

Rust SDK の各コールバックは引数としてラベル (`&str`) を受け取る。

## RPC

C++ SDK と Rust SDK で設計が異なる。

| 項目 | C++ SDK | Rust SDK |
|---|---|---|
| 方式 | コールバック (`OnRpc`) | async/await (`send_rpc_request`) |
| 送信メソッド名 | `Rpc` | `send_rpc_request` |
| メッセージ組み立て | アプリケーション側 | SDK 内部 (JSON-RPC 2.0) |
| id 採番 | アプリケーション側 | SDK 内部で自動採番 |
| notification | - | `RpcRequestOptions::notification` で指定 |
| タイムアウト | - | `RpcRequestOptions::timeout` で指定 |

C++ SDK はコールバック方式で `OnRpc` が送受信両方を扱う。
Rust SDK は async/await 方式で `send_rpc_request` が JSON-RPC 2.0 メッセージの組み立て、送信、レスポンス受信を一つのメソッドで行う。

### シグネチャ

```rust
pub async fn send_rpc_request(
    &self,
    method: &str,
    params: Option<JsonString>,
    options: RpcRequestOptions,
) -> Result<Option<RpcResponse>>
```

- `method`: JSON-RPC 2.0 の method 名
- `params`: JSON-RPC 2.0 の params (Object または Array の JSON 文字列、省略可)
- `options`: リクエストオプション

### RpcRequestOptions

| フィールド | 型 | デフォルト | 説明 |
|---|---|---|---|
| `notification` | `bool` | `false` | `true` の場合はレスポンスを待たずに即座に `Ok(None)` を返す |
| `timeout` | `Duration` | `5秒` | レスポンスの待機タイムアウト |

### 動作

- SDK 内部で JSON-RPC 2.0 メッセージを組み立てて DataChannel 経由で送信する
- `id` は SDK 内部で自動採番する
- `notification` が `true` の場合は `id` を付与せず、送信後に即座に `Ok(None)` を返す
- `notification` が `false` の場合はレスポンスの `id` を突き合わせて対応するリクエストに返す
- 複数の RPC リクエストを同時に送信できる
- タイムアウトした場合は `Error::RpcTimeout` を返し、その後にレスポンスが届いても破棄する

## 接続設定

| 機能 | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| TLS 接続 (wss://) | o | o | |
| 非 TLS 接続 (ws://) | o | o | |
| HTTP Proxy | o | 未実装 | |
| WebSocket insecure モード | o | o | WebSocket の SSL 検証スキップ |
| TURN-TLS insecure モード | o | o | TURN-TLS の SSL 検証スキップ |
| WebSocket クライアント証明書 | o | o | WebSocket の client_cert / client_key |
| TURN-TLS クライアント証明書 | o | 未実装 | TURN-TLS の client_cert / client_key (webrtc-rs へのパッチが必要) |
| WebSocket CA 証明書指定 | o | o | WebSocket の CA 証明書 |
| TURN-TLS CA 証明書指定 | o | o | TURN-TLS の CA 証明書 |
| User-Agent カスタマイズ | o | o | デフォルト "Sora Rust SDK {version}" |
| WebSocket 接続タイムアウト | o | o | デフォルト 30 秒 |
| WebSocket 閉じるタイムアウト | o | o | デフォルト 3 秒 |
| DataChannel シグナリングタイムアウト | o | 未実装 | C++ SDK でも未使用 |
| 切断待機タイムアウト | o | o | デフォルト 5 秒 |
| degradation_preference | o | 未実装 | |
| cpu_adaptation | o | 未実装 | |
