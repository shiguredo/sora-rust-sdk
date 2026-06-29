---
name: sora-rust-sdk
description: 時雨堂の WebRTC SFU Sora 向け Rust クライアント SDK sora_sdk の機能・API リファレンス。SoraConnectionContext / SoraConnection / SoraConnectionHandle による接続管理、Audio / Video 設定、VideoCodecPreference / VideoCodecCapability によるコーデック選択、DataChannel メッセージング、JSON-RPC 2.0 over DataChannel、TLS / TURN-TLS / HTTP プロキシ設定、複数クライアント同時実行に関する質問時に使用。
---

# sora_sdk

WebRTC SFU Sora のクライアントを Rust で実装するための SDK。シグナリング、メディア送受信、DataChannel メッセージング、JSON-RPC、コーデックバックエンド統合を提供する。

## 特徴

- **Sora シグナリング**: WebSocket / DataChannel シグナリング両対応。複数シグナリング URL のレース接続、リダイレクト対応。
- **複数ロール**: `sendonly` / `recvonly` / `sendrecv` をサポート。
- **メディア機能**: マルチストリーム、サイマルキャスト、スポットライト、転送フィルター、シグナリング通知。
- **コーデック**: VP8 / VP9 / AV1 / H.264 / H.265。OpenH264 / Apple VideoToolbox / AMD AMF / NVIDIA Video Codec / Intel VPL / V4L2-M2M のバックエンド統合。
- **MP4 無変換送信**: `Mp4PassthroughVideoCodecCapability` で MP4 ファイルの音声・映像トラックをデコード/エンコードを挟まずに Sora へ送信。
- **DataChannel メッセージング**: `#` プレフィックスのユーザー定義 DataChannel でバイナリ送受信。
- **JSON-RPC 2.0 over DataChannel**: SDK が id 採番とエンベロープを担当。
- **TURN-TLS**: 独自 CA 証明書 (DER) による検証。
- **HTTP プロキシ (CONNECT)**: `http://` のみ対応。Basic 認証可。
- **複数クライアント同時実行**: 1 つの `SoraConnectionContext` を `Arc` で共有して N 接続。

## バージョン情報

- crate 名: `sora_sdk`
- バージョン: 2026.1.0-canary.10
- Rust Edition: 2024
- 最小 Rust バージョン: 1.88
- ライセンス: Apache-2.0
- 対応 Sora: 2025.1.0 以降
- 対応プラットフォーム: Ubuntu 22.04 / 24.04 (x86_64, arm64), macOS 15 / 26 (arm64), Windows 11 / Server 2025 (x86_64)

`shiguredo_webrtc` クレートが提供する `AudioTrack` / `VideoTrack` / `VideoTrackSource` / `RtpTransceiver` / `RtpReceiver` / `IceServer` 等を直接受け取る公開 API があるため、利用側の `Cargo.toml` に `shiguredo_webrtc` を追加する必要がある。

## Cargo features

| feature | 既定 | 内容 |
|---------|------|------|
| `openh264` | 有効 | OpenH264 による H.264 ソフトウェアエンコード/デコード |
| `amf` | 無効 | AMD AMF によるハードウェアエンコード/デコード (Windows / Linux) |
| `nvcodec` | 無効 | NVIDIA Video Codec によるハードウェアエンコード/デコード (Windows / Linux) |
| `vpl` | 無効 | Intel VPL によるハードウェアエンコード/デコード (Windows / Linux) |
| `v4l2` | 無効 | V4L2-M2M によるハードウェアエンコード/デコード (Raspberry Pi) |
| `libcamera` | 無効 | libcamera による映像入力 (Raspberry Pi) |

機能フラグは加算式。複数バックエンドを同時に有効化可能。フラグを有効にしても GPU / ドライバが無ければ各 `*::new()` がランタイムでエラーを返す。

## コア API

### 接続コンテキスト

| 型 | 説明 | 主要メソッド |
|----|------|-------------|
| `SoraConnectionContext` | `PeerConnectionFactory` と内部スレッド (network / worker / signaling) をまとめて保持。プロセス全体で 1 つ作って `Arc` で共有する | `new() -> Result<Arc<Self>>`, `new_with_config(SoraConnectionContextConfig) -> Result<Arc<Self>>`, `create_audio_source() -> Result<AudioTrackSource>`, `create_audio_track(&AudioTrackSource) -> Result<AudioTrack>`, `create_video_track(&VideoTrackSource) -> Result<VideoTrack>` |
| `SoraConnectionContextConfig` | コンテキストの設定 (フィールド: `adm_config`, `video_codec_preference`, `video_codec_capabilities`) | `Default::default()` (Internal / InternalApple capabilities を自動登録) |
| `AdmConfig` | AudioDeviceModule の選択 | `NoAudioDevice` (既定、Dummy ADM), `UseBuiltIn` (OS 標準), `UseExternal(shiguredo_webrtc::AudioDeviceModule)` |

`AudioTrackSource` はコンテキストから生成する。`VideoTrackSource` は `shiguredo_webrtc` 側 (`FakeVideoCapturer` / `AdaptedVideoTrackSource`) または本クレートの `Mp4VideoCapturer` / `LibcameraVideoCapturer` から生成する。

### 接続ビルダー

| 型 | 説明 |
|----|------|
| `SoraConnection` | 接続本体。`run()` でシグナリングからメディア接続までを駆動する |
| `SoraConnectionBuilder` | ビルダー。ムーブスタイルで連結し `.build()` で `(SoraConnection, SoraConnectionHandle)` を返す |
| `SoraConnectionCommand` | `SoraConnectionHandle` が内部的に送信するコマンドの enum。通常はユーザーが直接構築しない |
| `TlsConfig` | WebSocket (シグナリング接続) の TLS 設定。`insecure` / `client_cert` / `client_key` / `ca_cert` を保持。Builder の `insecure` / `client_cert` / `ca_cert` メソッド経由で設定する |

`SoraConnection::builder(context, signaling_urls, channel_id, role) -> SoraConnectionBuilder` で開始。`signaling_urls` は `Vec<String>`、`channel_id` は `String`、`role` は `Role`。

#### コールバック設定

| メソッド | クロージャ署名 | 説明 |
|----------|---------------|------|
| `on_signaling_message` | `Fn(SignalingType, SignalingDirection, &str)` | シグナリングメッセージ送受信時 |
| `on_notify` | `Fn(&str)` | notify メッセージ受信時 |
| `on_push` | `Fn(&str)` | push メッセージ受信時 |
| `on_track` | `Fn(RtpTransceiver)` | トラック追加時 |
| `on_remove_track` | `Fn(RtpReceiver)` | トラック削除時 |
| `on_switched` | `Fn()` | DataChannel シグナリングに切り替わった時 |
| `on_websocket_close` | `Fn(Option<u16>, &str)` | WebSocket 切断時 |
| `on_message` | `Fn(&str, &[u8])` | `#` プレフィックス DataChannel 受信時 |
| `on_data_channel` | `Fn(&str)` | DataChannel 作成時 |
| `on_data_channel_open` | `Fn(&str)` | DataChannel オープン時 |
| `on_data_channel_message` | `Fn(&str, &[u8])` | DataChannel メッセージ受信時 |
| `on_data_channel_close` | `Fn(&str)` | DataChannel クローズ時 |

全コールバックは `Fn + Send + Sync + 'static`。内部タスクから呼ばれるためブロックさせないこと。重い処理はチャンネルで自分のタスクへ転送する。

#### 送信トラック

| メソッド | 引数 | 説明 |
|----------|------|------|
| `sender_audio_track` | `AudioTrack` | 送信する音声トラック |
| `sender_video_track` | `VideoTrack` | 送信する映像トラック |

#### 接続オプション

| メソッド | 引数 | 説明 |
|----------|------|------|
| `client_id` | `String` | クライアント ID |
| `bundle_id` | `String` | バンドル ID |
| `metadata` | `JsonString` | 認証用メタデータ |
| `audio` | `Audio` | 音声設定 |
| `video` | `Video` | 映像設定 |
| `data_channel_signaling` | `bool` | DataChannel シグナリング有効化 |
| `ignore_disconnect_websocket` | `bool` | DataChannel 切替後の WebSocket 切断を無視 |
| `simulcast` | `bool` | サイマルキャスト有効化 |
| `simulcast_request_rid` | `String` | 受信したい rid |
| `spotlight` | `bool` | スポットライト有効化 |
| `spotlight_focus_rid` | `String` | フォーカス rid |
| `spotlight_unfocus_rid` | `String` | アンフォーカス rid |
| `signaling_notify_metadata` | `JsonString` | シグナリング通知メタデータ |
| `data_channels` | `Vec<ConnectDataChannel>` | DataChannel 設定 |
| `forwarding_filters` | `Vec<ForwardingFilter>` | 転送フィルター |
| `ice_server_url_configurer` | `Fn(&mut IceServer, &[String])` | ICE server URL のフィルタ/書き換え |
| `user_agent` | `String` | WebSocket User-Agent (未指定時は `Sora Rust SDK {version}`) |

#### タイムアウト

| メソッド | 既定値 | 説明 |
|----------|--------|------|
| `websocket_connection_timeout` | 30 秒 | WebSocket 接続タイムアウト |
| `websocket_close_timeout` | 3 秒 | WebSocket クローズ待機タイムアウト |
| `disconnect_wait_timeout` | 5 秒 | 切断完了待機タイムアウト |

#### TLS / TURN-TLS / プロキシ

| メソッド | 引数 | 説明 |
|----------|------|------|
| `insecure` | `bool` | サーバー証明書検証スキップ (本番では使わない) |
| `client_cert` | `cert: String, key: String` | クライアント証明書と秘密鍵 (PEM) |
| `ca_cert` | `String` | CA 証明書 (PEM) |
| `turn_tls_insecure` | `bool` | TURN-TLS 証明書検証スキップ |
| `turn_tls_ca_cert` | `Vec<u8>` | TURN-TLS の CA 証明書 (DER) |
| `proxy` | `ProxyInfo` | HTTP プロキシ (CONNECT 経由、`http://` のみ) |

WebSocket TLS は PEM、TURN-TLS は DER である点に注意。

### 接続ハンドル

| 型 | 説明 |
|----|------|
| `SoraConnectionHandle` | `Clone`。`SoraConnection::run()` を別タスクで実行中に外部から制御するためのハンドル |

| メソッド | 戻り値 | 説明 |
|----------|--------|------|
| `selected_signaling_url()` | `Result<Option<String>>` | 最初に WebSocket 接続成功したシグナリング URL |
| `connected_signaling_url()` | `Result<Option<String>>` | 現在接続中のシグナリング URL (リダイレクト後はリダイレクト先) |
| `disconnect()` | `Result<()>` | 切断要求 |
| `send_message(label, data)` | `Result<()>` | `#` プレフィックス DataChannel にバイナリ送信 |
| `send_rpc_request(method, params, options)` | `Result<Option<RpcResponse>>` | JSON-RPC 2.0 リクエスト送信 |
| `get_stats()` | `Result<JsonString>` | PeerConnection の統計情報 (JSON) |

`run()` 終了後はいずれも `Error::CommandSendFailed` などのエラーを返す。

### 接続実行

`SoraConnection::run(self) -> Result<()>` は `async fn` で、接続が終了するまでブロックする。通常は `tokio::spawn` で別タスクに渡し、`SoraConnectionHandle` で外部から `disconnect()` を呼び出す。

## 接続設定の型

### Role

```rust
pub enum Role { SendOnly, RecvOnly, SendRecv }
```

| メソッド | 説明 |
|----------|------|
| `Role::parse(&str)` | `"sendonly" \| "recvonly" \| "sendrecv"` をパース。不正値は `Error::InvalidRole` |
| `Role::as_sora_role()` | Sora シグナリング上の文字列表現 |
| `Role::wants_send()` / `wants_recv()` | 方向判定 |

### Audio / Video

| 型 | バリアント / フィールド |
|----|----------------------|
| `Audio` | `Bool(bool)`, `Audio { codec_type: Option<AudioCodecType>, bit_rate: Option<u32>, opus_params: Option<AudioOpusParams> }` |
| `AudioCodecType` | `OPUS` |
| `AudioOpusParams` | `channels`, `maxplaybackrate`, `minptime`, `ptime`, `stereo`, `sprop_stereo`, `useinbandfec`, `usedtx` (すべて `Option`) |
| `Video` | `Bool(bool)`, `Video { codec_type, bit_rate, vp9_params, av1_params, h264_params, h265_params }` |
| `VideoCodecType` | `VP8`, `VP9`, `H264`, `H265`, `AV1` |
| `VideoVP9Params` | `profile_id: Option<u32>` (0..3) |
| `VideoH264Params` | `profile_level_id: Option<String>`, `b_frame: Option<bool>` |
| `VideoH265Params` | `level_id: Option<String>`, `profile_id: Option<u32>` (0..31), `tier_flag: Option<u32>` (0..1), `tx_mode: Option<String>` (`"SRST" \| "MRST" \| "MRMT"`), `b_frame: Option<bool>` |
| `VideoAV1Params` | `profile: Option<u32>` (0..2), `level_idx: Option<u32>` (0..31), `tier: Option<u32>` (0..1) |

コンストラクタ:

| 関数 | 用途 |
|------|------|
| `Audio::new_bool(bool)` | 音声の有効/無効のみ |
| `Audio::new_opus(bit_rate, opus_params)` | Opus 音声 |
| `Video::new_bool(bool)` | 映像の有効/無効のみ |
| `Video::new_vp8(bit_rate)` | VP8 |
| `Video::new_vp9(bit_rate, vp9_params)` | VP9 |
| `Video::new_av1(bit_rate, av1_params)` | AV1 |
| `Video::new_h264(bit_rate, h264_params)` | H.264 |
| `Video::new_h265(bit_rate, h265_params)` | H.265 |

H.264 / H.265 の `b_frame: true` は Sora 側の `sora.conf` で対応する設定が必要。

### DataChannel / 転送フィルター / 通知

| 型 | フィールド |
|----|----------|
| `ConnectDataChannel` | `label: String`, `direction: String`, `ordered: Option<bool>`, `max_packet_life_time: Option<i32>`, `max_retransmits: Option<i32>`, `protocol: Option<String>`, `compress: Option<bool>`, `header: Option<Vec<JsonString>>` |
| `ForwardingFilter` | `name: Option<String>`, `priority: Option<i32>`, `action: Option<String>`, `rules: Vec<Vec<ForwardingFilterRule>>`, `version: Option<String>`, `metadata: Option<JsonString>` |
| `ForwardingFilterRule` | `field: String`, `operator: String`, `values: Vec<String>` |
| `ProxyInfo` | `url: String`, `username: Option<String>`, `password: Option<String>`, `user_agent: Option<String>` |
| `ParsedProxyInfo` | `ProxyInfo::parse()` で検証済みのプロキシ接続情報を取得する公開型。全フィールドは非公開で accessor (`host()` / `port()` / `username()` / `password()` / `user_agent()`) 経由で取得する。主に PBT 用途で公開されている |
| `JsonString` | `nojson::RawJsonOwned` のラッパー。`str::parse::<JsonString>()` で構築 (不正 JSON は `Error::JsonParse`) |
| `SignalingType` | `WebSocket`, `DataChannel` |
| `SignalingDirection` | `Sent`, `Received` |

## コーデック選択

| 型 | 説明 |
|----|------|
| `VideoCodecPreference` | コーデック選好。`default()` で空。`new(Vec<PreferenceCodec>)` / `new_from_capability(&dyn VideoCodecCapability)` で生成。`codecs()` / `find(direction, codec_type)` / `find_mut(...)` / `get_or_add(...)` / `has_implementation(impl)` / `merge(&other)` を提供 |
| `PreferenceCodec` | preference 内のコーデックエントリ。`new(direction, codec_type, implementation)` で生成。`direction()` / `codec_type()` / `implementation()` / `set_implementation(impl)` を提供 |
| `VideoCodecCapability` | トレイト (`: Send`)。各バックエンドが実装する。`SoraConnectionContextConfig::video_codec_capabilities` に `Box<dyn VideoCodecCapability>` を積む。必須メソッドは `get_implementation()` と `get_supported_formats(direction)`。デフォルト実装つきメソッドは `is_supported(direction, codec_type)` / `resolve_sdp_format(direction, format)` / `create_video_encoder(env, format) -> Option<VideoEncoder>` / `create_video_decoder(env, format) -> Option<VideoDecoder>` |
| `VideoCodecImplementation` | 実装識別。`new(name, description)` で生成。`name()` / `description()` を提供 |
| `CodecDirection` | encoder / decoder の方向。`as_str()` (`"Encoder"` / `"Decoder"`) / `as_label()` (`"encoder"` / `"decoder"`) を提供 |
| `validate_video_codec_preference(&preference, &[Box<dyn VideoCodecCapability>])` | `new_with_config` 内部でも呼ばれる整合性チェック。preference と capabilities が一致しない場合 `Error::InvalidVideoCodecPreference` |
| `SoraVideoEncoderFactory` / `SoraVideoDecoderFactory` | 内部で利用される factory (通常はユーザーが直接触らない) |
| `AlignmentEncoderAdapter` | エンコーダーのアライメント補正アダプタ |
| `SimulcastCapabilityHelper` | サイマルキャスト対応判定ヘルパー |
| `codec_type_from_format(&SdpVideoFormatRef)` | フォーマットから `VideoCodecType` を解決 |

### 標準のコーデックバックエンド

| 型 | feature / 条件 | 用途 |
|----|---------------|------|
| `InternalVideoCodecCapability` | 常時 | libwebrtc 内蔵 (VP8 / VP9 / AV1 など) |
| `InternalAppleVideoCodecCapability` | macOS / iOS | VideoToolbox による H.264 / H.265 |
| `Mp4PassthroughVideoCodecCapability` | 常時 | MP4 ファイル無変換送信 |
| `Openh264VideoCodecCapability` | `openh264` | OpenH264 ソフトウェア H.264 |
| `AmfVideoCodecCapability` | `amf` | AMD AMF |
| `NvCodecVideoCodecCapability` | `nvcodec` | NVIDIA Video Codec |
| `VplVideoCodecCapability` | `vpl` | Intel VPL |
| `V4l2VideoCodecCapability` | `v4l2` | V4L2-M2M (Raspberry Pi) |

新しい capability を加えるたびに、対応する `VideoCodecPreference` を `merge` して preference 側にも追加すること。`SoraConnectionContextConfig::default()` は Internal と (macOS/iOS 上では) InternalApple を自動登録する。

## MP4 / libcamera

| 型 | feature / 条件 | 説明 |
|----|---------------|------|
| `Mp4SampleReader` | 常時 | MP4 ファイルからサンプルを取得 |
| `Mp4EncodedSample` | 常時 | エンコード済みサンプル |
| `Mp4VideoCapturer` | 常時 | `VideoTrackSource` 互換のキャプチャ。`Mp4PassthroughVideoCodecCapability` と組で使う |
| `Mp4Error` | 常時 | MP4 関連のエラー |
| `LibcameraVideoCapturer` | `libcamera` | libcamera 経由の映像入力 |
| `LibcameraVideoCapturerBuilder` | `libcamera` | 上記のビルダー |
| `LibcameraNativeFrameBuffer` | `libcamera` | libcamera のフレームバッファ |

## DataChannel メッセージング / JSON-RPC 2.0

| 動作 | API |
|------|-----|
| ユーザー定義 DataChannel をシグナリング時に宣言 | `SoraConnectionBuilder::data_channels(Vec<ConnectDataChannel>)` |
| 受信ハンドラ | `SoraConnectionBuilder::on_message(Fn(&str, &[u8]))` (ラベルは `#` 始まり) |
| 送信 | `SoraConnectionHandle::send_message(label, data)` |

JSON-RPC 2.0 は SDK 側で `{ "jsonrpc": "2.0", "method": ..., "params": ..., "id": ... }` を組み立てる。利用側は内側の `params` を `Option<JsonString>` で渡すだけ (パラメータ不要なら `None`)。

| 型 | フィールド / バリアント |
|----|----------------------|
| `RpcRequestOptions` | `notification: bool` (既定 `false`), `timeout: Duration` (既定 5 秒) |
| `RpcResponse::Success` | `result: JsonString` |
| `RpcResponse::Error` | `code: i32`, `message: String`, `data: Option<JsonString>` |

`notification: true` の場合は SDK が id を発番せず、`Ok(None)` を即返す。

## コード例

### sendrecv で接続する

```rust
use sora_sdk::{Role, SoraConnection, SoraConnectionContext};

let context = SoraConnectionContext::new()?;

// AudioTrack はコンテキストから生成する
let audio_source = context.create_audio_source()?;
let audio_track = context.create_audio_track(&audio_source)?;

// VideoTrackSource は shiguredo_webrtc 側で用意する
// (FakeVideoCapturer / AdaptedVideoTrackSource / Mp4VideoCapturer 等)

let (connection, handle) = SoraConnection::builder(
    context,
    vec!["wss://sora.example.com/signaling".to_string()],
    "channel-id".to_string(),
    Role::SendRecv,
)
.sender_audio_track(audio_track)
.on_notify(|text| println!("notify: {text}"))
.on_track(|transceiver| println!("track: {:?}", transceiver.mid()))
.build()?;

tokio::spawn(async move {
    let _ = connection.run().await;
});

// ... 別の場所で disconnect
handle.disconnect().await?;
```

### コーデックバックエンドを明示的に組み立てる

```rust
use sora_sdk::{
    InternalVideoCodecCapability, SoraConnectionContext,
    SoraConnectionContextConfig, VideoCodecCapability, VideoCodecPreference,
};

let mut config = SoraConnectionContextConfig {
    video_codec_preference: VideoCodecPreference::default(),
    video_codec_capabilities: Vec::new(),
    ..Default::default()
};

let cap: Box<dyn VideoCodecCapability> = Box::new(InternalVideoCodecCapability::new());
config
    .video_codec_preference
    .merge(&VideoCodecPreference::new_from_capability(cap.as_ref()));
config.video_codec_capabilities.push(cap);

// nvcodec 等を追加する場合も同じ手順を繰り返す。
let context = SoraConnectionContext::new_with_config(config)?;
```

`new_with_config` は内部で `validate_video_codec_preference` を呼び出すため、preference と capabilities の整合が崩れていると `Error::InvalidVideoCodecPreference` を返す。

### Audio / Video 設定

```rust
use sora_sdk::{Audio, AudioOpusParams, Video, VideoH264Params};

let audio = Audio::new_opus(
    Some(64),
    Some(AudioOpusParams { stereo: Some(true), ..Default::default() }),
);
let video = Video::new_h264(
    Some(2_500),
    Some(VideoH264Params {
        profile_level_id: Some("42e01f".to_string()),
        b_frame: None,
    }),
);

builder = builder.audio(audio).video(video);
```

### DataChannel メッセージング

```rust
let (_conn, handle) = SoraConnection::builder(/* ... */)
    .on_message(|label, data| {
        println!("recv {label}: {} bytes", data.len());
    })
    .build()?;

handle.send_message("#chat", b"hello").await?;
```

### JSON-RPC 2.0 over DataChannel

```rust
use sora_sdk::{JsonString, RpcRequestOptions, RpcResponse};

let params: JsonString = r#"{"key":"value"}"#.parse()?;

// 通常のリクエスト (既定タイムアウト 5 秒)
match handle
    .send_rpc_request("method_name", Some(params), RpcRequestOptions::default())
    .await?
{
    Some(RpcResponse::Success { result }) => { /* result: JsonString */ }
    Some(RpcResponse::Error { code, message, data }) => { /* ... */ }
    None => { /* notification (通常はここに来ない) */ }
}

// notification (レスポンスを待たない)
let _ = handle
    .send_rpc_request(
        "event",
        None,
        RpcRequestOptions { notification: true, ..Default::default() },
    )
    .await?;
// 戻り値は Ok(None)

// タイムアウト変更
let _ = handle
    .send_rpc_request(
        "slow_method",
        None,
        RpcRequestOptions {
            timeout: std::time::Duration::from_secs(10),
            ..Default::default()
        },
    )
    .await?;
```

### 統計情報の取得と切断

```rust
let h = handle.clone();
tokio::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    if let Ok(stats) = h.get_stats().await {
        println!("stats: {stats}");
    }
    let _ = h.disconnect().await;
});
```

### 複数クライアントを 1 コンテキストで実行する

```rust
let context = SoraConnectionContext::new()?;
let urls = vec!["wss://sora.example.com/signaling".to_string()];

for i in 0..5 {
    let ctx = context.clone();
    let urls = urls.clone();
    tokio::spawn(async move {
        let (conn, _h) = SoraConnection::builder(
            ctx,
            urls,
            format!("channel-{i}"),
            sora_sdk::Role::RecvOnly,
        )
        .build()?;
        conn.run().await
    });
}
```

### TLS / TURN-TLS / プロキシ

```rust
use sora_sdk::ProxyInfo;

builder = builder
    .client_cert(client_pem, key_pem)        // PEM
    .ca_cert(ca_pem)                          // PEM
    .turn_tls_ca_cert(turn_ca_der)            // DER
    .proxy(ProxyInfo {
        url: "http://proxy.example.com:3128".to_string(),
        username: Some("user".to_string()),
        password: Some("pass".to_string()),
        user_agent: None,
    });
```

### 複数シグナリング URL のレース

```rust
let urls = vec![
    "wss://sora1.example.com/signaling".to_string(),
    "wss://sora2.example.com/signaling".to_string(),
];

let (conn, handle) = SoraConnection::builder(
    context, urls, "channel-id".into(), Role::SendRecv,
).build()?;

tokio::spawn(async move { let _ = conn.run().await; });

// 接続成立後、勝者の URL を確認できる
if let Some(url) = handle.selected_signaling_url().await? {
    println!("connected via {url}");
}
// リダイレクト後の URL は connected_signaling_url() で取得する
```

## エラー型

`Error` / `Result = std::result::Result<T, Error>`。`anyhow` / `thiserror` は使わない。主な分類:

| カテゴリ | 代表的なバリアント |
|----------|------------------|
| 入力検証 | `InvalidRole`, `HostEmpty`, `HostInvalidFormat`, `UriParse`, `UrlMissingScheme`, `UrlUnsupportedScheme`, `UrlUserinfoNotSupported`, `UrlFragmentNotAllowed`, `UrlMissingHost` |
| プロキシ | `ProxyUrlUnsupportedScheme`, `ProxyUrlUserinfoNotSupported`, `ProxyUrlFragmentNotAllowed`, `ProxyUrlMissingHost`, `ProxyUrlPathNotAllowed`, `ProxyUrlQueryNotAllowed`, `ProxyConnectDecode`, `ProxyConnectEncode`, `ProxyConnectResponseMissing`, `ProxyConnectStatusNotSuccessful`, `ProxyAuth` |
| ネットワーク | `DnsResolve`, `NoResolvedAddress`, `TcpConnectTimeout`, `TcpConnect`, `TlsConfig`, `InvalidServerName`, `TlsConnectTimeout`, `TlsConnect`, `Websocket`, `Io` |
| シグナリング | `SignalingUrlsEmpty`, `AllSignalingUrlsFailed { errors }`, `UnsupportedMessageType`, `JsonParse` |
| WebRTC | `Webrtc`, `PeerConnectionMissing`, `SetRemoteDescriptionTimeout`, `SetRemoteDescriptionResponseMissing`, `SetRemoteDescriptionFailed`, `AnswerTimeout`, `AnswerResponseMissing`, `AnswerFailed`, `SetLocalDescriptionTimeout`, `SetLocalDescriptionResponseMissing`, `SetLocalDescriptionFailed`, `SimulcastVideoSenderMissing`, `SimulcastSetParametersFailed`, `CandidateNotSupported` |
| DataChannel / RPC | `DataChannelMissing`, `DataChannelSendFailed`, `Utf8DecodeFailed`, `RpcTimeout` |
| TLS 証明書 | `TurnTlsCaCert`, `ClientCertParse`, `ClientKeyParse`, `CaCertParse`, `ClientCertKeyIncomplete` |
| コーデック | `InvalidVideoCodecCapability`, `InvalidVideoCodecPreference` |
| 内部コマンド | `CommandSendFailed`, `CommandResponseMissing` |
| バックエンド固有 (feature 付き) | `Libcamera`, `LibcameraMessage`, `Openh264`, `Amf { source }`, `AmfMessage`, `Vpl { source }`, `VplMessage`, `NvCodec { source }`, `NvCodecMessage`, `V4l2 { source }`, `V4l2Message` |

エラーメッセージ (`Display`) は日本語。ログメッセージは英語、というプロジェクト方針と分けて扱うこと。

## 既知の制限事項・注意点

- **コンテキスト生成は重い**: `SoraConnectionContext::new()` は内部スレッドを 3 本起動するため、プロセスあたり 1 つに集約し `Arc` で共有する。
- **`connection.run()` はブロッキング**: 別タスクで実行し、外部制御は `SoraConnectionHandle` (Clone) を介する。
- **コールバックを長時間ブロックしない**: 内部タスクから呼ばれるため、重い処理は自分の async タスクへ転送する。
- **HTTP プロキシは `http://` のみ**: `https://` プロキシ、パス、クエリ、userinfo はサポート外。
- **TLS 設定の単位の違い**: WebSocket TLS の証明書は PEM、TURN-TLS の CA 証明書は DER。
- **ハードウェアコーデックは feature + runtime 両方の条件**: feature 有効化だけでなく GPU / ドライバが揃わないと `*Capability::new()` がエラーを返す。
- **`VideoTrackSource` は本クレートでは作らない**: `shiguredo_webrtc` 側の capturer / source、もしくは本クレートの `Mp4VideoCapturer` / `LibcameraVideoCapturer` から生成する。
- **`#` プレフィックス以外のラベルは触らない**: `send_message` / `on_message` はユーザー定義 DataChannel 専用。SDK 内部用ラベル (`signaling` 等) を渡すと `Error::DataChannelMissing` になる。
- **JSON-RPC の id は SDK が管理する**: 利用側が `id` を組み立てる必要はない。`params` の中身だけ渡す。
- **ロギングは `shiguredo_webrtc` の `rtc_log_*` マクロ**: SDK 内のログは libwebrtc 側 (`rtc_log_info!` / `rtc_log_warning!` / `rtc_log_error!`) に流れる。`log` / `tracing` クレートには依存していない。
- **デフォルトの ADM は Dummy**: マイク入力が必要な場合は `AdmConfig::UseBuiltIn` か `UseExternal` を明示する。
