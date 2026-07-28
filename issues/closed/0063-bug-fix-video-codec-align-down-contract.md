# `align_down` が「align 不可能な場合に非アライン値を返す」契約破綻を修正する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-28
- Model: Opus 4.7
- Branch: feature/fix-video-codec-align-down-contract
- Polished: 2026-07-27

## 目的

`src/video_codec.rs:153-159` の `align_down` は名前と裏腹に、`value < alignment` のとき aligned 値 (0) を採用せず **元の非アライン値をそのまま返す**。`apply_alignment_to_codec` の呼び出し側は 0 以下だけ弾いており、simulcast の低解像度レイヤーで容易に「aligned に切り下げられていない解像度」が下流エンコーダに届いてしまう。関数契約を「align 不可能なら None を返す」に修正する。

## 優先度根拠

High。ハードウェアエンコーダはコーデックごとに解像度アライン制約（H.264 は 16、AV1 は 64x16 等）を持ち、非アライン解像度で init すると失敗するか、最悪の場合破損した符号化を行う。simulcast の 3 レイヤーで最低解像度が alignment 未満になるのは実運用で発生し得るシナリオ。

なお、現状 `AlignmentEncoderAdapter` を実際に使用しているのは AMF AV1 エンコーダ（`src/video_codecs/amf.rs`）のみ。NVCODEC と VPL には `AlignmentEncoderAdapter` の使用実績がなく、当該バグの即時的な影響範囲は AMF に限定される。

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

`apply_alignment_to_codec`（video_codec.rs:161-）は `aligned_codec_width <= 0` しか弾かないため、`codec.set_width(15)` が通ってしまう。下流エンコーダには「16 でアラインされていない 15」がそのまま届く。

## 設計方針

1. `align_down` のシグネチャを `fn align_down(value: i32, alignment: i32) -> Option<i32>` に変更。
2. 判定は以下の順序で行う（上から順に評価する）:
   1. `alignment <= 0` → `None`（除算ゼロ回避。`alignment == 0` は無効な引数）
   2. `value <= 0` → `None`（align 対象として意味をなさない。`alignment == 1` より先に判定することで、`align_down(-1, 1)` のようなケースで負の値を誤って返さない）
   3. `alignment == 1` → `Some(value)`（1 は任意の整数を割り切るため、常に align 可能）
   4. `let aligned = value - (value % alignment)` を計算し、`aligned > 0` なら `Some(aligned)`、`aligned == 0` なら `None`
6. `apply_alignment_to_codec` のシグネチャは変更せず（`Option<(i32, i32)>` のまま）、内部で `align_down` の `None` 返却を検出して早期 return する。
7. `AlignmentEncoderAdapter::init_encode` で `apply_alignment_to_codec` が `None` を返した場合:
   - `target_size` を `None` に設定する。
   - しかし `self.encoder.init_encode()` を**呼ばずに `VideoCodecStatus::Error` を返す**。非アライン解像度のまま下流エンコーダに渡る経路を塞ぐ。
   - 後続の `encode` 呼び出しでは `target_size == None` となり、アライメントなしの生フレームが下流に届かないよう `encode` も `VideoCodecStatus::Error` を返す。
8. simulcast stream の部分状態不整合（stream だけ非アラインのまま残る）は #0064 で対応する。#0064 では案 A（関数全体で None を返す）を推奨しており、本 issue の設計方針 7 とも整合する。

## 解決方法

`align_down` のシグネチャを `fn align_down(value: i32, alignment: i32) -> Option<i32>` に変更した。
- 判定順序: `alignment <= 0` → `None`, `value <= 0` → `None`, `alignment == 1` → `Some(value)`, それ以外は `aligned > 0` なら `Some(aligned)`、`aligned == 0` なら `None`。
- `apply_alignment_to_codec` 内で `align_down` の `None` 返却を検出し、対応するよう修正。
- `AlignmentEncoderAdapter::init_encode` でアライン不能時に `target_size` を `None` に設定し、`init_encode` と `encode` が `VideoCodecStatus::Error` を返すよう修正。
- 設計方針に沿った単体テストを追加した。

## 完了条件

- `align_down(value, alignment)` が `Option<i32>` を返し、align 不可能な入力に対して必ず `None` を返す。
- 以下の単体テストが追加されている（`src/video_codec.rs` 内の `#[cfg(test)] mod tests`）:
  - `align_down(320, 16)` → `Some(320)`（正常系: アライン不要）
  - `align_down(321, 16)` → `Some(320)`（正常系: 切り下げ）
  - `align_down(16, 16)` → `Some(16)`（境界: value == alignment）
  - `align_down(15, 16)` → `None`（align 不能）
  - `align_down(0, 16)` → `None`（value == 0）
  - `align_down(-1, 16)` → `None`（負の value）
  - `align_down(16, 1)` → `Some(16)`（alignment == 1 は常に align 可能）
  - `align_down(16, 0)` → `None`（alignment == 0 は無効）
- `AlignmentEncoderAdapter::init_encode` がアライン不能時に `VideoCodecStatus::Error` を返し、`encode` も後続で `Error` を返す。
- 既存テスト `alignment_updates_codec_and_simulcast_streams` および `alignment_encoder_adapter_encoder_info_contains_adapter_name` の改修が必要な場合は修正する。
- `apply_alignment_to_codec` から下流エンコーダに非アライン解像度が届く経路が存在しない。
- `cargo test --workspace --all-features` が通る。
- `cargo fmt --all -- --check` が通る。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
