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

- [CHANGE] `VideoCodecPreference` の `find_mut` を非公開にし、`get_or_add` / `has_implementation` と `PreferenceCodec` の `set_implementation` を削除し、`CodecDirection` の `as_label` / `as_str` を `pub(crate)` にする
  - @voluntas
- [UPDATE] `shiguredo_webrtc` を 0.150.3 に上げ、Ubuntu 26.04 LTS に対応する
  - @voluntas
