# 非ゼロ composition time offset を含む MP4 入力を拒否する

- Priority: High
- Created: 2026-08-10
- Completed: 2026-08-10
- Branch: feature/fix-reject-b-frame-input
- Polished: 2026-08-10

## 目的

B frame（非ゼロ composition time offset）を含む MP4 入力を `Mp4SampleReader` の初期化時に拒否し、decode order と表示時刻が異なる映像を壊れた表示時刻のまま送信しないようにする。

## 現状

`Mp4SampleReader::new_inner` は `shiguredo_mp4::demux::Sample::composition_time_offset` を破棄し、sample の decode timestamp と duration だけを保持する。
`Mp4VideoCapturer` は decode order で sample を読み出し、単調増加の壁時計時刻を `VideoFrame` の timestamp に設定して送信するため、B frame を含む MP4 では RTP timestamp が表示時刻を表さず、受信側の映像表示順序が壊れる。

B frame の正しい表示時刻を保証する対応（presentation timestamp の保持と RTP timestamp への反映）は、libwebrtc の `VideoStreamEncoder::OnFrame` が入力 RTP timestamp を NTP time から上書きする制約の回避を含む大規模な作業であり、保留中である。

## 設計方針

- 全コーデック（H.264、H.265、VP8、VP9、AV1）共通で、非ゼロの composition time offset を含む sample を検出する
- `composition_time_offset` が `None` または `Some(0)` の sample だけを受理し、非ゼロの sample があれば reader 初期化時に拒否する
- エラーには sample index と codec 名を含め、ユーザーが原因を特定できるようにする
- 新エラー variant `UnsupportedCompositionTimeOffset` を `Mp4Error` に追加する
- 既存 fixture（ctts なし、DTS == PTS）は従来どおり受理する
- テスト用に B frame を含む小さな H.264 MP4 fixture を `testdata/` に追加する
  - fixture は ffmpeg で生成し、生成コマンドと version、H.264 profile をテストコメントに記録する
  - CI で ffmpeg を起動しない

## 変更対象

- `src/video_codecs/mp4.rs`
- `testdata/` の B frame fixture
- `CHANGES.md`

## 完了条件

- B frame fixture の読み込みが sample index と codec 名を含む `UnsupportedCompositionTimeOffset` エラーで失敗する
- 既存 fixture（ctts なし）が従来どおり読み込める
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 解決方法

- `Mp4Error::UnsupportedCompositionTimeOffset` を追加し、`Mp4SampleReader::new_inner` のサンプルループで非ゼロの composition time offset を含む sample を検出して拒否するようにした
  - エラーには sample index (0 始まり) と codec 名を含め、ユーザーが原因を特定できるようにした
  - `composition_time_offset` が `None` または `Some(0)` の sample は従来どおり受理する
- B フレームを含む H.264 MP4 fixture を ffmpeg 7.1.1 で生成し、`testdata/red-bframe-320x320-h264.mp4` として追加した
  - 生成コマンド、ffmpeg version、H.264 profile、timescale、composition time offset をテストコメントに記録した
- 既存の H.264 fixture も `src/video_codecs/testdata/` からリポジトリルートの `testdata/` へ移動し、`archive-` プレフィックスを外して `red-320x320-h264.mp4` に改名した
- テストを追加した
  - B frame fixture が `UnsupportedCompositionTimeOffset` エラーで拒否されること
  - ctts ボックスの全 offset を 0 にパッチした MP4 が受理されること
- `CHANGES.md` の develop セクションに `[FIX]` を追記した
