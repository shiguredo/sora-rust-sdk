# 変更履歴

- UPDATE
  - 後方互換がある変更
- ADD
  - 後方互換がある追加
- CHANGE
  - 後方互換のない変更
- FIX
  - バグ修正

## develop

- [CHANGE] `Mp4Error::InconsistentSampleDescription` から `fields` を削除し、 `InvalidAv1Track` を追加する
  - サンプルエントリーの相違は `index` のみを報告する
  - AV1 track 検証の失敗は文脈入りのメッセージで報告する
  - @sile
- [CHANGE] `Mp4Error::InvalidH264Track` を追加する
  - H.264 track 検証の失敗は文脈入りのメッセージで報告する
  - @sile
- [CHANGE] `Mp4Error` の Display メッセージを英語に統一する
  - 既存 variant の日本語メッセージを英語に置き換える
  - `InvalidAv1Track` の内部メッセージも英語にする
  - @sile
- [CHANGE] `VideoCodecPreference::has_implementation` の引数を実値から参照に変更する
  - 不要な所有権移動を避けるため `&VideoCodecImplementation` を受ける
  - @melpon
- [ADD] AudioEncoder / AudioDecoder をユーザー側でカスタマイズ可能にするための音声コーデックフレームワークを追加する
  - `AudioCodecCapability` trait (`src/audio_codec_capability.rs`) を追加する
  - `AudioCodecImplementation` (`src/audio_codec_capability.rs`) を追加する
  - `AudioCodecPreference` / `AudioPreferenceCodec` (`src/audio_codec_preference.rs`) を追加する
  - `validate_audio_codec_preference` (`src/audio_codec_preference.rs`) を追加する
  - `Error::InvalidAudioCodecCapability` / `Error::InvalidAudioCodecPreference` (`src/error.rs`) を追加する
  - `InternalAudioCodecCapability` (`src/audio_codecs/internal.rs`) を追加する
  - `SoraConnectionContextConfig` に `audio_codec_preference` / `audio_codec_capabilities` を追加する。デフォルトは `InternalAudioCodecCapability` のみで、builtin のうち Opus / ISAC / G722 / PCMU / PCMA を広告する
  - `SoraAudioEncoderFactory` / `SoraAudioDecoderFactory` (`src/audio_codec.rs`) を内部実装として追加する
  - shiguredo_webrtc の `AudioEncoder` / `AudioDecoder` をユーザー注入可能にする upstream API に依存する
  - @melpon
- [ADD] Mp4SampleReader を複数の Mp4VideoCapturer で共有できるようにする
  - @sile
- [UPDATE] shiguredo_mp4 を 2026.4.0 から 2026.5.0 に更新する
  - @sile
- [UPDATE] shiguredo_webrtc を 0.152.1-canary.1 から 0.152.1-canary.2 に更新する
  - @melpon
- [FIX] MP4 AV1 の `configOBUs` を各 sync sample の先頭に付与するようにする
  - 今までは `configOBUs` を破棄しており、Sequence Header OBU や静的 Metadata OBU が sync sample に含まれない入力では受信側が decode できない payload になっていた
  - `Mp4SampleReader` 初期化時に AV1 track の OBU 列と Sequence Header 一貫性 / RTP packetizer 順序 / random access 条件を検証し、不正な入力は `Mp4Error::InvalidAv1Track` で拒否する
  - `Mp4VideoTrackInfo` に AV1CodecConfigurationRecord 由来の field と `configOBUs` を保持し、sample entry 一貫性検証の比較対象へ含める
  - AV1 required SDP format に `av1C` 由来の `profile` / `level-idx` / `tier` を 10 進文字列で明示し、incoming との照合で profile 完全一致と level / tier 上限を検証する
  - @sile
- [FIX] MP4 の H.264 トラックで `avcC` 由来の `profile-level-id` を SDP capability に反映する
  - 今までは `packetization-mode=1` だけを付けた bare `H264` を広告しており、Main / High Profile の MP4 では実 bitstream と capability の profile / level が食い違っていた
  - `Mp4SampleReader` 初期化時に全 SPS を解析し、`avcC` の profile / constraint / level と SPS の一致、SPS と `avc1` の寸法の一致を検証し、不一致は `Mp4Error::InvalidH264Track` で拒否する
  - 空の SPS / PPS リストと NAL type 不正の PPS は `Mp4Error::InvalidH264Track` で拒否する
  - `avcC` 由来の profile-level-id を固定 libwebrtc の `kProfilePatterns` と同じ規則で判定し、認識されない profile / level の入力を `Mp4Error::InvalidH264Track` で拒否する
  - H.264 required SDP format に `packetization-mode=1` に加えて `profile-level-id` を明示し、incoming との照合で sub-profile 完全一致と level 下限を検証する
  - sample entry 一貫性検証の比較対象に `avcC` box 全体と抽出後の profile-level-id を含める
  - ISO/IEC 14496-15 に違反するが実在する chroma 拡張欠落の `avcC` は mp4-rs と同様に受理し、再エンコード不能のため `avcc_box` は `None` として扱う
  - @sile

### misc

## 2026.1.0

**リリース日**: 2026-08-25
