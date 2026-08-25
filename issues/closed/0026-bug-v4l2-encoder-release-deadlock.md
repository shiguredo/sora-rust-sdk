# V4L2 エンコーダーの `release()` が Mutex 保持中に `encoder = None` を実行しデッドロックの恐れがある

- Priority: High
- Created: 2026-06-23
- Completed: 2026-06-25
- Model: Opus 4.7
- Branch: feature/fix-v4l2-encoder-release-deadlock
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 (video codec 層の致命的バグ修正) のうち「`v4l2.rs` の callback と encoder 同居デッドロック懸念」分。

## 目的

`src/video_codecs/v4l2.rs:648-660` の `V4l2VideoEncoder::release()` は `shared_state` の `MutexGuard` を保持したまま `shared_state.encoder = None;` を実行している。`H264Encoder` の Drop 時に drain 処理が同期的に走り、内部から登録済みコールバック (`handle_v4l2_encode_callback`) が呼ばれた場合、そのコールバックは同じ `shared_state` の `lock()` を取りに来るため自己デッドロックする。

同等のリスクを抱える箇所は SDK 内に他にもあったが、いずれも対称的に解消済みである:

- `src/video_codecs/vpl.rs:522-531` の VPL エンコーダー `release()` は `drop(callback_state)` を挟んでから `self.encoder = None;` を行う形にコメント付きで明示修正されている (「self.encoder = None で drain 処理が走ってコールバックハンドラが呼ばれ、コールバックハンドラの中でロックを獲得しようとするので、ここで Mutex を unlock しておかないとデッドロックになる」)
- `src/video_codecs/v4l2.rs:890-897` の V4L2 デコーダー `release()` も `drop(callback_state)` の後で `self.decoder = None;` する形に揃っている

V4L2 エンコーダー側だけが取り残されている。ただし V4L2 エンコーダーは VPL や V4L2 デコーダーと構造が異なり、`encoder` が `EncoderSharedState` (Mutex 内) に包まれているため、単純な `drop(shared_state);` 後に `self.encoder = None;` はできない。正しい最小修正は `Option::take()` で encoder を Mutex 内から取り出し、Mutex を解放した後で encoder を drop する方法である。

なお `handle_v4l2_convert_callback` (`v4l2.rs:281`) も同じ `Mutex` を lock するが、`release()` では converter を encoder より先に drop するため (L651)、encoder の drain が convert callback を発火させることはない。本 issue のデッドロックシナリオには無関係である。

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

なお `EncoderSharedState` は `encoder` と `callback` の両方を含む単一構造体で、`handle_v4l2_encode_callback` (`src/video_codecs/v4l2.rs:232-235`) は `shared_state.lock().unwrap()` を取得する。エンコーダーの drain がこのコールバックを発火させた瞬間にデッドロックが成立する。

VPL エンコーダーや V4L2 デコーダーと異なり、V4L2 エンコーダーは encoder が `Arc<Mutex<EncoderSharedState>>` の内部にある。この構造的差異により、VPL の `drop(callback_state); self.encoder = None;` というパターンはそのまま適用できない。`encoder = None` を実行するには Mutex の再取得が必要だが、再取得すると encoder の drop 時に MutexGuard が生存しているため結局デッドロックする。

`Option::take()` により encoder の所有権を Mutex 内から取り出し、Mutex を解放した後で drop する方法が最小限の正しい修正となる。

## 設計方針

- `Option::take()` を用いて encoder の所有権を Mutex 内から取り出し、Mutex を解放した後で encoder を drop する
- `shared_state.callback = None;` までは Mutex 保持で行う。これにより drain 発火時にコールバックが `callback = None` を検出して早期 return する
- encoder の drop 後に `handle_v4l2_encode_callback` が発火した場合も、`callback = None` により `return;` するためデッドロックしない
- 同位置にコメント (vpl.rs と同等の内容、日本語) を残し、再発防止する
- `#[cfg(feature = "libcamera")]` のフィールド (`converter` / `native_input_config`) は encoder より先に drop されており、encoder の drain が `handle_v4l2_convert_callback` を発火させることはないため、修正不要
- `rebuild_needed` / `input_mode` のリセットも現状のままで問題ないため、順序のみ調整する

### 修正コード

```rust
fn release(&mut self) -> VideoCodecStatus {
    #[cfg(feature = "libcamera")]
    {
        self.converter = None;
        self.native_input_config = None;
    }
    self.rebuild_needed = false;
    self.input_mode = EncoderInputMode::MmapI420;
    // shared_state.encoder.take() で所有権を取り出し、ロック外で drop する。
    // shared_state.encoder の drop 時に drain 処理が走ってコールバックハンドラが
    // 呼ばれるが、先に callback = None にしているのでコールバックは早期 return する。
    let encoder = {
        let mut shared_state = self.shared_state.lock().unwrap();
        shared_state.callback = None;
        shared_state.encoder.take()
    };
    drop(encoder);
    VideoCodecStatus::Ok
}
```

注: `encoder` を `EncoderSharedState` から外す案は検討したが、`H264Encoder` はスレッドセーフではないため `handle_v4l2_convert_callback`（別スレッド）との排他に `Arc<Mutex<>>` が必要であり、現在の `EncoderSharedState` 内に encoder を置く設計は正しい。本 issue では `Option::take()` による最小変更でデッドロックを解消する。

## 完了条件

- `V4l2VideoEncoder::release()` で `shared_state.encoder` の所有権を `Option::take()` で Mutex 内から取り出し、Mutex 解放後に drop していること
- encoder の drop による drain がコールバックハンドラ (`handle_v4l2_encode_callback`) を発火させた場合、`shared_state.callback = None` により早期 return し、デッドロックしないこと
- vpl.rs の release() と同等のコメント（日本語）が付与されていること
- 公開 API (`V4l2VideoCodecCapability`) のシグネチャと振る舞いに変更がないこと
- `src/video_codecs/v4l2.rs:998-1076` の既存単体テストが修正後もすべて通過すること
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/video_codecs/v4l2.rs:648-660` の `release()` を `Option::take()` を使った以下の形に書き換える:

   ```rust
   fn release(&mut self) -> VideoCodecStatus {
       #[cfg(feature = "libcamera")]
       {
           self.converter = None;
           self.native_input_config = None;
       }
       self.rebuild_needed = false;
       self.input_mode = EncoderInputMode::MmapI420;
       // shared_state.encoder.take() で所有権を取り出し、ロック外で drop する。
       // shared_state.encoder の drop 時に drain 処理が走ってコールバックハンドラが
       // 呼ばれるが、先に callback = None にしているのでコールバックは早期 return する。
       let encoder = {
           let mut shared_state = self.shared_state.lock().unwrap();
           shared_state.callback = None;
           shared_state.encoder.take()
       };
       drop(encoder);
       VideoCodecStatus::Ok
   }
   ```

   注: `H264Encoder` の Drop が drain を同期的に発火させることは VPL 側のコメント (`vpl.rs:525-527`) で既に確認済みであり、V4L2 側も同様の挙動を示す。
2. 既存単体テスト (`src/video_codecs/v4l2.rs:998-1076`) が通過することを確認する
3. `CHANGES.md` に `[FIX] V4L2 エンコーダーの release() で Mutex 保持中に encoder を drop してデッドロックする問題を修正する` エントリを追記する
4. `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通ることを確認する

## 関連

- `src/video_codecs/vpl.rs:522-531`: 同種問題の解決実装 (コメント付き)
- `src/video_codecs/v4l2.rs:890-897`: V4L2 デコーダー側の解決実装

## 解決方法

`V4l2VideoEncoder::release()` で `shared_state.encoder` を `Option::take()` で Mutex 内から取り出し、Mutex 解放後に drop するよう修正。

- `shared_state.callback = None` を先に設定し、encoder drop 時の drain がコールバックを発火させても早期 return する
- VPL エンコーダー (`vpl.rs`) と同等の日本語コメントを付与

### 修正ファイル
- `src/video_codecs/v4l2.rs`: `release()` の修正
- `CHANGES.md`: [FIX] エントリ追加
