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
- [FIX] 非ゼロの composition time offset (B frame) を含む MP4 を reader 初期化時に拒否する
  - 今までは composition time offset を無視して decode order のまま送信していた
  - @sile
