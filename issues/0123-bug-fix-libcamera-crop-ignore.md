# libcamera の crop 情報を反映する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-libcamera-crop-ignore
- Polished: {YYYY-MM-DD}

## 目的

`AdaptedVideoTrackSource::adapt_frame` が返す crop 情報を両パスで反映し、アスペクト比不一致時に画像が歪まないようにする。

## 現状

`src/libcamera.rs` のキャプチャ処理は、`AdaptFrameResult` の crop フィールド (`crop_x` / `crop_y` / `crop_width` / `crop_height`) を mapped パス・native パスの両方で無視して、フルフレームを adapted サイズへスケールする。

- mapped パス (`on_frame_buffer`) は scale のみ行い crop を捨てる
- native パス (`LibcameraNativeFrameBuffer::crop_and_scale`) は crop 引数をすべて破棄し、scaled サイズだけ書き換えたバッファを返す

libwebrtc の `AdaptedVideoTrackSource::AdaptFrame` はアスペクト比・ピクセル数制約に応じて crop 領域を返すことがあるため、カメラとエンコーダのアスペクト比が異なる場合に画像が歪む。既存テスト (`native_frame_buffer_crop_and_scale_updates_scaled_size_only`) が crop 無視を仕様として固定してしまっている。

## 設計方針

- `AdaptFrameResult` の crop 情報を mapped パスで `crop_and_scale` に渡す
- `LibcameraNativeFrameBuffer::crop_and_scale` で crop 領域を実際に切り出してからスケールする
- 既存テストを実装に合わせて修正する

## 完了条件

- アスペクト比不一致時に crop が適用され、画像が歪まない
- crop なしの場合の挙動が従来どおりである
- `crop_and_scale` の crop 動作を検証するテストがある
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs`
- `CHANGES.md`
