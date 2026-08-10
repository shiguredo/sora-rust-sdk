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
- [FIX] MP4 の算術 overflow と capturer の停止遅延を解消する
  - 今までは入力範囲・サンプル範囲・累積 duration の演算が未検査で overflow 時に panic または wraparound していた
  - 今までは長いフレーム間隔の待機中に停止シグナルを確認できず、破棄時に join が長時間停止していた
  - 今まではサンプル数に上限がなく、巨大なサンプル数を宣言した MP4 で長時間ループしていた
  - @sile
