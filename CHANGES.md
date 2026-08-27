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

- [FIX] MP4 AV1 の `configOBUs` を各 sync sample の先頭に付与するようにする
  - 今までは `configOBUs` を破棄しており、Sequence Header OBU や静的 Metadata OBU が sync sample に含まれない入力では受信側が decode できない payload になっていた
  - `Mp4SampleReader` 初期化時に AV1 track の OBU 列と Sequence Header 一貫性 / RTP packetizer 順序 / random access 条件を検証し、不正な入力は `Mp4Error::InvalidAv1Track` で拒否する
  - `Mp4VideoTrackInfo` に AV1CodecConfigurationRecord 由来の field と `configOBUs` を保持し、sample entry 一貫性検証の比較対象へ含める
  - AV1 required SDP format に `av1C` 由来の `profile` / `level-idx` / `tier` を 10 進文字列で明示し、incoming との照合で profile 完全一致と level / tier 上限を検証する
  - @sile

### misc

## 2026.1.0

**リリース日**: 2026-08-25
