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

- [UPDATE] `shiguredo_webrtc` を 0.146.2-canary.1 に上げる
  - @sile
- [ADD] Windows に対応する
  - @melpon
- [ADD] `SoraClientBuilder::ice_server_url_configurer` を追加する
  - @melpon
- [ADD] `sora_sdk` に MP4 パススルー用の `video_codecs::mp4` を追加する
  - @melpon
- [ADD] MP4 ファイルからエンコード済み映像をパススルー送信する機能を追加
  - @voluntas, @melpon

### misc

- [ADD] sumomo に --input-mp4 オプションを追加
  - @voluntas, @melpon