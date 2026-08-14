# libcamera のコピー系テストに stride > width のケースを追加する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-libcamera-stride-tests
- Polished: 2026-08-15

## 目的

libcamera の plane コピー処理のテストを、実機で発生する stride > width (アライメント) のケースまで拡張する。

## 現状

`src/libcamera.rs` の `copy_i420_planes_to_buffer` / `copy_nv12_planes_to_buffer` の正パステストは、幅 4 で Y plane 長がちょうど幅×高さ (stride == width) の余白なしの入力のみを使用している。既存のエラーパステスト (`copy_i420_planes_to_buffer_rejects_invalid_stride` など) は plane 長が rows で割り切れない入力を対象としており、stride > width の成功系ケースは一度も検証されていない。

libcamera は `stream_config.stride()` を width と独立に提供する (src/libcamera.rs の `stream_config.stride()` 参照) ため、実機ではアライメントにより stride が width より大きくなることがある。`plane_stride_from_len` は「len / rows」で stride を推測し、`i420_copy` / `nv12_copy` が推測した stride で padding をスキップしてコピーする。stride が width より大きい入力は、この一連の処理が padding を正しく扱えるかの本命ケースである。

## 設計方針

- stride > width の入力で padding が正しくスキップされるテストを追加する (例: I420 の幅 4・高さ 2 で Y plane 長 16 (stride 8)・U/V plane 長 8。NV12 も同様に Y plane 長 16・UV plane 長 8)
  - padding バイトに識別値 (例: 0xFF) を入れ、出力バッファにデータバイトのみがコピーされることを assert する。padding は行ごとにデータの直後に配置し (例: 行 0 = データ 4 バイト + 0xFF 4 バイト、行 1 = データ 4 バイト + 0xFF 4 バイト)、stride の誤推測を検出できるようにする
- 例の通り stride > width を再現できればよく、特定の実機のアライメント値に合わせる必要はない
- plane 長が rows で割り切れないエラーパスは、既存のエラーパステスト (`copy_i420_planes_to_buffer_rejects_invalid_stride` など) が既に検証しているため追加しない

## 完了条件

- stride > width のケースでコピーが正しく行われるテストが `copy_i420_planes_to_buffer` / `copy_nv12_planes_to_buffer` のそれぞれにある
- 既存テストの挙動が変わらない
- `cargo test --workspace --features libcamera` が成功する
- テストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs` (テストモジュール)
