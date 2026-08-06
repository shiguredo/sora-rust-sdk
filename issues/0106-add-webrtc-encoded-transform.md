# WebRTC Encoded Transform (フレーム変換) をサポートする

- Priority: High
- Created: 2026-08-06
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/add-webrtc-encoded-transform
- Polished: {YYYY-MM-DD}

## 目的

RTCRtpSender / RTCRtpReceiver のエンコード済みフレームを、送信時はエンコーダーとパケタイザーの間、受信時はデパケタイザーとデコーダーの間で加工できるようにする。これにより SFrame によるメディア暗号化や、エンコード済みフレームに対する任意の変換処理を SDK 利用者が実装できるようになる。

W3C の WebRTC Encoded Transform 仕様 (Working Draft 2026-06-25) で定義される API のうち、Script Transform (フレームを加工する関数を差し込む方式) に相当する機能を Rust で提供する。SFrame (暗号化方式) 自体は本 issue の対象外とし、フレーム変換の基盤が完成した後に別 issue で対応する。

## 現状

libwebrtc (m150, branch-heads/7871) の C++ API には FrameTransformer 一式が完備している。

- `api/frame_transformer_interface.h`: `TransformableFrameInterface` / `TransformableVideoFrameInterface` / `TransformableAudioFrameInterface` / `TransformedFrameCallback` / `FrameTransformerInterface` / `FrameTransformerHost`
- `api/rtp_sender_interface.h` / `api/rtp_receiver_interface.h`: `FrameTransformerHost` を継承し `SetFrameTransformer` を公開
- `VideoFrameMetadata`: frameId / dependencies / spatialIndex / temporalIndex / codec などのメタデータを保持

しかし shiguredo-webrtc 0.150.3 には FrameTransformer に関するコード・言及が一切なく、C ラッパー (webrtc_c) も Rust API も存在しない。Rust 側のデータ構造も `EncodedImage` の wrap は限定的で、`TransformableFrameInterface` 系の wrap はゼロから追加する必要がある。

また、shiguredo-webrtc の prebuilt パスは GitHub Releases から `libwebrtc_c.a` + `bindings.rs` を取得する方式のため、新しい C API を追加しただけでは利用できない。shiguredo/webrtc-rs リポジトリ側の変更とリリース、prebuilt の再公開が前提となる (ローカル検証は `--features source-build` で可能)。

## 設計方針

### shiguredo-webrtc (別リポジトリ) への追加

libwebrtc の C++ API を Rust から使えるようにするため、既存の 3 層構成すべてに FrameTransformer を追加する。

- `webrtc/src/webrtc_c/api/` に FrameTransformer の C ヘッダー + `.cc` を追加する
  - `FrameTransformerInterface` を wrap する C オブジェクト (`webrtc_FrameTransformerInterface` 相当) と、`Transform` 呼び出しを Rust コールバックへ転送する構造
  - `TransformableFrameInterface` を wrap する C オブジェクトと、`GetData` / `SetData` / メタデータ取得系の C 関数
- `webrtc/src/webrtc_c.h` に追加分の include を加える
- `src/api/` に Rust ラッパーを追加し `api/mod.rs` で export する
  - `FrameTransformerHandler` (Rust トレイト): `transform` メソッドでフレームを受け取り、加工後のフレームを返す
  - `TransformableFrameInterface` の wrap: フレームデータの取得・書き換え、SSRC / payload type / MIME type / capture time / メタデータ (ビデオ) の読み取り
  - `TransformedFrameCallback` の wrap: 変換済みフレームの返却経路
  - `RtpSender::set_frame_transformer` / `RtpReceiver::set_frame_transformer`
- コールバックは既存の「関数ポインタ構造体 (cbs) + `Box<dyn Trait>` の user_data」パターンに従う (video_encoder.rs の `VideoEncoderEncodedImageCallback` と同型)
- バックプレッシャーは持たない。libwebrtc 側の委譲実装 (`RTPSenderVideoFrameTransformerDelegate` 等) に任せ、フレームの順序保証・ドロップの判断も libwebrtc の仕様に従う

### sora-rust-sdk への追加

- shiguredo-webrtc の更新を受け、SDK の公開 API にフレーム変換の設定経路を追加する
- 例: `SoraConnection` または接続設定に、送受信それぞれの transform を指定できるフィールドを追加する
- フレーム変換は SDP のシグナリングには影響しないため、既存のシグナリングフローは変更しない

## 変更対象

- shiguredo-webrtc (別リポジトリ)
  - `webrtc/src/webrtc_c/api/` の C ラッパー追加
  - `webrtc/src/webrtc_c.h` の include 追加
  - `src/api/` の Rust ラッパー追加 (`frame_transformer.rs` 等)
  - `api/mod.rs` の export 追加
  - リリースと prebuilt の再公開
- sora-rust-sdk
  - `Cargo.toml` の shiguredo-webrtc バージョン更新
  - `src/connection.rs` (または接続設定型) への transform 設定 API 追加
  - `src/lib.rs` の公開 API 追加

## テスト戦略

- shiguredo-webrtc 側
  - パススルー変換 (フレームを無加工で返す) の transform を設定し、フレームデータの取得・書き換えができることを確認する
  - フレームデータを書き換えた場合、書き換えた内容が送信 (または受信) 側に反映されることを確認する
- sora-rust-sdk 側
  - パススルー変換の単体テストで、設定 API が正しく shiguredo-webrtc へ渡ることを確認する
  - モック・スタブは使わない

## 完了条件

- shiguredo-webrtc に FrameTransformer の C ラッパーと Rust API が追加され、リリースされている
- sora-rust-sdk から RtpSender / RtpReceiver (または SoraConnection の設定経由) で transform を設定できる
- transform 内でエンコード済みフレームのデータを取得・書き換えでき、その結果が実メディアに反映される
- `cargo test --workspace` が成功する
