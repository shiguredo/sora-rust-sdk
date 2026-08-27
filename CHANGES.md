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

- [ADD] Mp4SampleReader を複数の Mp4VideoCapturer で共有できるようにする
  - @sile
- [CHANGE] `Mp4Error::InconsistentSampleDescription` から `fields` を削除し、 `InvalidAv1Track` を追加する
  - サンプルエントリーの相違は `index` のみを報告する
  - AV1 track 検証の失敗は文脈入りのメッセージで報告する
  - @sile
- [FIX] MP4 AV1 の `configOBUs` を各 sync sample の先頭に付与するようにする
  - 今までは `configOBUs` を破棄しており、Sequence Header OBU や静的 Metadata OBU が sync sample に含まれない入力では受信側が decode できない payload になっていた
  - `Mp4SampleReader` 初期化時に AV1 track の OBU 列と Sequence Header 一貫性 / RTP packetizer 順序 / random access 条件を検証し、不正な入力は `Mp4Error::InvalidAv1Track` で拒否する
  - `Mp4VideoTrackInfo` に AV1CodecConfigurationRecord 由来の field と `configOBUs` を保持し、sample entry 一貫性検証の比較対象へ含める
  - AV1 required SDP format に `av1C` 由来の `profile` / `level-idx` / `tier` を 10 進文字列で明示し、incoming との照合で profile 完全一致と level / tier 上限を検証する
  - @sile
- [FIX] MP4 の H.264 トラックで `avcC` 由来の `profile-level-id` を SDP capability に反映する
  - 今までは `packetization-mode=1` だけを付けた bare `H264` を広告しており、Main / High Profile の MP4 では実 bitstream と capability の profile / level が食い違っていた
  - `Mp4SampleReader` 初期化時に全 SPS を解析し、`avcC` の profile / constraint / level と SPS の一致、SPS と `avc1` の寸法の一致を検証し、不一致は `Mp4Error::InvalidH264Track` で拒否する
  - `avcC` 由来の profile-level-id を固定 libwebrtc の `kProfilePatterns` と同じ規則で判定し、認識されない profile / level の入力を `Mp4Error::InvalidH264Track` で拒否する
  - H.264 required SDP format に `packetization-mode=1` に加えて `profile-level-id` を明示し、incoming との照合で sub-profile 完全一致と level 下限を検証する
  - sample entry 一貫性検証の比較対象に `avcC` box 全体と抽出後の profile-level-id を含める
  - @sile

### misc

## 2026.1.0

**リリース日**: 2026-08-25
