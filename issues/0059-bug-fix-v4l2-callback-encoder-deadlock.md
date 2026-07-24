# V4L2 の encode / convert コールバックで shared_state ロック保持中に `encoder.encode` を呼ぶデッドロック経路を解消する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-v4l2-callback-encoder-deadlock
- Polished: {YYYY-MM-DD}

## 目的

V4L2 バックエンドの encode / convert コールバックが `shared_state.lock()` を保持したまま `encoder.encode()` を呼んでいる経路がある。`encoder.encode()` が同期的にドレイン処理でエンコードコールバックを発火するケースがあると、`std::sync::Mutex` の再入で完全デッドロックになるため、AMF / NVCODEC / VPL と同じく「lock 内では callback ポインタだけを取り出し、encoder 呼び出しはロック外で行う」設計に揃える。

## 優先度根拠

High。デッドロックは接続を止め、`SoraConnection` タスク全体を hang させる。libwebrtc / V4L2 のスケジューリング次第で顕在化するため、実運用中に一度発生すると再現条件の追跡が困難。V4L2 の `release` 側 (867 行付近) はすでに「take() してロック外で drop」する形で同種の問題を回避しており、encode パスだけ設計が非対称。

## 現状

`src/video_codecs/v4l2.rs:283-306` で convert コールバックが shared_state を lock したまま `encoder.encode()` を呼ぶ:

```rust
let mut shared_state = shared_state.lock().unwrap();
let Some(encoder) = shared_state.encoder.as_mut() else {
    rtc_log_warning!("V4L2 convert callback dropped frame because encoder is not initialized");
    return;
};
match encoder.encode(
    EncodeInput::DmaBuf { fd: dmabuf_fd, bytesused, length },
    timestamp_us,
    value.force_keyframe,
    callback_value,
) { ... }
```

`src/video_codecs/v4l2.rs:621-638` の MMAP パスも同構造:

```rust
let mut shared_state = self.shared_state.lock().unwrap();
let Some(encoder) = shared_state.encoder.as_mut() else { ... };
match encoder.encode(
    EncodeInput::Mmap(&mut fill),
    timestamp_us,
    force_keyframe,
    callback_value,
) { ... }
```

一方 `handle_v4l2_encode_callback` は同じ `shared_state` を lock して callback を取り出す。`encoder.encode()` が同期ドレインで encode callback を発火すると、同じ mutex を再入して取得しようとしデッドロックする。

## 設計方針

`V4l2VideoEncoder` の共有状態から `encoder` を分離し、`encoder` は `V4l2VideoEncoder` 側の独立フィールドに持つ (AMF/NVCODEC/VPL と同形状)。`shared_state` は「エンコードコールバックポインタ + resolution 等の callback 側から必要な情報」だけに絞る。

具体的な段取り:

1. `EncoderSharedState` から `encoder` フィールドを外し、`V4l2VideoEncoder` の独立フィールドに移す。
2. convert callback (`handle_v4l2_convert_callback`) と MMAP encode パス (`V4l2VideoEncoder::encode`) の両方で、`shared_state.lock()` は callback ポインタ (Copy 型) の取り出しに限定し、直後にロックを解放してから `encoder.encode()` を呼ぶ。
3. `handle_v4l2_encode_callback` (エンコード完了コールバック) も同様に、ロック内では callback を取り出すだけ、ロック外で `callback.on_encoded_image(...)` を呼ぶ。
4. release 側の take() パターンは維持する。

## 完了条件

- V4L2 バックエンドの encode / convert / encode-callback / decode-callback パスで、`shared_state` を lock したまま外部 (`encoder.encode` / `on_encoded_image` 等) を呼ぶ経路が存在しない。
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace` が通る。
- 再入経路のデッドロックが起きないことを、単体で検証するテストがあるか、少なくともコードコメントで「lock 内では外部を呼ばない」不変条件が明示されている。
