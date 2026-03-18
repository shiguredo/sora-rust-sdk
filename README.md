# Sora Rust SDK

[![crates.io](https://img.shields.io/crates/v/sora_sdk.svg)](https://crates.io/crates/sora_sdk)
[![docs.rs](https://docs.rs/sora_sdk/badge.svg)](https://docs.rs/sora_sdk)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![GitHub Actions](https://github.com/shiguredo/sora-rust-sdk/actions/workflows/ci.yml/badge.svg)](https://github.com/shiguredo/sora-rust-sdk/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white)](https://discord.gg/shiguredo)

Sora Rust SDK は [WebRTC SFU Sora](https://sora.shiguredo.jp/) の Rust クライアントアプリケーションを開発するためのライブラリです。

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss/blob/master/README.en.md> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## Sora Rust SDK について

様々なプラットフォームに対応した WebRTC SFU Sora 向けの Rust SDK です。

## 特徴

- マルチストリーム対応
- サイマルキャスト対応
- スポットライト対応
- DataChannel シグナリング対応
- DataChannel メッセージング対応
- JSON-RPC over DataChannel 対応
- 転送フィルター対応
- シグナリング通知対応
- シグナリングリダイレクト対応
- 複数シグナリング URL 対応
- メタデータ認証対応
- シグナリング通知メタデータ対応
- クライアント証明書認証対応
- TURN-TLS 対応
- HTTP プロキシ対応
- VP8 / VP9 / H.264 / H.265 / AV1 対応
- 複数クライアント同時実行対応

## 使い方

### 依存関係の追加

`Cargo.toml` に以下を追加してください。

```toml
[dependencies]
sora_sdk = "<version>"
shiguredo_webrtc = "<version>"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
```

`shiguredo_webrtc` は `VideoTrack` や `AudioTrack`、`RtpTransceiver` など sora_sdk の公開 API で必要な型を提供します。

### sendrecv で接続する

映像・音声を送受信する例です。

```rust
use sora_sdk::{Role, SoraClient, SoraClientContext};

#[tokio::main]
async fn main() -> Result<(), sora_sdk::Error> {
    // 1. SoraClientContext を作成する
    //    PeerConnectionFactory や WebRTC 関連スレッドを管理します。
    //    複数の SoraClient で共有できます。
    let context = SoraClientContext::new()?;

    // 2. AudioTrack を作成する
    let audio_source = context.create_audio_source()?;
    let audio_track = context.create_audio_track(&audio_source)?;

    // VideoTrackSource は shiguredo_webrtc クレートから作成する
    // (FakeVideoCapturer や AdaptedVideoTrackSource など)

    // 3. SoraClient::builder() で接続設定を組み立てる
    let (client, _handle) = SoraClient::builder(
        context,
        vec!["wss://sora.example.com/signaling".to_string()],
        "your-channel-id".to_string(),
        Role::SendRecv,
    )
    .sender_audio_track(audio_track)
    // .sender_video_track(video_track)
    .on_notify(|text| {
        println!("notify: {text}");
    })
    .on_track(|transceiver| {
        println!("track added: {:?}", transceiver.mid());
    })
    .build()?;

    // 4. client.run() で Sora に接続する
    //    接続が終了するまでブロックします。
    //    handle.disconnect() で切断できます。
    client.run().await?;

    Ok(())
}
```

### sendonly で接続する

映像・音声を送信する例です。

```rust
use sora_sdk::{Role, SoraClient, SoraClientContext};

#[tokio::main]
async fn main() -> Result<(), sora_sdk::Error> {
    let context = SoraClientContext::new()?;

    // AudioTrack を作成する
    let audio_source = context.create_audio_source()?;
    let audio_track = context.create_audio_track(&audio_source)?;

    // VideoTrackSource は shiguredo_webrtc クレートから作成する
    // (FakeVideoCapturer や AdaptedVideoTrackSource など)

    let (client, _handle) = SoraClient::builder(
        context,
        vec!["wss://sora.example.com/signaling".to_string()],
        "your-channel-id".to_string(),
        Role::SendOnly,
    )
    .sender_audio_track(audio_track)
    // .sender_video_track(video_track)
    .build()?;

    client.run().await?;

    Ok(())
}
```

### recvonly で接続する

映像・音声を受信する例です。

```rust
use sora_sdk::{Role, SoraClient, SoraClientContext};

#[tokio::main]
async fn main() -> Result<(), sora_sdk::Error> {
    let context = SoraClientContext::new()?;

    let (client, _handle) = SoraClient::builder(
        context,
        vec!["wss://sora.example.com/signaling".to_string()],
        "your-channel-id".to_string(),
        Role::RecvOnly,
    )
    .on_notify(|text| {
        println!("notify: {text}");
    })
    .build()?;

    client.run().await?;

    Ok(())
}
```

### SoraClient::builder() の設定

`SoraClient::builder()` では以下の設定が可能です。

```rust
let (client, handle) = SoraClient::builder(context, signaling_urls, channel_id, role)
    // コールバック
    .on_signaling_message(|type_, direction, text| { /* シグナリングメッセージ送受信時 */ })
    .on_notify(|text| { /* notify メッセージ受信時 */ })
    .on_push(|text| { /* push メッセージ受信時 */ })
    .on_track(|transceiver| { /* トラック追加時 */ })
    .on_remove_track(|receiver| { /* トラック削除時 */ })
    .on_switched(|| { /* DataChannel に切り替わった時 */ })
    .on_websocket_close(|code, reason| { /* WebSocket 切断時 */ })
    // メッセージング
    .on_message(|label, data| { /* メッセージ受信時 */ })
    // DataChannel
    .on_data_channel(|label| { /* DataChannel 作成時 */ })
    .on_data_channel_open(|label| { /* DataChannel オープン時 */ })
    .on_data_channel_message(|label, data| { /* DataChannel メッセージ受信時 */ })
    .on_data_channel_close(|label| { /* DataChannel クローズ時 */ })
    // 送信トラック
    .sender_video_track(video_track)
    .sender_audio_track(audio_track)
    // 接続オプション
    .client_id("client-1".to_string())           // クライアント ID
    .bundle_id("bundle-1".to_string())           // バンドル ID
    .metadata(metadata)                          // メタデータ (認証用など)
    .audio(audio)                                // 音声設定
    .video(video)                                // 映像設定
    .data_channel_signaling(true)                // DataChannel シグナリング
    .ignore_disconnect_websocket(true)           // WebSocket 切断を無視
    .simulcast(true)                             // サイマルキャスト
    .simulcast_request_rid("r0".to_string())     // サイマルキャスト rid 指定
    .spotlight(true)                             // スポットライト
    .spotlight_focus_rid("r1".to_string())       // スポットライト フォーカス rid
    .spotlight_unfocus_rid("r0".to_string())     // スポットライト アンフォーカス rid
    .signaling_notify_metadata(notify_metadata)  // シグナリング通知メタデータ
    .data_channels(data_channels)                // DataChannel 設定
    .forwarding_filters(filters)                 // 転送フィルター
    // TLS 設定
    .insecure(false)                             // サーバー証明書の検証をスキップ
    .client_cert(cert, key)                      // クライアント証明書 (PEM)
    .ca_cert(ca)                                 // CA 証明書 (PEM)
    // TURN-TLS 設定
    .turn_tls_insecure(false)                    // TURN-TLS 証明書の検証をスキップ
    .turn_tls_ca_cert(der)                       // TURN-TLS CA 証明書 (DER)
    // プロキシ設定
    .proxy(proxy_info)                           // HTTP プロキシ
    // タイムアウト設定
    .websocket_connection_timeout(Duration::from_secs(30))
    .websocket_close_timeout(Duration::from_secs(3))
    .disconnect_wait_timeout(Duration::from_secs(5))
    // その他
    .user_agent("MyApp/1.0".to_string())         // WebSocket User-Agent
    .build()?;
```

### 切断と統計情報の取得

`SoraClient::builder().build()` が返す `SoraClientHandle` を使って、別タスクから切断や統計情報の取得ができます。

```rust
// 別タスクから切断する
let handle_clone = handle.clone();
tokio::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    let _ = handle_clone.disconnect().await;
});

// 統計情報を取得する
let stats = handle.get_stats().await?;
```

### RPC

`SoraClientHandle` を使って JSON-RPC 2.0 over DataChannel でリクエストを送信できます。
SDK が JSON-RPC 2.0 メッセージの組み立てと id 採番を行います。

```rust
use sora_sdk::{JsonString, RpcRequestOptions, RpcResponse};

// リクエストを送信してレスポンスを待つ (デフォルトタイムアウト 5 秒)
let params: JsonString = r#"{"key": "value"}"#.parse()?;
let response = handle.send_rpc_request(
    "method_name",
    Some(params),
    RpcRequestOptions::default(),
).await?;

match response {
    Some(RpcResponse::Success { result }) => {
        // result: JsonString
    }
    Some(RpcResponse::Error { code, message, data }) => {
        // code: i32, message: String, data: Option<JsonString>
    }
    None => {
        // notification の場合
    }
}

// notification (レスポンスを待たない)
let response = handle.send_rpc_request(
    "method_name",
    None,
    RpcRequestOptions {
        notification: true,
        ..Default::default()
    },
).await?;
// response: None

// タイムアウトを 10 秒に変更
let response = handle.send_rpc_request(
    "method_name",
    None,
    RpcRequestOptions {
        timeout: std::time::Duration::from_secs(10),
        ..Default::default()
    },
).await?;
```

### メッセージ送信

`SoraClientHandle` を使って `#` プレフィックス付きラベルのユーザー定義 DataChannel にバイナリデータを送信できます。

```rust
handle.send_message("#my-channel", b"hello").await?;
```

### 複数クライアントの同時実行

`SoraClientContext` は `Send + Sync` を実装しているため、複数の `SoraClient` で共有できます。

```rust
let context = SoraClientContext::new()?;

for i in 0..5 {
    let ctx = context.clone();
    let channel_id = format!("channel-{i}");
    tokio::spawn(async move {
        let (client, _handle) = SoraClient::builder(
            ctx,
            vec!["wss://sora.example.com/signaling".to_string()],
            channel_id,
            Role::RecvOnly,
        )
        .build()
        .unwrap();
        let _ = client.run().await;
    });
}
```

## 構成

```
sora-rust-sdk/
├── src/                      # sora_sdk クレート
├── examples/
│   ├── sumomo/               # Sora クライアントサンプル
│   └── zakuro/               # Sora 負荷試験ツール
└── e2e-tests/                # エンドツーエンドテスト
```

## サンプル

### sumomo

Sora クライアントのサンプルです。詳細は [examples/sumomo/README.md](examples/sumomo/README.md) を参照してください。

```bash
cargo run -p sumomo -- \
    --signaling-url wss://sora.example.com/signaling \
    --channel-id your-channel-id \
    --role sendrecv
```

### zakuro

Sora WebRTC SFU 負荷試験ツールです。将来的に [zakuro](https://github.com/shiguredo/zakuro) (C++ 版) を置き換える予定です。
詳細は [examples/zakuro/README.md](examples/zakuro/README.md) を参照してください。

```bash
cargo run -p zakuro -- \
    --sora-signaling-url wss://sora.example.com/signaling \
    --sora-channel-id zakuro-test \
    --sora-role sendonly \
    --vcs 10
```

## ビルド

### 前提条件

- Rust 1.88 以上
- libclang (bindgen 用)
- Python 3 (webrtc ビルド用)

### ビルド手順

```bash
cargo build
```

## 対応 WebRTC SFU Sora

- Sora 2025.1.0 以降

## 対応プラットフォーム

- Ubuntu 24.04 LTS x86_64
- Ubuntu 24.04 LTS arm64
- Ubuntu 22.04 LTS x86_64
- Ubuntu 22.04 LTS arm64
- macOS Tahoe 26 arm64
- macOS Sequoia 15 arm64

### Ubuntu の対応バージョン

直近の LTS 2 バージョンをサポートします。

### macOS の対応バージョン

直近の 2 バージョンをサポートします。

### 将来対応予定

- Windows 11 x86_64
- Windows Server 2025 x86_64

## 優先実装

優先実装とは Sora のライセンスを契約頂いているお客様限定で Sora Rust SDK の実装予定機能を有償にて前倒しで実装することです。

### 優先実装が可能な対応一覧

**詳細は Discord やメールなどでお気軽にお問い合わせください**

- Windows x86_64 対応
- Windows arm64 対応

## サポートについて

### Discord

- **サポートしません**
- アドバイスします
- フィードバック歓迎します

最新の状況などは Discord で共有しています。質問や相談も Discord でのみ受け付けています。

<https://discord.gg/shiguredo>

### バグ報告

Discord へお願いします。

## ライセンス

Apache License 2.0

```text
Copyright 2026-2026, Wandbox LLC (Original Author)
Copyright 2026-2026, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```
