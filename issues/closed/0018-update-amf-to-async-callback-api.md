# AMF エンコーダー/デコーダーを非同期コールバック API に対応させる

Created: 2026-05-04
Completed: 2026-05-04
Model: deepseek-v4-pro 1.0

## 概要

`shiguredo_amf` が非同期コールバック方式に変更され、エンコーダー/デコーダーの入出力が `Vec<u8>` から `Surface`/`Buffer` に変わった。
`sora-rust-sdk` の `amf.rs` をこの新 API に追従させる。

## 背景

`shiguredo_amf` の最新バージョンでは以下の破壊的変更が行われている:

- `Encoder::new(config)` → `Encoder::new(config, callback)` : コンストラクタにコールバッククロージャが必須になった
- `encoder.encode(&[u8], &options)` → `encoder.encode(Surface, &options, user_data)` : 入力が `&[u8]` から `Surface` + `user_data` に変更
- `encoder.next_frame()` 廃止 : 出力はコールバック経由で `EncodedFrame<T>` として受け取る
- `Decoder::new(config)` → `Decoder::new(config, callback)` : 同上
- `decoder.decode(&[u8])` → `decoder.decode(Buffer, user_data)` : 入力が `&[u8]` から `Buffer` + `user_data` に変更
- `decoder.next_frame()` 廃止 : 出力はコールバック経由で `DecodedFrame<T>` として受け取る

## 対応内容

### amf.rs の変更

1. エンコーダーを非同期コールバック方式に移行する (vpl.rs を参考)
   - `EncoderCallbackValue` / `EncoderCallbackState` 型を追加
   - `handle_amf_encode_callback` 関数を追加
   - `AmfVideoEncoder` を `Arc<Mutex<EncoderCallbackState>>` + `Encoder<EncoderCallbackValue>` に変更
   - `encode()` で `alloc_surface()` による Surface 確保と NV12 データ書込み、`encoder.encode(surface, options, user_data)` 呼出し
2. デコーダーを非同期コールバック方式に移行する (vpl.rs を参考)
   - `DecoderCallbackValue` / `DecoderCallbackState` 型を追加
   - `handle_amf_decode_callback` 関数を追加
   - `AmfVideoDecoder` を `Arc<Mutex<DecoderCallbackState>>` + `Decoder<DecoderCallbackValue>` に変更
   - `decode()` で `alloc_buffer()` による Buffer 確保とビットストリーム書込み、`decoder.decode(buffer, user_data)` 呼出し
3. Import に `shiguredo_amf::amf::{Buffer, Plane, Surface}` / `DecodedFrame` / `EncodedFrame` を追加
4. テストは変更不要 (公開 Capability API 不変のため)

### 影響なし

- E2E テスト (`e2e-tests/tests/amf_video_codec.rs`, `e2e-tests/tests/simulcast.rs`)
- sumomo サンプル
- その他 AMF 参照コード

いずれも公開 `AmfVideoCodecCapability` API 経由で AMF を使用しており、内部実装変更の影響を受けない。

## 解決方法

- `AmfVideoEncoder` の `encoder` フィールドを `Encoder<EncoderCallbackValue>` に変更し、`rebuild_encoder()` でコールバッククロージャを `Encoder::new()` に渡すようにした
- `handle_amf_encode_callback()` を追加し、`EncodedFrame<T>` から `Buffer` の生データを取得して `EncodedImage` を構築するようにした
- `encode()` で `encoder.alloc_surface()` により Surface を確保し、`get_plane_at()` で NV12 データを書き込んでから `encoder.encode(surface, options, user_data)` を呼び出すようにした
- `AmfVideoDecoder` の `decoder` フィールドを `Decoder<DecoderCallbackValue>` に変更し、`rebuild_decoder()` でコールバッククロージャを `Decoder::new()` に渡すようにした
- `handle_amf_decode_callback()` を追加し、`DecodedFrame<T>` から `Surface` のプレーンを読み取り `nv12_copy` + `NV12Buffer` で `VideoFrame` を構築するようにした
- `decode()` で `decoder.alloc_buffer()` により Buffer を確保し、エンコードデータをコピーしてから `decoder.decode(buffer, user_data)` を呼び出すようにした
- `nv12_to_i420` による不要な色空間変換を `nv12_copy` + `NV12Buffer` に置き換えた
- `CHANGES.md` の `## develop` に `[CHANGE]` エントリを追記した
