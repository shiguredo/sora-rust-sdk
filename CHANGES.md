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

## 2026.2.0

**リリース日**: 2026-10-10

- [CHANGE] `Mp4Error::InconsistentSampleDescription` から `fields` を削除し、 `InvalidAv1Track` を追加する
  - サンプルエントリーの相違は `index` のみを報告する
  - AV1 track 検証の失敗は文脈入りのメッセージで報告する
  - @sile
- [CHANGE] `Mp4Error::InvalidH264Track` を追加する
  - H.264 track 検証の失敗は文脈入りのメッセージで報告する
  - @sile
- [CHANGE] `Mp4Error` の Display メッセージを英語に統一する
  - 既存 variant の日本語メッセージを英語に置き換える
  - `InvalidAv1Track` の内部メッセージも英語にする
  - @sile
- [CHANGE] `VideoCodecPreference::has_implementation` の引数を実値から参照に変更する
  - 呼び出し側が `VideoCodecImplementation` を clone せずに判定できるように `&VideoCodecImplementation` を受ける
  - @melpon
- [CHANGE] `SoraConnection::run` の戻り値を `Result<DisconnectReason>` に変更する
  - 接続が終了した理由を型付きで取得できる
  - `DisconnectReason` はクライアントからの切断、サーバーの `close`、シグナリングエラー、WebSocket の切断、DataChannel の close、PeerConnection の失敗を区別する
  - @melpon
- [ADD] `SoraConnectionContextConfig` に `environment` を追加する
  - libwebrtc の `Environment` を指定できる
  - @melpon
- [ADD] AudioEncoder / AudioDecoder をユーザー側でカスタマイズ可能にするための音声コーデックフレームワークを追加する
  - `AudioCodecCapability` trait (`src/audio_codec_capability.rs`) を追加する
  - `AudioCodecImplementation` (`src/audio_codec_capability.rs`) を追加する
  - `AudioCodecPreference` / `AudioPreferenceCodec` (`src/audio_codec_preference.rs`) を追加する
  - `validate_audio_codec_preference` (`src/audio_codec_preference.rs`) を追加する
  - `Error::InvalidAudioCodecCapability` / `Error::InvalidAudioCodecPreference` (`src/error.rs`) を追加する
  - `InternalAudioCodecCapability` (`src/audio_codecs/internal.rs`) を追加する
  - `SoraConnectionContextConfig` に `audio_codec_preference` / `audio_codec_capabilities` を追加する。デフォルトは `InternalAudioCodecCapability` のみで、builtin が広告する Opus / G722 / PCMU / PCMA を選択する (ISAC は builtin factory に無く、L16 / multi-channel Opus は広告されないため選択対象にならない)
  - `SoraAudioEncoderFactory` / `SoraAudioDecoderFactory` (`src/audio_codec.rs`) を内部実装として追加する
  - shiguredo_webrtc の `AudioEncoder` / `AudioDecoder` をユーザー注入可能にする upstream API に依存する
  - @melpon
- [ADD] Mp4SampleReader を複数の Mp4VideoCapturer で共有できるようにする
  - @sile
- [ADD] リリース時に sumomo の Linux バイナリを GitHub Release に添付する
  - Ubuntu 24.04 / 26.04 の x86_64 / aarch64 向けバイナリを tag リリース時に添付する
  - @voluntas
- [ADD] sumomo の MP4 パススルーで H.264 の h264_params を自動補完する
  - `--input-mp4` で H.264 を送るとき、avcC 由来の `profile_level_id` を connect に載せる
  - Sora offer と bitstream の不一致による reject を防ぐ
  - @voluntas
- [ADD] `SoraConnectionBuilder` に `degradation_preference` を追加する
  - `shiguredo_webrtc::DegradationPreference` で送信映像の負荷時の品質制御の優先度を指定できる
  - ネゴシエーションのたびに video sender の RTP パラメータへ反映する。映像を送信しない role では設定しない
  - sumomo に `--degradation-preference` を追加する (`maintain_framerate_and_resolution` / `maintain_framerate` / `maintain_resolution` / `balanced`)
  - @melpon
- [ADD] `SoraConnectionBuilder` に `adaptive_ptime` を追加する
  - 音声の適応的パケット化時間 (adaptivePtime) を指定できる
  - ネゴシエーションのたびに audio sender の RTP パラメータへ反映する。音声を送信しない role では設定しない
  - sumomo に `--adaptive-ptime` を追加する (true / false)
  - @melpon
- [ADD] `SoraConnectionEventHandler` に WebRTC の状態変化のコールバックを追加する
  - PeerConnection / ICE / ICE gathering / ネゴシエーション (SignalingState) の状態変化を型付きで受け取れる
  - メソッドは `on_connection_state_change` / `on_ice_connection_state_change` / `on_ice_gathering_state_change` / `on_signaling_state_change`
  - 状態の型は `shiguredo_webrtc` の `SignalingState` / `PeerConnectionState` / `IceConnectionState` / `IceGatheringState` をそのまま使う
  - @melpon
- [ADD] `SoraConnectionBuilder` に `disconnected_grace_period` を追加する
  - 接続確立後に PeerConnection が `Disconnected` のままであることを許容する時間を指定できる
  - 既定値は 10 秒で、`Connected` または `Connecting` へ戻ると経過はリセットされる
  - @melpon
- [UPDATE] shiguredo_webrtc を 0.150.3 から 0.154.1 に更新する
  - libwebrtc を m152 から m154 に更新する
  - 固定 libwebrtc の AV1 / H.264 source audit を m154 の commit で再検証する
  - `RtpEncodingParametersVector::get_mut` が追加されたことに追随する
  - `SignalingState` と `PeerConnectionObserverHandler::on_signaling_change` が追加されたことに追随する
  - `AudioDeviceModule::from_refcounted_ptr` が追加されたことに追随する
  - libwebrtc を m154.8037.1.1 から m154.8037.1.2 に更新する
  - 借用型 `XxxRef` が読み取り専用になり、書き換え用の `XxxRefMut` が追加されたことに追随する
  - `AudioDeviceModuleHandler` が `&mut self` と `AudioTransportPtr` を受け取るようになったことに追随する
  - @voluntas @melpon
- [UPDATE] shiguredo_mp4 を 2026.4.0 から 2026.5.0 に更新する
  - @sile
- [FIX] MP4 パススルーで encode 前に sample が欠落した場合、後続の delta sample を送信しないようにする
  - capturer は再生順を示す `playback_serial` を付与し、encoder は値の不連続を検出した場合、次のキーフレームまで待つ
  - 再生位置は変更せず、送信再開までの時間は入力 MP4 のキーフレーム間隔に依存する
  - @sile
- [FIX] MP4 AV1 の `configOBUs` を各 sync sample の先頭に付与するようにする
  - 今までは `configOBUs` を破棄しており、Sequence Header OBU や静的 Metadata OBU が sync sample に含まれない入力では受信側が decode できない payload になっていた
  - `Mp4SampleReader` 初期化時に AV1 track の OBU 列と Sequence Header 一貫性 / RTP packetizer 順序 / random access 条件を検証し、不正な入力は `Mp4Error::InvalidAv1Track` で拒否する
  - `Mp4VideoTrackInfo` に AV1CodecConfigurationRecord 由来の field と `configOBUs` を保持し、sample entry 一貫性検証の比較対象へ含める
  - AV1 required SDP format に `av1C` 由来の `profile` / `level-idx` / `tier` を 10 進文字列で明示し、incoming との照合で profile 完全一致と level / tier 上限を検証する
  - @sile
- [FIX] MP4 の H.264 トラックで `avcC` 由来の `profile-level-id` を SDP capability に反映する
  - 今までは `packetization-mode=1` だけを付けた bare `H264` を広告しており、Main / High Profile の MP4 では実 bitstream と capability の profile / level が食い違っていた
  - `Mp4SampleReader` 初期化時に全 SPS を解析し、`avcC` の profile / constraint / level と SPS の一致、SPS と `avc1` の寸法の一致を検証し、不一致は `Mp4Error::InvalidH264Track` で拒否する
  - 空の SPS / PPS リストと NAL type 不正の PPS は `Mp4Error::InvalidH264Track` で拒否する
  - `avcC` 由来の profile-level-id を固定 libwebrtc の `kProfilePatterns` と同じ規則で判定し、認識されない profile / level の入力を `Mp4Error::InvalidH264Track` で拒否する
  - H.264 required SDP format に `packetization-mode=1` に加えて `profile-level-id` を明示し、incoming との照合で sub-profile 完全一致と level 下限を検証する
  - sample entry 一貫性検証の比較対象に `avcC` box 全体と抽出後の profile-level-id を含める
  - ISO/IEC 14496-15 に違反するが実在する chroma 拡張欠落の `avcC` は mp4-rs と同様に受理し、再エンコード不能のため `avcc_box` は `None` として扱う
  - @sile
- [FIX] 接続中に Offer の `data_channels` に含まれる DataChannel が閉じた場合に接続を終了する
  - `signaling` 以外の内部ラベルと `#` プレフィックスのユーザー定義ラベルも対象にする
  - @melpon
- [FIX] 接続確立後に PeerConnection のメディア経路が死んだ場合に接続を終了する
  - `PeerConnectionState` が `Failed` になった場合は即座に、`Disconnected` のまま `disconnected_grace_period` を超えた場合も終了する
  - `Connected` または `Connecting` へ戻った場合は接続を維持する
  - 接続確立前の失敗は Sora サーバーの接続タイムアウトに任せる
  - @melpon

### misc

- [UPDATE] SoraConnectionContext の worker thread に network thread を使う
  - 専用 worker thread の生成を削除し、`PeerConnectionFactoryDependencies::set_worker_thread` に network thread を渡す
  - @melpon
- [UPDATE] Rust toolchain を 1.94 に固定し、e2e-tests の rust-version を 1.94 にする
  - e2e-tests が利用する `raden` が MSRV 1.94 を要求するため
  - 公開クレートの MSRV 1.93 は CI の msrv ジョブで検証する
  - @voluntas
- [UPDATE] PBT を proptest から noprop に移行する
  - `pbt/` の依存と `ParsedProxyInfo` のプロパティテストを noprop の Runner API に置き換える
  - @voluntas

## 2026.1.0

**リリース日**: 2026-08-25
