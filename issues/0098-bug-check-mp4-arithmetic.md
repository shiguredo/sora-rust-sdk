# MP4 の位置と時刻演算を検査する

- Priority: Medium
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-arithmetic
- Polished: {YYYY-MM-DD}

## 目的

MP4 の入力範囲、sample duration、送信予定時刻の演算を checked arithmetic 化し、悪意ある入力による panic や wraparound を防ぐ。

## 優先度根拠

Medium。ローカル MP4 入力が必要だが、構造上有効な大きな値でも overflow と panic が発生し得る。

## 現状

`Mp4SampleReader::new` は required input の `start + size` と累積 duration のマイクロ秒変換を未検査で計算する。
`Mp4VideoCapturer` は累積 duration を `Instant` へ未検査で加算する。

## 設計方針

- file position と size に `try_from` と `checked_add` を使用する
- duration 変換は十分な幅の整数で計算し、最終型へ検査付き変換する
- `Instant::checked_add` を使用する
- 実用上扱わない時間長には明確な上限とエラーを設ける

## 完了条件

- 各演算が overflow 時に error を返す
- release build でも wraparound しない
- large-size box と大きな duration の境界テストがある
- 通常の MP4 再生 timing に回帰がない
