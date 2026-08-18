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
- [CHANGE] `Mp4PassthroughVideoCodecCapability` の構築を `Mp4SampleReader::passthrough_capability()` に一本化する
  - MP4 パススルーを利用している既存コードは `Mp4PassthroughVideoCodecCapability::new(codec_type)` を `reader.passthrough_capability()` に書き換える必要がある
  - `Mp4PassthroughVideoCodecCapability::new` は撤去し、reader からの生成だけを外部に公開する
  - 後続の H.264 profile-level-id 対応や AV1 configOBUs 対応など、コーデック固有の必須 SDP parameter を capability から表明する経路の土台になる
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
- [FIX] MP4 の途中でサンプルエントリーが切り替わる入力を `Mp4SampleReader` の初期化時に拒否する
  - 今までは最初のサンプルエントリーだけを採用し、以後の切り替わりを無視して気付かれないまま壊れた映像を送信していた
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

### misc

- [UPDATE] ビデオコーデック preference の可否判定を `is_supported` に一本化する
  - デフォルトの `is_supported` の実体は「コーデック名だけの `SdpVideoFormat` を `resolve_sdp_format` に通せるか」なので、追加の既存検証は既存 capability では結果が一致するだけだった
  - 例えば MP4 パススルーが H.264 の `profile-level-id` などのコーデック固有 parameter を必須とする場合、以下の順序で `is_supported` の override が事実上無効化されていた
    1. capability が `is_supported(Encoder, H264)` の override で「使える」と表明する
    2. capability の `resolve_sdp_format` はコーデック固有 parameter を必須とし、parameter なしの入力を拒否するよう実装されている
    3. しかしこの既存検証はその capability の `resolve_sdp_format` に「H.264（parameter なし）」を渡す
    4. capability が拒否 → preference 検証全体が失敗し、override が無効化される
  - この追加の既存検証を `validate_video_codec_preference` から削除する。これにより後続の MP4 パススルーやコーデック固有 required parameter を持つ capability が preference 検証で拒否されずに済む
  - `SoraVideoEncoderFactory::create` / `SoraVideoDecoderFactory::create` の「解決済み format を create_video_encoder / create_video_decoder に渡す」既存配線を実装コメントと回帰テストで固定する（本体挙動は変えない）
  - @sile
