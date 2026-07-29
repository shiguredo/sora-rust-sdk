# MP4 の presentation timestamp を保持する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-presentation-timestamps
- Polished: {YYYY-MM-DD}

## 目的

MP4 sample の composition time offset を保持し、decode order と presentation order が異なる映像を正しい時刻で送信する。

## 優先度根拠

High。B frame を含む正規 MP4 で RTP timestamp が表示時刻を表さず、映像順序や音声同期が壊れる。

## 現状

`Mp4SampleReader` は sample の decode timestamp と duration を保存するが、composition time offset を保存しない。
`Mp4VideoCapturer` は MP4 timestamp を利用せず、フレーム送信時の現在時刻から timestamp を生成する。

## 設計方針

- sample ごとに検査済みの presentation timestamp を保持する
- presentation timestamp を WebRTC frame と RTP timestamp へ一貫して反映する
- 負の offset、並び替え、timescale 変換を checked arithmetic で扱う
- reorder を対応対象外にする場合は入力時に明示的に拒否する

## 完了条件

- composition time offset が 0 の既存 MP4 に回帰がない
- B frame を含む MP4 の presentation timestamp が期待値と一致する
- 映像の表示順と音声同期を実ファイルで確認できる
- offset と timescale の境界値テストがある
