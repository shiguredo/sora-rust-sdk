# AudioEncoder / AudioDecoder をユーザー側でカスタム可能にする

- Priority: High
- Created: 2026-08-19
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/add-audio-codec-capability
- Polished: {YYYY-MM-DD}

## 目的

ビデオコーデックと対になる形で、音声エンコーダー / デコーダーをユーザー側でカスタムできるようにする。
現在 `SoraConnectionContextConfig` は音声の encoder / decoder factory を builtin に固定しており、独自実装を注入できない。
MP4 音声入力（カスタム AudioEncoder での Opus passthrough）の前提となる。

## 現状

- `SoraConnectionContext::new_with_config` は `AudioEncoderFactory::builtin()` / `AudioDecoderFactory::builtin()` を `PeerConnectionFactoryDependencies` に直接設定している
- shiguredo_webrtc は audio の encoder / decoder factory を `builtin()` のみ公開しており、ユーザー実装を注入する API が無い
- ビデオ側には `VideoCodecCapability`（`src/video_codec_capability.rs`）/ `VideoCodecPreference`（`src/video_codec_preference.rs`）/ `SoraVideoEncoderFactory`・`SoraVideoDecoderFactory`（`src/video_codec.rs`）/ `InternalVideoCodecCapability`（`src/video_codecs/internal.rs`）のフレームワークがあり、`video_codec_preference` / `video_codec_capabilities` でカスタムできる

## 設計方針

ビデオのフレームワークと対になる構成にする。

- `shiguredo_webrtc::AudioCodecType` を新設する（ビデオの `VideoCodecType` と同じ位置づけ。SDP 名との相互変換を持つ）
- webrtc-rs に audio decoder 側の API（`AudioDecoderFactory::new_with_handler` / `AudioDecoderFactoryHandler` / `AudioDecoder` / `AudioDecoderHandler`）を追加する。encoder 側に不足があれば同 issue で追加する
- SDK に以下を追加する:
  - `AudioCodecCapability` trait（`src/audio_codec_capability.rs` 新規）
  - `AudioCodecPreference`（`src/audio_codec_preference.rs` 新規。ビデオと同じく direction を持つ）
  - `SoraAudioEncoderFactory` / `SoraAudioDecoderFactory`（`src/audio_codec.rs` 新規）
  - `InternalAudioCodecCapability`（`src/audio_codecs/internal.rs` 新規。builtin への委譲。デフォルト構成）
- `SoraConnectionContextConfig` に `audio_codec_preference` / `audio_codec_capabilities` を追加し、デフォルトは `InternalAudioCodecCapability` のみ（builtin と等価な動作）
- シグナリング側 `sora_sdk::AudioCodecType`（Opus のみ）は、必要に応じて `shiguredo_webrtc::AudioCodecType` と相互変換する（ビデオ側の変換と対になる）

## 完了条件

- `SoraConnectionContextConfig::audio_codec_capabilities` に capability を登録すると、ネゴシエーションされた音声コーデックの encoder / decoder が capability 経由で生成される
- デフォルト構成で builtin と同等の動作になる
- 型・構造・テストがビデオのフレームワークと対になっている

## 変更対象

- webrtc-rs（別リポジトリ）: `AudioCodecType`、audio encoder / decoder の handler / factory API
- `src/audio_codec_capability.rs`（新規）
- `src/audio_codec_preference.rs`（新規）
- `src/audio_codec.rs`（新規）
- `src/audio_codecs/internal.rs`（新規）
- `src/connection_context.rs`（`SoraConnectionContextConfig` の拡張）
- `Cargo.toml` / `Cargo.lock`（shiguredo_webrtc の更新）
- `CHANGES.md`
