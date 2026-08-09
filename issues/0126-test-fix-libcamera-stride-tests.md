# libcamera のコピー系テストに stride > width のケースを追加する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-libcamera-stride-tests
- Polished: {YYYY-MM-DD}

## 目的

libcamera の plane コピー処理のテストを、実機でほぼ常時発生する stride > width (アライメント) のケースまで拡張する。

## 現状

`src/libcamera.rs` の `copy_i420_planes_to_buffer` / `copy_nv12_planes_to_buffer` のテストは、すべて幅 4 で plane 長がちょうど幅×高さ (stride == width) の入力のみを使用している。

`plane_stride_from_len` は「len / rows」で stride を推測するロジックであり、実機 (Raspberry Pi の 64 バイトアライン等) で常時発生する「stride > width」のケースで一度も検証されていない。stride が width より大きい入力で padding を正しくスキップできるかは、この推測ロジックの本命ケースである。

## 設計方針

- stride > width の入力 (例: 幅 4・高さ 2・plane 長 16) で padding が正しくスキップされるテストを追加する
- plane 長が rows で割り切れない入力のエラーパスもテストする
- 実機のアライメント値に近いケースを選ぶ

## 完了条件

- stride > width のケースでコピーが正しく行われるテストがある
- plane 長が rows で割り切れないケースのエラーパステストがある
- 既存テストの挙動が変わらない
- `cargo test --workspace` が成功する
- テストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs` (テストモジュール)
