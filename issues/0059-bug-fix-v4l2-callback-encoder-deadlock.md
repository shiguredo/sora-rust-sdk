# V4L2 エンコーダーの `rebuild_*` で Mutex 保持中に古い encoder を Drop しデッドロックする経路を解消する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-28
- Model: DeepSeek V4 Pro
- Branch: feature/fix-v4l2-callback-encoder-deadlock
- Polished: 2026-07-27

## 目的

`rebuild_mmap_encoder` (`v4l2.rs:363-364`) と `rebuild_native_pipeline` (`v4l2.rs:416-417`) の両方で、`shared_state` ロック保持中に `shared_state.encoder = Some(new_encoder)` を実行している。この代入で古い `H264Encoder` の Drop が同期的に走る。

`H264Encoder::Drop` は `poller.stop()` → `thread.join()` を実行する。ポーラースレッドが `handle_v4l2_encode_callback` → `shared_state.lock()` を取得しようとした場合、呼び出し元スレッドとポーラースレッドの間で循環待機 (デッドロック) が成立する。

`release()` (`v4l2.rs:651-669`) は #0026 で既に `take()` パターンにより修正済みだが、`rebuild_*` 側の経路だけが取り残されているため、同じパターンで修正する。

## 優先度根拠

High。デッドロックは `SoraConnection` タスク全体を hang させ、一度発生すると再現条件の追跡が困難。トリガーは解像度変更やエンコーダー再初期化で、通常運用中に発生しうるパス。`release()` は既に同じパターンで修正済みであり、修正漏れの箇所を対称的に直すだけ。

## 現状

### 経路 1: `rebuild_mmap_encoder` (`v4l2.rs:345-375`)

```rust
fn rebuild_mmap_encoder(&mut self) -> Result<()> {
    // ...
    let encoder = H264Encoder::new(config, FnEncodeHandler::new(move |result| {
        handle_v4l2_encode_callback(&shared_state, result);
    }))?;

    let mut shared_state = self.shared_state.lock().unwrap();
    shared_state.encoder = Some(encoder);  // ← 古い encoder がここで Drop → poller.stop() → thread.join()
    drop(shared_state);
    // ...
}
```

### 経路 2: `rebuild_native_pipeline` (`v4l2.rs:380-426`, libcamera パス)

```rust
fn rebuild_native_pipeline(&mut self) -> Result<()> {
    // ...
    let mut shared_state = self.shared_state.lock().unwrap();
    shared_state.encoder = Some(encoder);  // ← 同上
    drop(shared_state);
    // ...
}
```

いずれも `shared_state` ロック保持中に古い encoder が Drop され、`poller.stop()` → `thread.join()` でポーラースレッドの終了を待つ。ポーラースレッドが `handle_v4l2_encode_callback` (`v4l2.rs:202-251`) 内で `shared_state.lock()` を取得しようとするとデッドロックする。

### ロック範囲を絞るべき箇所（デッドロックはしないが設計上改善）

`V4l2VideoEncoder::encode()` MMAP パス (`v4l2.rs:621-638`):

```rust
let mut shared_state = self.shared_state.lock().unwrap();
let Some(encoder) = shared_state.encoder.as_mut() else { ... };
encoder.encode(EncodeInput::Mmap(&mut fill), ...)
```

`handle_v4l2_convert_callback` (`v4l2.rs:284-306`):

```rust
let mut shared_state = shared_state.lock().unwrap();
let Some(encoder) = shared_state.encoder.as_mut() else { ... };
encoder.encode(EncodeInput::DmaBuf { ... }, ...)
```

これらは `encode()` が同期的に CAPTURE DQBUF やコールバックハンドラを呼ばないためデッドロックしない。しかしロック保持中に外部呼び出しをする設計は競合を増やし、将来的な変更に対しても脆弱である。

なお `handle_v4l2_convert_callback` はコンバーターのポーラースレッドから呼ばれ、`V4l2VideoEncoder::encode()` と並行実行可能（前者は `Arc` 経由、後者は `&mut self` + `Arc<Mutex<>>` 経由）。take/put-back パターンを両方に適用すると、片方が encoder を take 中にもう片方が rebuild をトリガーし、put-back で新 encoder を古い encoder で上書きする競合が発生する。したがって `handle_v4l2_convert_callback` は現状のロック保持パターンを維持し、`V4l2VideoEncoder::encode()` 側のみロック範囲を縮小する。

### 既に正しい箇所

`release()` (`v4l2.rs:651-669`) は #0026 の修正により既に `take()` パターンで安全:

```rust
let encoder = {
    let mut shared_state = self.shared_state.lock().unwrap();
    shared_state.callback = None;
    shared_state.encoder.take()
};
drop(encoder);  // ロック外で Drop
```

`handle_v4l2_encode_callback` (`v4l2.rs:235-251`) も既に「ロック内で callback ポインタ (Copy 型) を取り出し、ロック外で `on_encoded_image` を呼ぶ」正しいパターン。修正不要。

## 設計方針

修正の基本戦略は #0026 の `release()` と同様、`Option::take()` で古い encoder の所有権を Mutex 内から取り出し、ロック外で Drop する。

### step 1: `rebuild_mmap_encoder` の修正

```rust
fn rebuild_mmap_encoder(&mut self) -> Result<()> {
    // ...
    let new_encoder = H264Encoder::new(config, FnEncodeHandler::new(move |result| {
        handle_v4l2_encode_callback(&shared_state, result);
    }))?;

    // 古い encoder を take() で取り出し、ロック外で drop する。
    // この間 shared_state.encoder は None になるが、
    // rebuild_mmap_encoder は &mut self で呼ばれるため encode() とは排他。
    // handle_v4l2_convert_callback は encoder が None の場合フレームを
    // ドロップする（警告ログ出力して return）。rebuild 中の短期間のみであり許容範囲。
    let old_encoder = {
        let mut shared_state = self.shared_state.lock().unwrap();
        shared_state.encoder.take()
    };
    drop(old_encoder);

    // converter を encoder assign より先に Drop する。
    // 旧 converter の最終コールバックが encoder=None を参照すれば
    // 安全にフレームドロップできる。encoder assign 後に converter を
    // Drop すると、新 encoder に旧フォーマットのフレームが届き不整合になる。
    // converter の Drop でも poller.stop() → thread.join() が走り、
    // 最大 500ms ブロックする可能性がある。
    #[cfg(feature = "libcamera")]
    {
        self.converter = None;
        self.native_input_config = None;
    }

    // converter が落ちた後、新しい encoder を設定する。
    {
        let mut shared_state = self.shared_state.lock().unwrap();
        shared_state.encoder = Some(new_encoder);
    }

    self.input_mode = EncoderInputMode::MmapI420;
    self.rebuild_needed = false;
    Ok(())
}
```

`callback` は `None` にしない。`release()` と異なり、エンコーダーは引き続き使用されるため callback を残す必要がある。Drop 中に `handle_v4l2_encode_callback` が発火しても、`shared_state` ロックは解放済みのためデッドロックせず、callback も有効なままである。

### step 2: `rebuild_native_pipeline` の修正

`rebuild_mmap_encoder` と同じ take/drop/assign の 3 段階パターンを適用する。encoder の take/drop は converter の置換より先に行い、converter の置換は encoder assign より先に行う。

```rust
fn rebuild_native_pipeline(&mut self) -> Result<()> {
    // ...native_input_config のバリデーション...

    let mut encoder_config = EncoderConfig::new(/* ... */);
    encoder_config.device_path = self.device_path.clone();
    // ...
    let shared_state = self.shared_state.clone();
    let new_encoder = H264Encoder::new(
        encoder_config,
        FnEncodeHandler::new(move |result| {
            handle_v4l2_encode_callback(&shared_state, result);
        }),
    )?;

    // new_converter は encoder assign 前に作成する（作成には shared_state は不要）。
    // encoder assign 後にコールバックが新 encoder を参照できるようにする。
    let mut converter_config = ConverterConfig::new(/* ... */);
    // ...
    let shared_state = self.shared_state.clone();
    let new_converter = ImageConverter::new(converter_config, move |result| {
        handle_v4l2_convert_callback(&shared_state, result);
    })?;

    // 1. 古い encoder を take し、ロック外で drop
    let old_encoder = {
        let mut shared_state = self.shared_state.lock().unwrap();
        shared_state.encoder.take()
    };
    drop(old_encoder);

    // 2. 古い converter を encoder assign より先に Drop
    //    旧 converter の最終コールバックは encoder=None を見て安全にフレームドロップする
    self.converter = Some(new_converter);  // 旧 converter がここで Drop

    // 3. 新しい encoder を設定
    {
        let mut shared_state = self.shared_state.lock().unwrap();
        shared_state.encoder = Some(new_encoder);
    }

    self.width = native_config.scaled_width;
    self.height = native_config.scaled_height;
    self.input_mode = EncoderInputMode::NativeDmabuf;
    self.rebuild_needed = false;
    Ok(())
}
```

`new_converter` の作成が失敗した場合、encoder は未 assign のままになり状態は保全される。`self.converter` も更新されていないため、converter 不在 + encoder 不在の状態は `encode()` 内の rebuild チェック (`!self.shared_has_encoder() || self.converter.is_none()`) により次フレームで検出され再試行される。`InputMode` が `NativeDmabuf` のままだが、`encoder` も `converter` も存在しないため、`encode()` の早期チェックで安全に弾かれる。

### step 3: `V4l2VideoEncoder::encode()` MMAP パスのロック範囲縮小

`V4l2VideoEncoder::encode()` は `&mut self` で呼ばれるため、他スレッドの `encode()` と競合しない。take/put-back パターンが安全に適用できる。

```rust
let mut encoder_opt = self.shared_state.lock().unwrap().encoder.take();
let Some(ref mut enc) = encoder_opt else {
    rtc_log_error!("V4L2 encode failed: encoder is not initialized");
    return VideoCodecStatus::Error;
};
let status = match enc.encode(
    EncodeInput::Mmap(&mut fill),
    timestamp_us,
    force_keyframe,
    callback_value,
) {
    Ok(()) => VideoCodecStatus::Ok,
    Err(V4l2Error::NoAvailableBuffer) => VideoCodecStatus::NoOutput,
    Err(err) => {
        rtc_log_error!("V4L2 encode failed: {}", err);
        VideoCodecStatus::Error
    }
};
self.shared_state.lock().unwrap().encoder = encoder_opt;
status
```

encoder が `None` の場合、`take()` 前後で状態は変わらず（`None` → `None`）、早期 return される。永久的に encoder が失われることはない。

take 中は `shared_state.encoder` が `None` になる。この期間に `handle_v4l2_convert_callback` が発火すると `encoder.as_mut()` が `None` を返しフレームがドロップされる。`encode()` の呼び出しは同期的かつ短時間であるため許容範囲。

### step 4: `handle_v4l2_convert_callback` は現状維持

`handle_v4l2_convert_callback` はコンバーターのポーラースレッドから呼ばれ、`V4l2VideoEncoder::encode()` と並行実行可能である。take/put-back を適用すると、encode() 側が rebuild をトリガーした場合に put-back が新 encoder を上書きする競合が発生する。また `encoder.encode()` の内部でデッドロックする経路も存在しないため、現状のロック保持パターンを維持する。

### step 5: `handle_v4l2_encode_callback` は修正不要（現状維持）

既にロック内では `callback` ポインタの取り出しのみ行い、`on_encoded_image` はロック外で呼んでいる。修正不要。

### step 6: `set_rates()` も現状維持

`set_rates()` (`v4l2.rs:678-691`) はロック保持中に `encoder.set_bitrate()` を呼んでいるが、`set_bitrate()` は ioctl のみでブロックもコールバック発火も行わないためデッドロックしない。また `&mut self` で呼ばれるため encode() とは排他。現状維持とする。

## 完了条件

- `rebuild_mmap_encoder` で古い encoder の Drop が `shared_state` ロック外で行われること
- `rebuild_native_pipeline` で古い encoder の Drop が `shared_state` ロック外で行われること
- `V4l2VideoEncoder::encode()` MMAP パスで `encoder.encode()` がロック外で呼ばれること
- `handle_v4l2_convert_callback` は現状維持（take/put-back 非適用の理由をコードコメントで明示すること）
- 公開 API (`V4l2VideoCodecCapability`) のシグネチャと振る舞いに変更がないこと
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace` が通ること
- `src/video_codecs/v4l2.rs` 内の既存単体テストが修正後もすべて通過すること
- 各修正箇所に「ロック中に外部呼び出しをしない」「take/put-back を適用可能な条件」を日本語コメントで明示すること

## 解決方法

`Option::take()` パターンを用いて、以下の 3 箇所の `shared_state` ロック範囲を縮小した。

1. `rebuild_mmap_encoder`: `shared_state` ロック中に `encoder.take()` で古い encoder の所有権を取り出し、ロック解放後に Drop する。converter の Drop は encoder assign より先に行う。
2. `rebuild_native_pipeline`: 同様に take/drop/assign の 3 段階パターンを適用。
3. `V4l2VideoEncoder::encode()` MMAP パス: `encoder.take()` で encoder を取り出し、ロック外で `encode()` を呼び、put-back する。

`handle_v4l2_convert_callback` は並行実行の競合リスクがあるため take/put-back を適用せず現状維持とした。

## 関連

- #0026: `release()` の同種デッドロックを `take()` パターンで修正済み
- `src/video_codecs/vpl.rs:522-531`: VPL エンコーダーの `release()` も同パターン（コメント付き）
