# libcamera の crop 情報を反映する

- Priority: Medium
- Created: 2026-08-10
- Completed: 2026-08-19
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

mapped パスと native パスの両方で `AdaptFrameResult` の crop 情報を反映する。**CPU によるピクセルコピーは一切発生させない**。

- mapped パス (`on_frame_buffer`): `scale` の代わりに `crop_and_scale` を呼び、crop 領域を切り出してから adapted サイズへ縮小する。`I420Buffer` / `NV12Buffer` へは既にコピー済みのため、libyuv が crop+scale を 1 パスで処理し、追加コピーは発生しない。
- native パス (`on_native_frame_buffer` / `LibcameraNativeFrameBuffer`): バッファに crop フィールド (`crop_x` / `crop_y` / `crop_width` / `crop_height`) を追加して `AdaptFrameResult` の crop 情報を保持し、`crop_and_scale` はピクセルを変換せず crop 情報を引き継いだバッファを返す（メタデータのみ）。実際の crop+scale は `src/video_codecs/v4l2.rs` の V4L2 ImageConverter の HWA で行い、DMA-BUF ゼロコピーを維持する。
- `shiguredo_v4l2` に crop サポートが必要。これは crates.io の依存 (`~2026.1`) であるため、crates.io へのリリース更新または `[patch]` で対応する。具体的には次が考えられる:
  - DMABUF 入力の平面オフセット指定（`v4l2_plane.data_offset` に crop 座標を反映する）で入力領域を制御する
  - または V4L2_SEL_TGT_CROP による入力 crop 領域の指定
- 既存テスト (`native_frame_buffer_crop_and_scale_updates_scaled_size_only`) を、crop 情報を引き継ぐ実装に合わせて修正する

crop が非ゼロになるのは `AdaptFrame` がアスペクト比・ピクセル数制約の不一致を検出したときのみで、一致していればフルフレーム（crop なし）のまま従来どおり動作する。

## 完了条件

- アスペクト比不一致時に crop が適用され、画像が歪まない
- native パスで crop 適用時も CPU コピーが発生しない（DMA-BUF ゼロコピーを維持する）
- `shiguredo_v4l2` の crop サポートを利用でき、native パスの crop 動作が HWA で行われる
- crop なしの場合の挙動が従来どおりである
- `LibcameraNativeFrameBuffer::crop_and_scale` の crop 情報引き継ぎを検証するテストがある
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs`
- `src/video_codecs/v4l2.rs`
- `shiguredo_v4l2`（crop サポート追加とバージョン更新）
- `CHANGES.md`

## 解決方法

実機 (Raspberry Pi 4 Model B / kernel 6.18.39) での検証の結果、本 issue の前提が成立しないことが判明したため closed にする。

- `AdaptedVideoTrackSource::adapt_frame` を実 PeerConnection + エンコーダ wants (OpenH264, 1280x720) の構成で実行し、アスペクト比不一致のカメラ解像度 (640x480 / 1920x1080 / 1280x960) で crop 値 (`crop_x` / `crop_y` / `crop_width` / `crop_height`) を確認した
- 結果、`adapt_frame` は常にフルフレームの crop (`crop_x = 0` / `crop_y = 0`) を返し、crop は一切発生しなかった。adapted サイズもソースのアスペクト比を維持する (例: 1280x960 入力 → 960x720)
- これは libwebrtc M150 の `VideoStreamEncoder` が `OnOutputFormatRequest` (アスペクト比 crop を発生させる唯一の経路) を呼ばず、`VideoAdapter::AdaptFrameResolution` の `target_aspect_ratio` が設定されないためである
- そのため「`adapt_frame` が返す crop 情報を native パスで反映して歪みを直す」という本 issue の設計は機能せず、`shiguredo_v4l2` の crop サポートも使われない
- 仮に歪みが観測される場合、原因は crop 無視ではなくコンバータ/エンコーダの解像度設定側にあるため、issue を書き直して改めて調査する必要がある
