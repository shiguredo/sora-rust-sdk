# MP4 の AV1 configOBUs を送信 bitstream に反映する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-av1-config-obus
- Polished: {YYYY-MM-DD}

## 目的

AV1CodecConfigurationBox の `configOBUs` を保持し、AV1 sample から仕様に沿った送信 bitstream を構成する。

## 優先度根拠

High。Sequence Header OBU などが `configOBUs` にだけ存在する正規 MP4 を送信すると、decoder が必要な初期情報を受け取れない。

## 現状

`Mp4SampleReader::extract_track_info` は AV1 の parameter information を保持せず、sample data だけをそのまま利用する。
AV1 の `configOBUs` は demuxer から取得可能だが、現在の track information へ保存されない。

## 設計方針

- AV1 の `configOBUs` を track information に保持する
- sync sample の前に必要な OBU を仕様どおり配置する
- sample 側の Sequence Header OBU と重複する場合の扱いを定義する
- malformed OBU を成功として送信しない

## 完了条件

- `configOBUs` にのみ Sequence Header OBU がある MP4 を送信できる
- sample 側にも同じ OBU がある場合に不正な重複を作らない
- AV1 decoder が先頭の key frame から復号できる
- 実 MP4 と境界入力のテストがある
