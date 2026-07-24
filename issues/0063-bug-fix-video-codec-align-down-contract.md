# `align_down` が「align 不可能な場合に非アライン値を返す」契約破綻を修正する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-video-codec-align-down-contract
- Polished: {YYYY-MM-DD}

## 目的

`src/video_codec.rs:153-159` の `align_down` は名前と裏腹に、`value < alignment` のとき aligned 値 (0) を採用せず **元の非アライン値をそのまま返す**。`apply_alignment_to_codec` の呼び出し側は 0 以下だけ弾いており、simulcast の低解像度レイヤーで容易に「aligned に切り下げられていない解像度」が下流エンコーダに届いてしまう。関数契約を「align 不可能なら None を返す」に修正する。

## 優先度根拠

High。ハードウェアエンコーダ (AMF / NVCODEC / VPL) はコーデックごとに解像度アライン制約 (H.264 は 16、AV1 は 64x16 等) を持ち、非アライン解像度で init すると失敗するか、最悪の場合破損した符号化を行う。simulcast の 3 レイヤーで最低解像度が alignment 未満になるのは実運用で発生し得るシナリオ。

## 現状

```rust
fn align_down(value: i32, alignment: i32) -> i32 {
    if value <= 0 || alignment <= 1 {
        return value;
    }
    let aligned = value - (value % alignment);
    if aligned > 0 { aligned } else { value }
}
```

例: `align_down(15, 16)` は `aligned = 0` → `else` 分岐で `value = 15` を返す。

`apply_alignment_to_codec` (video_codec.rs:161-) は `aligned_codec_width <= 0` しか弾かないため、`codec.set_width(15)` が通ってしまい、下流エンコーダには「16 でアラインされていない 15」がそのまま届く。

## 設計方針

1. `align_down` のシグネチャを `fn align_down(value: i32, alignment: i32) -> Option<i32>` に変更。
2. `value <= 0 || alignment <= 1` は `None` を返す (今までの「素通し」挙動を破棄)。
3. `aligned == 0` の場合も `None` を返す。
4. 呼び出し側 `apply_alignment_to_codec` (video_codec.rs:161-) で `None` を検出し、以下のいずれかで対応:
   - 該当 simulcast stream / codec について「アライン不能」を明示的に無効化する (幅高さ 0 に設定 + `rtc_log_warning!`)。
   - もしくは関数全体で `None` を返し、上位で判断させる。
5. 上位でどう判断するかは issue 0064 (`apply_alignment_to_codec` の部分状態不整合) と合わせて設計する。

## 完了条件

- `align_down(value, alignment)` が `Option<i32>` を返し、align 不可能な入力に対して必ず `None` を返す。
- `align_down(15, 16)` が `None` を返す単体テストが追加されている。
- `apply_alignment_to_codec` から下流エンコーダに非アライン解像度が届く経路が存在しない。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
