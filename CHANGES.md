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
- [UPDATE] `shiguredo_webrtc` を 0.150.3 に上げ、Ubuntu 26.04 LTS に対応する
  - @voluntas
- [UPDATE] `shiguredo_mp4` を 2026.4.0 に上げる
  - @voluntas
- [FIX] MP4 の `length_size_minus_one` が reserved 値のときに panic せずエラーを返すようにする
  - @voluntas
- [FIX] 非ゼロの composition time offset (B フレーム) を含む MP4 を `Mp4SampleReader` の初期化時に拒否する
  - 今までは composition time offset を無視してデコード順のまま送信していた
  - @sile
- [FIX] sumomo の `--audio false` 指定時に音声トラックが SDP に含まれないようにする
  - @voluntas
- [FIX] MP4 映像の長いフレーム間隔の待機中に `Mp4VideoCapturer` を速やかに停止できるようにする
  - 今までは待機中に停止フラグを確認できず、破棄時に join が長時間停止していた
  - @sile
