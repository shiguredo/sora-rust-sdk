# VPL デコーダーの非同期コールバック対応

Created: 2026-05-01
Completed: 2026-05-01
Model: DeepSeek-V4-Pro

## 背景

`shiguredo_vpl` (vpl-rs) の `Decoder` が commit `398d271` で非同期コールバック API (`Decoder<T>`) へ変更された。
主な変更点:

- `Decoder` が `Decoder<T>` になり、コンストラクタでコールバック `F: for<'a> FnMut(Result<DecodedFrame<'a, T>, Error>) + Send + 'static` を受け取る
- `Decoder::decode` のシグネチャが `decode(&mut self, data: &[u8], value: T)` に変更
- `Decoder::next_frame` が削除された
- `DecodedFrame` が借用ベース (`y: &'a [u8]`, `uv: &'a [u8]`) に変更された
- `DecoderConfig` に `async_depth` フィールドが追加された

`sora-rust-sdk` の `src/video_codecs/vpl.rs` は古い同期 API を使用しているため、コンパイルエラーになる。

## 修正内容

### vpl.rs の VplVideoDecoder を非同期コールバック方式に移行

- `DecoderCallbackValue` 型を追加し、`DecodedFrame<T>` で value を受け渡す
- `DecoderCallbackState` 型を `Arc<Mutex<...>>` で共有し、V4L2 と同じパターンにする
- `handle_vpl_decode_callback` 関数を追加し、コールバック内で NV12 から `VideoFrame` を構築して libwebrtc の callback を呼び出す
- `VplVideoDecoder` の `decoder` フィールドを `Decoder` から `Decoder<DecoderCallbackValue>` に変更
- `callback` フィールドを `Arc<Mutex<DecoderCallbackState>>` 経由の共有状態に変更
- `ensure_decoder` を削除し、`rebuild_decoder` を追加
- `configure` で `rebuild_decoder` を呼ぶように変更
- `decode` で同期ポーリング (`next_frame`) を廃止し、`decoder.decode(data, value)` を呼び即座に返すように変更
- `register_decode_complete_callback` と `release` で `Arc<Mutex<...>>` を経由するように変更

### NV12 色空間変換の廃止

- 旧コードでは NV12 → I420 に変換していたが、NV12 のまま `NV12Buffer` を構築し `cast_to_video_frame_buffer()` で `VideoFrame` を生成する方式に変更
- `nv12_to_i420` import を `nv12_copy` import に差し替え
- `I420Buffer` import を `NV12Buffer` import に差し替え

## 解決方法

1. `V4l2VideoDecoder` (src/video_codecs/v4l2.rs) の実装パターンを参考に、`VplVideoDecoder` を非同期コールバック方式へ移行した
2. `NV12Buffer` + `nv12_copy` で NV12→NV12 コピーのみ行い、I420 への色空間変換を廃止した
3. ビルドと clippy が通ること、既存の VPL 単体テスト 10 件が全て通過することを確認した
