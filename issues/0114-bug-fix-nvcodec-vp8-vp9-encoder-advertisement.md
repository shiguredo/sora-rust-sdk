# NVCodec の VP8/VP9 エンコーダー過剰広告を防ぐ

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-nvcodec-vp8-vp9-encoder-advertisement
- Polished: {YYYY-MM-DD}

## 目的

NVCodec が実際には初期化できない VP8/VP9 エンコーダーを広告しないようにする。

## 現状

`src/video_codecs/nvcodec.rs` では、ハードウェアが VP8/VP9 のエンコード対応を報告すると `supported_formats_for_codec` が `VideoCodecType::Vp8` / `VideoCodecType::Vp9` の SDP フォーマットを返す (エンコード方向の区別がない)。一方 `encoder_codec_config` は `_ => None` で VP8/VP9 に対応しておらず、`NvCodecVideoEncoder::init_encode` → `rebuild_encoder` が必ず `unsupported codec type for encoder config` で失敗する。

NVENC は VP9 エンコードを報告し得るハードウェア (Ada 以降等) があるため、該当環境では `create_video_encoder` は成功するのに `init_encode` が常に Error を返す破損エンコーダーが生成される。`NvCodecVideoCodecCapability` の doc コメントは「エンコードは H.264 / H.265 / AV1、デコードは H.264 / H.265 / AV1 / VP8 / VP9 をサポートする」と意図を明記しており、広告と実装が矛盾している。テストもこの整合性を検証していない。

## 設計方針

- エンコード方向の `get_supported_formats` から VP8/VP9 を除外する (H.264 / H.265 / AV1 のみ)
- `supported_formats_for_codec` と `encoder_codec_config` の対応関係を検証する単体テストを追加する
- デコード方向の VP8/VP9 対応は維持する

## 完了条件

- エンコード方向のサポートフォーマットに VP8/VP9 が含まれない
- デコード方向の VP8/VP9 が維持される
- `supported_formats_for_codec` と `encoder_codec_config` の整合性を検証するテストがある
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/video_codecs/nvcodec.rs`
- `CHANGES.md`
