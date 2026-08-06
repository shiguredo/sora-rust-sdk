# WebRTC Encoded Transform (フレーム変換) をサポートする

- Priority: High
- Created: 2026-08-06
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/add-webrtc-encoded-transform
- Polished: 2026-08-06

## 目的

RTCRtpSender / RTCRtpReceiver のエンコード済みフレームを、送信時はエンコーダーとパケタイザーの間、受信時はデパケタイザーとデコーダーの間で加工できるようにする。これにより SFrame によるメディア暗号化や、エンコード済みフレームに対する任意の変換処理を SDK 利用者が実装できるようになる。

W3C の WebRTC Encoded Transform 仕様 (Working Draft 2026-06-25) で定義される API のうち、Script Transform に相当する機能を Rust で提供する。SFrame (暗号化方式) 自体は本 issue の対象外とし、フレーム変換の基盤が完成した後に別 issue で対応する。

## 優先度根拠

High。SFrame によるメディア暗号化やエンコード済みフレームの加工は、Sora の暗号化モジュール相当の機能を Rust SDK に持ち込むための基盤であり、外部仕様 (W3C WebRTC Encoded Transform) に基づく正式な機能追加である。ただし、前提となる shiguredo-webrtc への API 追加とリリース、prebuilt の再公開が必要なため、実装には別リポジトリとの連携が必須である。

## 現状

libwebrtc (m150, branch-heads/7871) の C++ API には FrameTransformer 一式が完備している。

- `api/frame_transformer_interface.h`: `TransformableFrameInterface` / `TransformableVideoFrameInterface` / `TransformableAudioFrameInterface` / `TransformedFrameCallback` / `FrameTransformerInterface` / `FrameTransformerHost`
- `api/rtp_sender_interface.h` / `api/rtp_receiver_interface.h`: `FrameTransformerHost` を継承し `SetFrameTransformer` を公開
- `VideoFrameMetadata`: frameId / dependencies / spatialIndex / temporalIndex / codec などのメタデータを保持

一方、shiguredo-webrtc 0.150.3 には FrameTransformer に関するコード・言及が一切なく、C ラッパー (webrtc_c) も Rust API も存在しない。Rust 側のデータ構造も `EncodedImage` の wrap は encoded_data / rtp_timestamp / encoded_width / encoded_height / frame_type / qp の 6 フィールドのみで、ssrc / payload type / メタデータは公開されておらず、`TransformableFrameInterface` 系の wrap はゼロから追加する必要がある。

また、shiguredo-webrtc の prebuilt パスは GitHub Releases から `libwebrtc_c.a` + `bindings.rs` を取得する方式のため、新しい C API を追加しただけでは利用できない。shiguredo/webrtc-rs リポジトリ側の変更とリリース、prebuilt の再公開が前提となる (ローカル検証は `--features source-build` で可能)。

さらに、libwebrtc の FrameTransformer API にはキーフレーム要求の手段が存在しない。W3C 仕様の Script Transform には `generateKeyFrame` / `sendKeyFrameRequest` (§6.2) と `onkeyframerequest` イベント (§6.3 と §6.4) が定義されているが、libwebrtc の `FrameTransformerInterface` には該当 API が無く、実現には libwebrtc 側の拡張が必要になる。本 issue では対象外とし、別 issue で検討する。

## 設計方針

### shiguredo-webrtc (別リポジトリ) への追加

libwebrtc の C++ API を Rust から使えるようにするため、既存の 3 層構成すべてに FrameTransformer を追加する。

- `webrtc/src/webrtc_c/api/` に FrameTransformer の C ヘッダー + `.cc` を追加する
  - `FrameTransformerInterface` を wrap する C オブジェクトと、`Transform` 呼び出しを Rust コールバックへ転送する構造
  - `TransformableFrameInterface` を wrap する C オブジェクトと、`GetData` / `SetData` / メタデータ取得系の C 関数
- `webrtc/src/webrtc_c.h` に追加分の include を加える
- `src/api/` に Rust ラッパーを追加し `api/mod.rs` で export する
  - `FrameTransformerHandler` (Rust トレイト): 既存の `VideoEncoderEncodedImageCallbackHandler` と同様に `&mut self` で呼ばれる `transform` メソッドでフレームを受け取り、`SetData` によるインプレース書き換えで加工する。戻り値でフレームのドロップを表現する (W3C 仕様はプロセッサによるドロップを許容している)
  - `TransformableFrameInterface` の wrap: フレームデータの取得・書き換え、SSRC / payload type / MIME type / capture time / メタデータ (ビデオ) の読み取り
  - `TransformedFrameCallback` の wrap: 変換済みフレームの返却経路
  - `RtpSender::set_frame_transformer` / `RtpReceiver::set_frame_transformer`
- コールバックは既存の「関数ポインタ構造体 (cbs) + `Box<dyn Trait>` の user_data」パターンに従う (video_encoder.rs の `VideoEncoderEncodedImageCallback` と同型)
- 受信側は libwebrtc の `RegisterTransformedFrameSinkCallback(callback, ssrc)` が ssrc ごとに登録されるが、`OnTransformedFrame` には ssrc 引数がない。返却されたフレームは `TransformableFrameInterface::GetSsrc` でどのストリームのフレームかを判定し、ssrc ごとの delegate へ振り分けるルーティングを C ラッパーに持たせる。トラック削除時 (`on_remove_track`) の `UnregisterTransformedFrameSinkCallback(ssrc)` とも整合させる

#### 実行モデル: タスクキューによる別スレッド実行

transform の処理は重くなる可能性がある (例: SFrame 暗号化) ため、libwebrtc の呼び出しスレッドをブロックしない設計にする。呼び出し元は送信側がエンコーダーの処理スレッド、受信側がネットワークスレッド (libwebrtc の delegate 実装が `RTC_DCHECK_RUN_ON` で検証している) である。

libwebrtc の `FrameTransformerInterface::Transform` は同期呼び出しだが、`TransformedFrameCallback::OnTransformedFrame` は任意のスレッドから呼べる (libwebrtc の delegate 実装が内部のタスクキューへ PostTask する構成)。この性質を利用し、shiguredo-webrtc 側に単一ワーカーのタスクキューを内蔵する。

- `Transform` 呼び出しはフレームをタスクキューにポストして即座に return する
- ワーカースレッド上で `FrameTransformerHandler::transform` を同期実行する
- 処理完了後に `OnTransformedFrame` で変換済みフレームを返す
- フレームの順序保証は単一ワーカーの FIFO 実行で実現する (libwebrtc の C++ API には W3C 仕様の counter に相当する並べ替え防止機構が無いため、タスクキューの直列実行で代替する)。受信トラックが複数ある場合に 1 つのタスクキューを共有すると、1 つのトラックの重い変換処理が他トラックのフレーム遅延に波及する (head-of-line blocking) が、ドロップポリシーで吸収する
- バックプレッシャーは持たない。これは W3C 仕様の Stream processing (§2.1.1) がバックプレッシャーを無効化していることと整合する
- タスクキューのサイズ上限と、溢れた場合に最も古いフレームからドロップするポリシーは、shiguredo-webrtc の独自設計として設ける (W3C 仕様は UA が適応の責任を持つとしているが、具体的なドロップ方針は規定していない)。ただしキーフレームはドロップせず、キーフレーム待ちの復旧遅延を避ける。キューの上限はフレーム数で指定する

#### フレームの所有権と制約

- `TransformableFrameInterface` は Passkey により内部実装クラス (TransformableVideoSenderFrame 等) のみ構築可能。Rust 側からフレームを新規生成・注入することはできないため、加工は `SetData` によるインプレース書き換えを基本とする
- `GetData` が返すデータは「次の非 const メソッド呼び出しまで有効」。`SetData` 前に取得したスライスを保持しない実装にする
- フレームは `Transform` からワーカースレッドへ `unique_ptr` で移譲され、処理完了後に libwebrtc 側へ返却する。C ラッパーがこのライフサイクルを管理する
- transform の解除・破棄時は、タスクキューに滞留中のフレームを破棄し、ワーカースレッドを join してから C++ 側のオブジェクトを解放する
- RTP timestamp は `GetTimestamp` が deprecated で、後継の `GetRtpTimestampInfo` (`RtpTimestampWithOffset` / `RtpTimestampWithoutOffset` の variant) を wrap する。deprecated API は wrap しない
- wrap する setter は `SetData` を基本とし、ビデオメタデータの書き換えが必要になった場合は `SetMetadata` (受信側は frameId / dependencies のみ変更可能) の追加を検討する

### sora-rust-sdk への追加

公開 API は最初からきっちりした設計にする。破壊的変更を許容する。

- `SoraConnectionBuilder` に `sender_video_transform` / `receiver_video_transform` を追加する
  - 送信側: `add_sender_media_track` (src/connection.rs の `add_sender_tracks`) で生成した `RtpSender` に `set_frame_transformer` を適用する。`add_track` 直後に設定すれば最初のフレームから適用される。音声トラックには適用しない
  - 受信側: `on_track` イベント (src/connection.rs の `PeerConnectionObserverHandler::on_track`) で受け取った受信ビデオトラックの `RtpReceiver` に、run() のイベントループ内の `SoraEvent::Track` 処理で適用する。適用対象は全ての受信ビデオトラックとし、音声トラックには適用しない。受信トラックが複数ある場合も、ビルダーで渡された 1 つの transform インスタンスを共有して適用する
  - transform の型は shiguredo-webrtc の `FrameTransformerHandler` トレイトを `Box<dyn ... + Send>` で受け取る
- 接続中に transform を変更・解除する動的更新 API は本 issue の対象外とし、SFrame 対応の別 issue で検討する
- フレーム変換 (Script Transform 相当) は SDP のシグナリングには影響しないため、既存のシグナリングフローは変更しない (SFrame 対応時は SDP への影響を別 issue で検討する)

## 変更対象

- shiguredo-webrtc (別リポジトリ)
  - `webrtc/src/webrtc_c/api/` の C ラッパー追加
  - `webrtc/src/webrtc_c.h` の include 追加
  - `src/api/` の Rust ラッパー追加 (`frame_transformer.rs` 等)
  - `api/mod.rs` の export 追加
  - リリースと prebuilt の再公開
- sora-rust-sdk
  - `Cargo.toml` の shiguredo-webrtc バージョン更新
  - `src/connection.rs` への `sender_video_transform` / `receiver_video_transform` の適用処理追加
  - `src/lib.rs` の公開 API 追加
  - `CHANGES.md` の更新

## テスト戦略

- shiguredo-webrtc 側
  - パススルー変換 (フレームを無加工で返す) の transform を設定し、フレームデータの取得・書き換えができることを確認する
  - フレームデータを書き換えた場合、書き換えた内容が送信側・受信側それぞれに反映されることを確認する
  - タスクキュー経由の別スレッド実行で、フレームが順序を保って返ることを確認する
  - タスクキューが溢れた場合に、キーフレームを除いて古いフレームがドロップされることを確認する
- sora-rust-sdk 側
  - パススルー変換の単体テストで、設定 API が正しく shiguredo-webrtc へ渡ることを確認する
  - 実メディアへの反映は e2e-tests で確認する (sendrecv 等の既存テストに transform 設定を追加する)
  - モック・スタブは使わない

## 完了条件

- shiguredo-webrtc に FrameTransformer の C ラッパーと Rust API が追加され、リリースされている
- sora-rust-sdk の `SoraConnectionBuilder` から送受信それぞれの transform を設定できる
- transform 内でエンコード済みフレームのデータを取得・書き換えでき、その結果が実メディアに反映される (e2e-tests で確認)
- transform の処理が libwebrtc の呼び出しスレッドをブロックせず、タスクキューのワーカースレッドで実行される
- `cargo test --workspace` が成功する
