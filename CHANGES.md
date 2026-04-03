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

- [UPDATE] `shiguredo_webrtc` を 0.146.3 に上げる
  - @sile
- [ADD] Windows に対応する
  - @melpon
- [ADD] `SoraClientBuilder::ice_server_url_configurer` を追加する
  - @melpon
- [ADD] MP4 ファイルからエンコード済み映像をパススルー送信する機能を追加
  - @voluntas, @melpon
- [ADD] Apple 環境で ObjC default VideoCodecFactory を利用する `InternalHwaVideoCodecCapability` を追加する
  - @melpon
- [UPDATE] `VideoCodecCapability::is_supported` のデフォルト実装を設定する
  - @melpon
- [ADD] `VideoCodecCapability::get_supported_formats` を追加し、デフォルト実装も追加する
  - @melpon
- [FIX] 接続失敗時でも `PeerConnection` / `SoraClientContext` の破棄順序を保証するように保持フィールド順を調整する
  - @melpon

### misc

- [ADD] sumomo に --input-mp4 オプションを追加
  - @voluntas, @melpon
