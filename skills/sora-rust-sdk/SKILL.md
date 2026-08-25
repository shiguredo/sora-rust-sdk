---
name: sora-rust-sdk
description: 時雨堂の WebRTC SFU Sora 向け Rust クライアント SDK sora_sdk の機能・API リファレンス。SoraConnectionContext / SoraConnection / SoraConnectionHandle による接続管理、Audio / Video 設定、VideoCodecPreference / VideoCodecCapability によるコーデック選択、MP4 無変換送信 (Mp4SampleReader / Mp4VideoCapturer)、DataChannel メッセージング、JSON-RPC 2.0 over DataChannel、TLS / TURN-TLS / HTTP プロキシ設定、複数クライアント同時実行に関する質問時に使用。
---

# sora_sdk

WebRTC SFU Sora のクライアントを Rust で実装するための SDK。シグナリング、メディア送受信、DataChannel メッセージング、JSON-RPC、コーデックバックエンド統合を提供する。

## 特徴

- **Sora シグナリング**: WebSocket / DataChannel シグナリング両対応。複数シグナリング URL のレース接続、リダイレクト対応。
- **複数ロール**: `sendonly` / `recvonly` / `sendrecv` をサポート。
- **メディア機能**: マルチストリーム、サイマルキャスト、スポットライト、転送フィルター、シグナリング通知。
- **コーデック**: VP8 / VP9 / AV1 / H.264 / H.265。OpenH264 / Apple VideoToolbox / AMD AMF / NVIDIA Video Codec / Intel VPL / V4L2-M2M のバックエンド統合。
- **MP4 無変換送信**: `Mp4PassthroughVideoCodecCapability` で MP4 ファイルの映像トラックをデコード / エンコードを挟まずに Sora へ送信し、音声トラックは無視する。capability は `Mp4SampleReader::passthrough_capability()` から生成する。
- **DataChannel メッセージング**: `#` プレフィックスのユーザー定義 DataChannel でバイナリ送受信。
- **JSON-RPC 2.0 over DataChannel**: SDK が id 採番とエンベロープを担当。
- **TURN-TLS**: 独自 CA 証明書 (DER) による検証。
- **HTTP プロキシ (CONNECT)**: `http://` のみ対応。Basic 認証可。
- **複数クライアント同時実行**: 1 つの `SoraConnectionContext` を `Arc` で共有して N 接続。

## バージョン情報

- crate 名: `sora_sdk`
- バージョン: 2026.1.0
- Rust Edition: 2024
- 最小 Rust バージョン: 1.93
- ライセンス: Apache-2.0
- 対応 Sora: 2025.2.0 以降
- 対応プラットフォーム: Ubuntu 22.04 / 24.04 / 26.04 (x86_64, arm64), macOS 15 / 26 (arm64), Windows 11 / Server 2025 (x86_64), Raspberry Pi (Linux, arm64)

`shiguredo_webrtc` クレートが提供する `AudioTrack` / `VideoTrack` / `VideoTrackSource` / `RtpTransceiver` / `RtpReceiver` / `IceServer` 等を直接受け取る公開 API があるため、利用側の `Cargo.toml` に `shiguredo_webrtc` を追加する必要がある。

## Cargo features

| feature | 既定 | 内容 |
|---------|------|------|
| `openh264` | 有効 | OpenH264 による H.264 ソフトウェアエンコード / デコード |
| `amf` | 無効 | AMD AMF によるハードウェアエンコード / デコード (Windows / Linux) |
| `nvcodec` | 無効 | NVIDIA Video Codec によるハードウェアエンコード / デコード (Windows / Linux) |
| `vpl` | 無効 | Intel VPL によるハードウェアエンコード / デコード (Linux) |
| `v4l2` | 無効 | V4L2-M2M によるハードウェアエンコード / デコード (Raspberry Pi) |
| `libcamera` | 無効 | libcamera による映像入力 (Raspberry Pi) |

機能フラグは加算式。複数バックエンドを同時に有効化可能。フラグを有効にしても GPU / ドライバが無ければ各 `*::new()` がランタイムでエラーを返す。`vpl` は Linux 専用で、他の OS ではフラグを有効にしてもモジュールごとコンパイルされない。

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

`SoraConnection::builder(context, signaling_urls, channel_id, role, event_handler) -> SoraConnectionBuilder` で開始。`signaling_urls` は `Vec<String>`、`channel_id` は `String`、`role` は `Role`、`event_handler` は `SoraConnectionEventHandler` トレイトを実装した任意の型のインスタンス（`impl SoraConnectionEventHandler + 'static`）。WebSocket TLS の設定（`insecure` / `ca_cert` / `client_cert`）は Builder の各メソッド経由で行う。

#### イベントハンドラ (`SoraConnectionEventHandler` トレイト)

イベント通知は Builder のチェーンメソッドではなく、`SoraConnectionEventHandler` トレイトを実装したユーザー定義型で受け取る。実装型のインスタンスを `SoraConnection::builder(...)` の第 5 引数に渡す。

- トレイトは `Send`（`Sync` は不要。各コールバックは単一タスクから直列に呼ばれるため）
- 全メソッドにデフォルト空実装が用意されているため、必要なメソッドだけオーバーライドすればよい
- コールバックは内部タスクから直列に呼ばれるためブロックさせないこと。重い処理はチャンネルで自分のタスクへ転送する
- ユーザー定義 struct に状態を持たせ `&mut self` で共有できる

| トレイトメソッド | シグネチャ | 説明 |
|----------------|-----------|------|
| `on_signaling_message` | `fn on_signaling_message(&mut self, signaling_type: SignalingType, direction: SignalingDirection, text: &str)` | シグナリングメッセージ送受信時 |
| `on_notify` | `fn on_notify(&mut self, text: &str)` | notify メッセージ受信時 |
| `on_push` | `fn on_push(&mut self, text: &str)` | push メッセージ受信時 |
| `on_track` | `fn on_track(&mut self, transceiver: RtpTransceiver)` | トラック追加時 |
| `on_remove_track` | `fn on_remove_track(&mut self, receiver: RtpReceiver)` | トラック削除時 |
| `on_switched` | `fn on_switched(&mut self)` | DataChannel シグナリングに切り替わった時 |
| `on_websocket_close` | `fn on_websocket_close(&mut self, code: Option<u16>, reason: &str)` | WebSocket 切断時 |
| `on_message` | `fn on_message(&mut self, label: &str, data: &[u8])` | `#` プレフィックス DataChannel 受信時 |
| `on_data_channel` | `fn on_data_channel(&mut self, label: &str)` | DataChannel 作成時 |
| `on_data_channel_open` | `fn on_data_channel_open(&mut self, label: &str)` | DataChannel オープン時 |
| `on_data_channel_message` | `fn on_data_channel_message(&mut self, label: &str, data: &[u8])` | DataChannel メッセージ受信時 |
| `on_data_channel_close` | `fn on_data_channel_close(&mut self, label: &str)` | DataChannel クローズ時 |

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
| `disconnect()` | `Result<()>` | Sora へ disconnect メッセージの送信を試みてから切断を開始する |
| `send_message(label, data)` | `Result<()>` | SDK 内部用ラベル (`signaling`、`stats`、`push`、`notify`、`rpc`) および `#` プレフィックスのないラベル、Offer 応答の `data_channels` に含まれていないラベルを渡すと `Error::InvalidDataChannelLabel` を返す |
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
| `Audio` | `Bool(bool)`, `Opus { bit_rate: Option<u32>, params: Option<AudioOpusParams> }` |
| `AudioCodecType` | `Opus` |
| `AudioOpusParams` | `channels`, `maxplaybackrate`, `minptime`, `ptime`, `stereo`, `sprop_stereo`, `useinbandfec`, `usedtx` (すべて `Option`) |
| `Video` | `Bool(bool)`, `Vp8 { bit_rate: Option<u32> }`, `Vp9 { bit_rate: Option<u32>, params: Option<VideoVP9Params> }`, `H264 { bit_rate: Option<u32>, params: Option<VideoH264Params> }`, `H265 { bit_rate: Option<u32>, params: Option<VideoH265Params> }`, `Av1 { bit_rate: Option<u32>, params: Option<VideoAV1Params> }` |
| `VideoCodecType` | `Vp8`, `Vp9`, `H264`, `H265`, `Av1` |
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
| `ParsedProxyInfo` | `ParsedProxyInfo::parse(&ProxyInfo)` で検証済みのプロキシ接続情報を取得する公開型。全フィールドは非公開で accessor (`host()` / `port()` / `username()` / `password()` / `user_agent()`) 経由で取得する。主に PBT 用途で公開されている |
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
| `validate_video_codec_preference(&preference, &[Box<dyn VideoCodecCapability>])` | `new_with_config` 内部でも呼ばれる整合性チェック。可否判定は各 capability の `is_supported` の結果を正とする。preference と capabilities が一致しない場合 `Error::InvalidVideoCodecPreference` |
| `SoraVideoEncoderFactory` / `SoraVideoDecoderFactory` | 内部で利用される factory (通常はユーザーが直接触らない) |
| `AlignmentEncoderAdapter` | エンコーダーのアライメント補正アダプター |
| `SimulcastCapabilityHelper` | `new(primary_factory)` / `new_with_builder(...)` で生成するサイマルキャスト対応ヘルパー。`get_supported_formats()` / `create_video_encoder(...)` を提供 |
| `codec_type_from_format(&SdpVideoFormatRef)` | フォーマットから `VideoCodecType` を解決 |

### 標準のコーデックバックエンド

| 型 | feature / 条件 | 生成方法 | 用途 |
|----|---------------|----------|------|
| `InternalVideoCodecCapability` | 常時 | `new() -> Self` | libwebrtc 内蔵 (VP8 / VP9 / AV1 など) |
| `InternalAppleVideoCodecCapability` | macOS / iOS | `new() -> Option<Self>` | VideoToolbox による H.264 / H.265 |
| `Mp4PassthroughVideoCodecCapability` | 常時 | `Mp4SampleReader::passthrough_capability() -> Self` | MP4 ファイル無変換送信 (Encoder 方向のみ、デコーダーは提供しない) |
| `Openh264VideoCodecCapability` | `openh264` | `new(path) -> Result<Self>` | OpenH264 ソフトウェア H.264 |
| `AmfVideoCodecCapability` | `amf` | `new() -> Result<Self>` | AMD AMF |
| `NvCodecVideoCodecCapability` | `nvcodec` | `new() -> Result<Self>` / `new_with_device_id(i32) -> Result<Self>` | NVIDIA Video Codec |
| `VplVideoCodecCapability` | `vpl` (Linux のみ) | `new() -> Result<Self>` | Intel VPL (Linux 専用) |
| `V4l2VideoCodecCapability` | `v4l2` | `new() -> Result<Self>` | V4L2-M2M (Raspberry Pi) |

新しい capability を加えるたびに、対応する `VideoCodecPreference` を `merge` して preference 側にも追加すること。`SoraConnectionContextConfig::default()` は Internal と (macOS/iOS 上では) InternalApple を自動登録する。

## MP4 / libcamera

| 型 | feature / 条件 | 説明 |
|----|---------------|------|
| `Mp4SampleReader` | 常時 | MP4 ファイルからサンプルを取得。`new<P: AsRef<Path>>(path)` で構築 (ファイルベース読み込みで全体をメモリに保持しない)。`len()` / `is_empty()` / `codec_type()` / `passthrough_capability()` を提供 |
| `Mp4PassthroughVideoCodecCapability` | 常時 | パススルー用 capability。`Mp4SampleReader::passthrough_capability()` からのみ生成できる |
| `Mp4VideoCapturer` | 常時 | `Mp4VideoCapturer::new(Mp4SampleReader)` で構築し `video_source()` で `VideoTrackSource` を取得。末尾に達すると先頭に戻ってループ再生する |
| `Mp4Error` | 常時 | MP4 関連のエラー enum (`Io`, `Demux`, `NoVideoTrack`, `NoVideoSamples`, `UnsupportedVideoCodec`, `InvalidNalLengthSize`, `InputPositionOutOfRange`, `InconsistentSampleTable`, `UnsupportedCompositionTimeOffset`, `InconsistentSampleDescription`)。`Error::Mp4 { source }` に包まれて返る |
| `Error::Mp4 { source: Mp4Error }` | 常時 | `Mp4Error` を source として保持する SDK 共通エラー |
| `LibcameraVideoCapturer` | `libcamera` | libcamera 経由の映像入力 |
| `LibcameraVideoCapturerBuilder` | `libcamera` | 上記のビルダー |
| `LibcameraNativeFrameBuffer` | `libcamera` | libcamera のフレームバッファ |

MP4 入力は映像トラックだけを送信し、音声トラックは無視する。
MP4 パススルーの入力制約 (いずれも `Mp4SampleReader::new` がエラーを返す):

- 対応映像コーデックは H.264 / H.265 / VP8 / VP9 / AV1
- 非ゼロの composition time offset (B フレーム) を含む MP4 は拒否 (`Mp4Error::UnsupportedCompositionTimeOffset`)
- 途中でサンプルエントリー (コーデック・解像度など) が切り替わる MP4 は拒否 (`Mp4Error::InconsistentSampleDescription`)

```rust
use sora_sdk::{Mp4SampleReader, Mp4VideoCapturer};

let reader = Mp4SampleReader::new("input.mp4")?;

// capability は reader から生成して context config に登録する
let capability = reader.passthrough_capability();
// (VideoCodecPreference への merge と video_codec_capabilities への push は
//  「コーデックバックエンドを明示的に組み立てる」と同じ手順)

let capturer = Mp4VideoCapturer::new(reader)?;
let video_source = capturer.video_source();
let video_track = context.create_video_track(&video_source)?;
```

`LibcameraVideoCapturer::builder()` は `camera_index()` / `width()` / `height()` / `native_frame_output()` / `control()` / `controls()` / `build()` を提供する。生成したキャプチャラーは `start()` / `stop()` / `video_source()` で制御する。

`LibcameraNativeFrameBuffer` は `fd()` / `size()` / `stride()` / `raw_width()` / `raw_height()` / `scaled_width()` / `scaled_height()` / `is_i420()` / `is_nv12()` を提供する。

## DataChannel メッセージング / JSON-RPC 2.0

| 動作 | API |
|------|-----|
| ユーザー定義 DataChannel をシグナリング時に宣言 | `SoraConnectionBuilder::data_channels(Vec<ConnectDataChannel>)` |
| 受信ハンドラ | `SoraConnectionEventHandler::on_message(&mut self, label: &str, data: &[u8])` (ラベルは `#` 始まり。トレイトメソッドなのでユーザー定義 struct に実装する) |
| 送信 | `SoraConnectionHandle::send_message(label, data)` |

JSON-RPC 2.0 は SDK 側で `{ "jsonrpc": "2.0", "method": ..., "params": ..., "id": ... }` を組み立てる。利用側は内側の `params` を `Option<JsonString>` で渡すだけ (パラメータ不要なら `None`)。

| 型 | フィールド / バリアント |
|----|----------------------|
| `RpcRequestOptions` | `notification: bool` (既定 `false`), `timeout: Duration` (既定 5 秒) |
| `RpcResponse::Success` | `result: JsonString` |
| `RpcResponse::Error` | `code: i32`, `message: String`, `data: Option<JsonString>` |

`notification: true` の場合は SDK が id を発番せず、`Ok(None)` を即返す。
応答は JSON-RPC 2.0 の Response Object として検証する。
応答待機中の Request ID と対応付けられる不正応答は `Error::RpcProtocolViolation { id: Some(...) }` を返す。
対応付けられない不正応答や未知の id を持つ応答はメッセージ単位で破棄し、接続を継続する。

## コード例

### sendrecv で接続する

```rust
use shiguredo_webrtc::RtpTransceiver;
use sora_sdk::{Role, SoraConnection, SoraConnectionContext, SoraConnectionEventHandler};

struct MyEventHandler;

impl SoraConnectionEventHandler for MyEventHandler {
    fn on_notify(&mut self, text: &str) {
        println!("notify: {text}");
    }
    fn on_track(&mut self, transceiver: RtpTransceiver) {
        println!("track: {:?}", transceiver.receiver().track().id());
    }
}

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
    MyEventHandler,
)
.sender_audio_track(audio_track)
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
use sora_sdk::SoraConnectionEventHandler;

struct MyEventHandler;

impl SoraConnectionEventHandler for MyEventHandler {
    fn on_message(&mut self, label: &str, data: &[u8]) {
        println!("recv {label}: {} bytes", data.len());
    }
}

// SoraConnection::builder(context, urls, channel_id, role, MyEventHandler)
let (_conn, handle) = SoraConnection::builder(/* ... 5 引数、末尾に MyEventHandler ... */)
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
use sora_sdk::SoraConnectionEventHandler;

struct MyEventHandler;
impl SoraConnectionEventHandler for MyEventHandler {}

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
            MyEventHandler,
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
use sora_sdk::SoraConnectionEventHandler;

struct MyEventHandler;
impl SoraConnectionEventHandler for MyEventHandler {}

let urls = vec![
    "wss://sora1.example.com/signaling".to_string(),
    "wss://sora2.example.com/signaling".to_string(),
];

let (conn, handle) = SoraConnection::builder(
    context, urls, "channel-id".into(), Role::SendRecv, MyEventHandler,
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
| プロキシ | `ProxyUrlUnsupportedScheme`, `ProxyUrlUserinfoNotSupported`, `ProxyUrlFragmentNotAllowed`, `ProxyUrlMissingHost`, `ProxyUrlPathNotAllowed`, `ProxyUrlQueryNotAllowed`, `ProxyConnectDecode`, `ProxyConnectEncode`, `ProxyConnectResponseMissing`, `ProxyConnectStatusNotSuccessful`, `ProxyConnectTimeout`, `ProxyAuth` |
| ネットワーク | `DnsResolve`, `NoResolvedAddress`, `TcpConnectTimeout`, `TcpConnect`, `TlsConfig`, `InvalidServerName`, `TlsConnectTimeout`, `TlsConnect`, `Websocket`, `Io`, `ProxyConnectUnexpectedTrailingData` |
| シグナリング | `SignalingUrlsEmpty`, `AllSignalingUrlsFailed { errors }`, `UnsupportedMessageType`, `JsonParse` |
| WebRTC | `Webrtc`, `SetRemoteDescriptionTimeout`, `SetRemoteDescriptionResponseMissing`, `SetRemoteDescriptionFailed`, `AnswerTimeout`, `AnswerResponseMissing`, `AnswerFailed`, `SetLocalDescriptionTimeout`, `SetLocalDescriptionResponseMissing`, `SetLocalDescriptionFailed`, `SimulcastVideoSenderMissing`, `SimulcastSetParametersFailed`, `CandidateNotSupported` |
| DataChannel / RPC | `DataChannelMissing`, `DataChannelSendFailed`, `Utf8DecodeFailed`, `RpcTimeout`, `RpcProtocolViolation { id }`, `InvalidDataChannelLabel`, `Redirected` |
| TLS 証明書 | `TurnTlsCaCert`, `ClientCertParse`, `ClientKeyParse`, `CaCertParse`, `ClientCertKeyIncomplete` |
| コーデック | `InvalidVideoCodecCapability`, `InvalidVideoCodecPreference` |
| 内部コマンド | `CommandSendFailed`, `CommandResponseMissing`, `CommandTimeout` |
| バックエンド固有 (feature 付き) | `Libcamera`, `LibcameraMessage`, `UnknownLibcameraControl`, `Openh264`, `Amf { source }`, `AmfMessage`, `Vpl { source }`, `VplMessage` (後者 2 つは Linux のみ), `NvCodec { source }`, `NvCodecMessage`, `V4l2 { source }`, `V4l2Message` |
| その他 | `Mp4 { source: Mp4Error }`, `InvalidSystemTime { source }` |

エラーメッセージ (`Display`) は日本語。ログメッセージは英語、というプロジェクト方針と分けて扱うこと。

## 既知の制限事項・注意点

- **コンテキスト生成は重い**: `SoraConnectionContext::new()` は内部スレッドを 3 本起動するため、プロセスあたり 1 つに集約し `Arc` で共有する。
- **`connection.run()` はブロッキング**: 別タスクで実行し、外部制御は `SoraConnectionHandle` (Clone) を介する。
- **コールバックを長時間ブロックしない**: 内部タスクから呼ばれるため、重い処理は自分の async タスクへ転送する。
- **HTTP プロキシは `http://` のみ**: `https://` プロキシ、パス、クエリ、userinfo はサポート外。
- **TLS 設定の単位の違い**: WebSocket TLS の証明書は PEM、TURN-TLS の CA 証明書は DER。
- **ハードウェアコーデックは feature + runtime 両方の条件**: feature 有効化だけでなく GPU / ドライバが揃わないと `*Capability::new()` がエラーを返す。
- **VPL は Linux 専用**: `vpl` feature は Linux 以外の OS ではコンパイルされず、`VplVideoCodecCapability` と `Error::Vpl` 系のバリアントも Linux 限定。
- **`VideoTrackSource` は本クレートでは作らない**: `shiguredo_webrtc` 側の capturer / source、もしくは本クレートの `Mp4VideoCapturer` / `LibcameraVideoCapturer` から生成する。
- **MP4 パススルーの入力制約**: B フレーム (非ゼロ composition time offset) を含む MP4 と、途中でサンプルエントリー (コーデック・解像度など) が切り替わる MP4 は `Mp4SampleReader::new()` が拒否する。
- **`send_message` のラベル制約**: SDK 内部用ラベル（`signaling`、`stats`、`push`、`notify`、`rpc`）および `#` プレフィックスのないラベル、Offer 応答の `data_channels` に含まれていないラベルを渡すと `Error::InvalidDataChannelLabel` を返す。`on_message` は `#` プレフィックスのユーザー定義 DataChannel 専用。
- **JSON-RPC の id は SDK が管理する**: 利用側が `id` を組み立てる必要はない。`params` の中身だけ渡す。
- **DataChannel の展開後サイズ上限**: `compress: true` の DataChannel メッセージは zlib 展開後 16 MiB まで。
  上限超過や不正な zlib ストリームはメッセージ単位で破棄し、接続を継続する。
- **DataChannel シグナリングの切替条件**: WebSocket で `switched` を受信し、Offer の `data_channels` に含まれる全 DataChannel が Open になってから切り替える。
- **DataChannel シグナリング中の Close**: `signaling` ラベルで Sora から `{"type": "close"}` を受信すると接続を終了する。
  他のラベルで受信した Close は接続終了として扱わない。
- **MP4 入力は映像専用**: 音声トラックは無視する。
  B フレームなどの非ゼロ composition time offset を含む映像は受理しない。
- **ロギングは `shiguredo_webrtc` の `rtc_log_*` マクロ**: SDK 内のログは libwebrtc 側 (`rtc_log_verbose!` / `rtc_log_info!` / `rtc_log_warning!` / `rtc_log_error!`) に流れる。`log` / `tracing` クレートには依存していない。
- **デフォルトの ADM は Dummy**: マイク入力が必要な場合は `AdmConfig::UseBuiltIn` か `UseExternal` を明示する。
