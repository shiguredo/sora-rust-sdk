# V4L2 エンコーダーの `release()` が Mutex 保持中に `encoder = None` を実行しデッドロックの恐れがある

- Priority: High
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-v4l2-encoder-release-deadlock
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 (video codec 層の致命的バグ修正) のうち「`v4l2.rs` の callback と encoder 同居デッドロック懸念」分。

## 目的

`src/video_codecs/v4l2.rs:648-660` の `V4l2VideoEncoder::release()` は `shared_state` の `MutexGuard` を保持したまま `shared_state.encoder = None;` を実行している。エンコーダー破棄時に drain 処理が走り、内部から登録済みコールバック (`handle_v4l2_encode_callback`) が呼ばれた場合、そのコールバックは同じ `shared_state` の `lock()` を取りに来るため自己デッドロックする。

同等のリスクを抱える箇所は SDK 内に他にもあったが、いずれも対称的に解消済みである:

- `src/video_codecs/vpl.rs:522-531` の VPL エンコーダー `release()` は `drop(callback_state)` を挟んでから `self.encoder = None;` を行う形にコメント付きで明示修正されている (「self.encoder = None で drain 処理が走ってコールバックハンドラが呼ばれ、コールバックハンドラの中でロックを獲得しようとするので、ここで Mutex を unlock しておかないとデッドロックになる」)
- `src/video_codecs/v4l2.rs:890-897` の V4L2 デコーダー `release()` も `drop(callback_state)` の後で `self.decoder = None;` する形に揃っている

V4L2 エンコーダー側だけが取り残されている。同パターンに揃える。

## 優先度根拠

High。

- 既に他箇所 (vpl エンコーダー / v4l2 デコーダー) でデッドロック対策として明示修正されている事象であり、当該パターンが実発火する根拠は SDK 内に揃っている
- `release()` は WebRTC のセッション再生成・コーデック切替などで通常運用中に呼ばれるパスであり、特異な異常系ではない
- 修正規模は数行で、既存の対称実装をそのまま移植するだけ
- 「If it hurts, do it more often」「Don't live with broken windows」(AGENTS.md) の観点でも、対称性の崩れは即座に正す

## 現状

`src/video_codecs/v4l2.rs:648-660`:

```rust
fn release(&mut self) -> VideoCodecStatus {
    #[cfg(feature = "libcamera")]
    {
        self.converter = None;
        self.native_input_config = None;
    }
    self.rebuild_needed = false;
    self.input_mode = EncoderInputMode::MmapI420;
    let mut shared_state = self.shared_state.lock().unwrap();
    shared_state.callback = None;
    shared_state.encoder = None;     // ← MutexGuard 保持中に drain 起動
    VideoCodecStatus::Ok
}
```

参考: VPL エンコーダー側 (`src/video_codecs/vpl.rs:522-531`) は次の形:

```rust
fn release(&mut self) -> VideoCodecStatus {
    let mut callback_state = self.callback_state.lock().unwrap();
    callback_state.callback = None;
    // self.encoder = None で drain 処理が走ってコールバックハンドラが呼ばれ、
    // コールバックハンドラの中でロックを獲得しようとするので、
    // ここで Mutex を unlock しておかないとデッドロックになる
    drop(callback_state);
    self.encoder = None;
    VideoCodecStatus::Ok
}
```

V4L2 デコーダー側 (`src/video_codecs/v4l2.rs:890-897`) も同様:

```rust
fn release(&mut self) -> VideoCodecStatus {
    let mut callback_state = self.callback_state.lock().unwrap();
    callback_state.callback = None;
    callback_state.resolution = None;
    drop(callback_state);
    self.decoder = None;
    VideoCodecStatus::Ok
}
```

なお `EncoderSharedState` は `encoder` と `callback` の両方を含む単一構造体で、`handle_v4l2_encode_callback` (`src/video_codecs/v4l2.rs:232-235` 付近) と `handle_v4l2_convert_callback` (`src/video_codecs/v4l2.rs:281` 付近) のいずれも同じ `Mutex` を `lock()` する。エンコーダーの drain がこれらのコールバックを発火させた瞬間にデッドロックが成立する。

## 設計方針

- `vpl.rs:522-531` および `v4l2.rs:890-897` と同じパターンに揃える
- `shared_state.callback = None;` までは Mutex 保持で行い、`drop(shared_state)` の後で `encoder = None` を実行する
- 同位置にコメント (vpl.rs と同等の内容、日本語) を残し、再発防止する
- `#[cfg(feature = "libcamera")]` のフィールド (`converter` / `native_input_config`) は `shared_state` のロック外で扱っているため触らない
- `rebuild_needed` / `input_mode` のリセットも現状のままで問題ないため、順序のみ調整する

## 完了条件

- `V4l2VideoEncoder::release()` で `shared_state.encoder = None;` を実行する前に `MutexGuard` が drop されている
- 同コーデックの drain がコールバックハンドラを再入させた場合にもデッドロックしない構造になっている
- vpl.rs と同等のコメントが付与されている
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/video_codecs/v4l2.rs:648-660` の `release()` を以下のような順序に書き換える:
   ```rust
   fn release(&mut self) -> VideoCodecStatus {
       #[cfg(feature = "libcamera")]
       {
           self.converter = None;
           self.native_input_config = None;
       }
       self.rebuild_needed = false;
       self.input_mode = EncoderInputMode::MmapI420;
       let mut shared_state = self.shared_state.lock().unwrap();
       shared_state.callback = None;
       // shared_state.encoder = None で drain 処理が走ってコールバックハンドラが呼ばれ、
       // コールバックハンドラの中でロックを獲得しようとするので、
       // ここで Mutex を unlock しておかないとデッドロックになる
       drop(shared_state);
       self.shared_state.lock().unwrap().encoder = None;
   }
   ```
   (実装時は `shared_state.encoder` を取り出すために再度 lock するか、`EncoderSharedState` 側を分割するかの設計判断を行う。vpl.rs の場合は encoder が外側にあったので drop だけで済んだが、V4L2 は encoder が shared_state の中にあるため、再 lock するなら lock の往復が増える。素直には `EncoderSharedState` から encoder を外側 (`V4l2VideoEncoder` 直下) に移すリファクタリングが望ましい)
2. リファクタリング案 (推奨): `encoder` を `Arc<Mutex<EncoderSharedState>>` の中から外し、`V4l2VideoEncoder` の直下フィールドに移す。これにより vpl.rs と同じく callback_state lock の外で `self.encoder = None;` が呼べる
3. その場合、`shared_state` 経由で encoder を参照していた箇所 (`shared_state.encoder.as_mut()` 等) も書き換える
4. リファクタリングの範囲が広がる場合は本 issue では対称化 (lock の取り直し) のみに留め、構造変更は別 issue として切り出す判断もあり得る

## 関連

- `src/video_codecs/vpl.rs:522-531`: 同種問題の解決実装 (コメント付き)
- `src/video_codecs/v4l2.rs:890-897`: V4L2 デコーダー側の解決実装
