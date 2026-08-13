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

- [CHANGE] 最小対応 Rust バージョンを 1.93 に上げる
  - @voluntas
- [CHANGE] Mp4Error の InputPositionOutOfRange と InconsistentSampleTable の file_size フィールドを usize から u64 に変更する
  - @sile
- [UPDATE] `shiguredo_webrtc` を 0.150.3 に上げ、Ubuntu 26.04 LTS に対応する
  - @voluntas
- [UPDATE] `shiguredo_mp4` を 2026.4.0 に上げる
  - @voluntas
- [UPDATE] MP4 読み込みをファイルベースに変更し、ファイル全体をメモリに保持しないようにする
  - @sile
- [FIX] MP4 の `length_size_minus_one` が reserved 値のときに panic せずエラーを返すようにする
  - @voluntas
- [FIX] 非ゼロの composition time offset (B フレーム) を含む MP4 を `Mp4SampleReader` の初期化時に拒否する
  - 今までは composition time offset を無視してデコード順のまま送信していた
  - @sile
- [FIX] MP4 の途中で sample description が切り替わる入力を `Mp4SampleReader` の初期化時に拒否する
  - 今までは最初の sample description だけを採用し、以後の切り替わりを無視して silently 壊れた映像を送信していた
  - `Mp4Error::InconsistentSampleDescription` で相違した field と sample index を返す
  - @sile
- [FIX] sumomo の `--audio false` 指定時に音声トラックが SDP に含まれないようにする
  - @voluntas
- [FIX] MP4 映像の長いフレーム間隔の待機中に `Mp4VideoCapturer` を速やかに停止できるようにする
  - 今までは待機中に停止フラグを確認できず、フレーム間隔が極端に長い入力では破棄時に join が長時間停止する可能性があった
  - @sile
- [FIX] MP4 映像の送信時刻の計算がオーバーフローしたときにパニックしないようにする
  - 今まではフレームの絶対送信時刻の計算が未検査で、尺が極めて長いフレームを含む入力ファイルを処理した場合に、パニックする可能性があった
  - 正常な MP4 ファイルでは、このようなフレームを含むことはまずないが、壊れた MP4 ファイルが渡された場合に備えての防御的な対応を追加した
  - @sile
- [FIX] MP4 の累積再生時間をオーバーフローしない変換で管理するようにする
  - 今まではタイムスケール単位の累積再生時間をマイクロ秒へ事前変換しており、累積再生時間が極めて大きい入力ファイルを処理した場合に、パニックまたはラップアラウンドする可能性があった
  - 正常な MP4 ファイルでは、このような再生時間を含むことはまずないが、壊れた MP4 ファイルが渡された場合に備えての防御的な対応を追加した
  - @sile
