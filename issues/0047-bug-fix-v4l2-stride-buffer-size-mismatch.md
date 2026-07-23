# V4L2 の stride とバッファサイズ計算の不整合を直す

- Priority: Medium
- Created: 2026-07-23
- Completed: {YYYY-MM-DD}
- Model: Composer
- Branch: feature/fix-v4l2-stride-buffer-size-mismatch
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 残項目。

## 目的

`src/video_codecs/v4l2.rs` で、`Resolution::yuv420_size()` による長さチェックと、stride ベースの平面分割が一致しない場合に `split_at_mut` パニックや不正コピーが起きないようにする。

## 優先度根拠

Medium。

- エンコードコールバック内でパニックするとプロセスを巻き込む
- 通常の V4L2 stride は偶数アラインであることが多いが、計算式の不整合は残っている
- 修正は局所的で、デコード側の `build_i420_frame` と式を揃えればよい

## 現状

`shiguredo_v4l2::Resolution::yuv420_size` は `stride * height * 3 / 2` を返す。

一方エンコード側（`v4l2.rs` の fill クロージャ付近）は:

1. `buf.len() < resolution.yuv420_size()` で早期 return
2. その後 `y_size = stride * height`、`uv_size = chroma_stride * chroma_height`（`div_ceil`）で `split_at_mut`

奇数の `stride` / `height` では 1 の通過後に 2 の必要バイト数が `yuv420_size()` を超え、`split_at_mut` でパニックしうる。

デコード側の `build_i420_frame`（`v4l2.rs:54`）は平面サイズを `checked_mul` / `div_ceil` で積み上げてから `data.len()` を検査しており、式が揃っていない。

## 設計方針

- バッファ長の検査と平面分割に同じサイズ計算を使う
- 既存の `build_i420_frame` 側の計算を正とし、エンコード側を合わせる（または共通ヘルパーに寄せる）
- 本 issue ではヘルパー横断の大規模共通化はしない（それは #0049）

## 完了条件

- エンコード側の長さチェックと平面分割が同じサイズ定義を使う
- 奇数 stride / height を含む単体テスト、または同等の計算テストでパニックしないことが確認できる
- `cargo test -p sora_sdk --features libcamera`（または当該コードがビルドされる feature）が通る

## 解決方法

1. `yuv420_size()` 依存をやめ、`build_i420_frame` と同じ Y/U/V サイズ計算で `buf.len()` を検査する
2. 必要ならサイズ計算を `v4l2.rs` 内の小さな関数に切り出す
3. 端数ケースの単体テストを追加する
