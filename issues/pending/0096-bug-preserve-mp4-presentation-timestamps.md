# MP4 H.264 の presentation timestamp を保持する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-presentation-timestamps
- Polished: 2026-07-29

## 目的

MP4 sample の composition time offset を保持し、decode order と presentation order が異なる映像を decode order のまま正しい表示時刻で送信する。

## 優先度根拠

High。B frame を含む正規 MP4 で RTP timestamp が表示時刻を表さず、受信側で映像の表示順序が壊れる。

## 現状

`Mp4SampleReader` は sample の decode timestamp と duration を保存するが、`shiguredo_mp4::demux::Sample::composition_time_offset` を破棄している。
この `Sample::timestamp` は decode timestamp であり、presentation timestamp は `timestamp + composition_time_offset` である。

`Mp4VideoCapturer` は sample を demux 順、すなわち decode order で読み出し、duration の累積値で pacing する。
一方で `VideoFrame::timestamp_us` は送信時の単調増加時刻から生成し、MP4 の presentation timestamp を WebRTC frame へ渡していない。
`Mp4PassthroughEncoder` も入力 `VideoFrame` の RTP timestamp をそのまま `EncodedImage` へコピーする。

`Mp4PassthroughVideoCodecCapability` は H.264 に `packetization-mode=1` だけを設定し、`profile-level-id` を広告しない。
RFC 6184 Section 8.1 では `profile-level-id` の省略時に Baseline Profile、Level 1 が既定になるが、B slice は Baseline Profile に含まれない。
このため B frame を含む Main / High Profile の MP4 をそのまま送るには、MP4 の AVCDecoderConfigurationRecord と SPS に一致する `profile-level-id` を SDP capability へ反映する必要がある。

本リポジトリが固定する `shiguredo_webrtc` の libwebrtc は `m150.7871.3.1` である。
対応する libwebrtc branch-head の commit `1f975dfd761af6e5d76d28333191973b258d82a8` には次の挙動がある。

- `video/video_stream_encoder.cc` の `VideoStreamEncoder::OnFrame` は、入力 frame の RTP timestamp を `90 * ntp_time_ms` で上書きする
- 同処理は `presentation_timestamp` を保持するが、同一または過去の NTP time を持つ frame を破棄する
- `video/frame_encode_metadata_writer.cc` は、encoder が入力 RTP timestamp を保持しない場合も encoded frame の配送を継続する
  - 出力 timestamp より古い pending input metadata は drop 扱いにし、observer へ `OnFrameDropped` を通知する
  - input metadata と一致しない encoded frame には encode timing metadata を付与せず、reorder または timestamp 非保持を示す警告を出す
- `video/video_stream_encoder.cc` の `VideoStreamEncoder::OnEncodedImage` は、encoder callback が返した `EncodedImage::RtpTimestamp()` を sink へ渡す
- `modules/rtp_rtcp/source/rtp_sender_video.cc` は、渡された RTP timestamp を送出 packet に設定する

したがって入力 `VideoFrame::timestamp_us` と NTP time は libwebrtc に frame を破棄させない単調増加の capture timeline に残し、MP4 presentation timeline から求めた RTP timestamp は passthrough encoder の callback で `EncodedImage` に設定する必要がある。

RTP video timestamp の clock rate は RFC 3551 Section 5 に従い 90,000 Hz とする。
H.264 の non-interleaved mode では RFC 6184 Sections 5.1、6.3 により、RTP sequence number の順序が decode order を表し、RTP timestamp が sampling time を表す。
そのため B frame を presentation order へ並べ替えず、decode order の送信と非単調になり得る presentation timestamp を両立させる。

## 前提

本 issue は issue 0140 で導入する以下の基盤を前提とする。

- `Mp4SampleReader` が required `SdpVideoFormat` を公開し、`Mp4PassthroughVideoCodecCapability::new(&Mp4SampleReader)` で capability を構築する
- reader と capability・各 `Mp4EncodedSample`・encoder handler が private の bitstream identity を `Arc::clone` で共有し、`Mp4PassthroughEncoder` が `Arc::ptr_eq` で照合して不一致サンプルを `VideoCodecStatus::Error` にする
- `Mp4PassthroughVideoCodecCapability::is_supported` が override され、bare `SdpVideoFormat` を `resolve_sdp_format` に通す経路を preference 生成・検証から排除する
- `Mp4SampleReader::new_inner` が全 `sample.sample_entry` を `extract_track_info` に通し、`codec_type` / `width` / `height` / `nal_length_size` / `parameter_sets` / `timescale` の byte-for-byte 一致を検証する
- `SoraVideoEncoderFactory::create` が `resolve_sdp_format` の返り値をそのまま `create_video_encoder` に渡す

加えて、H.264 profile-level-id 抽出と negotiation は issue 0141 で扱う。
B frame 対応の対象 MP4 は Main または High Profile になるため、実 bitstream に一致する `profile-level-id` を SDP に表明する経路（issue 0141）が確立している必要がある。

本 issue はこれらの基盤の上で、B frame の presentation timestamp / decode pacing / capture timeline / RTP timestamp 対応を追加する。

## 設計方針

### 対象範囲

- `src/video_codecs/mp4.rs` の video sample timeline と passthrough encoder を対象とする
- MP4 sample の decode order、payload、loop 内の sample 間 decode pacing は変更しない
  - presentation span が decode duration より長い場合だけ、loop 境界に末尾 gap を入れて次 loop の表示区間との重なりを防ぐ
- 非ゼロの composition time offset と非単調 PTS の送出は H.264 だけを対象とする
  - H.264 以外は全 sample の composition time offset が `None` または `Some(0)` の場合だけ従来どおり受理する
  - H.264 以外に非ゼロの composition time offset があれば、codec 名を含む明示的な unsupported timestamp error で reader 初期化を失敗させる
  - H.265、VP8、VP9、AV1 の decode / presentation order の対応は、各 RTP payload 仕様と実 fixture を用意する別 issue で扱う
- MP4 audio track は現在の capturer が読み出していないため、音声同期の実装と検証は対象外とする
- sample offset、size など presentation timeline と無関係な一般的 MP4 算術強化は issue 0098 で扱う
- 本 issue では signed composition time offset、presentation timestamp の正規化、presentation stride と loop deadline、presentation timestamp から microseconds と 90 kHz RTP timestamp への変換を checked arithmetic の対象にする
- MP4 の edit list は新たに解釈せず、demuxer が返す sample timestamp と composition time offset を入力とする
- H.264 profile-level-id 抽出と `resolve_sdp_format` の H.264 negotiation は issue 0141 で扱う

### sample timeline

reader が返す sample に次の値を別々に保持する。

- decode timestamp: `Sample::timestamp`
- signed composition time offset: `Sample::composition_time_offset.unwrap_or(0)`
- signed presentation timestamp: `decode timestamp + composition time offset`
- duration
- decode order の sample index

decode timestamp を signed の共通中間型へ変換し、composition time offset との加算は checked arithmetic で行う。
全 sample の presentation timestamp の最小値を `presentation_origin` とし、各 sample の正規化済み presentation timestamp を `presentation_timestamp - presentation_origin` とする。
これにより負の composition time offset を保持しながら、`VideoFrame::presentation_timestamp` が要求する非負の `Duration` へ変換できる。
`composition_time_offset == None` と `Some(0)` は同一の timeline として扱う。

sample は presentation timestamp で sort しない。
payload の reader cursor、encoder callback、RTP sequence number は既存の decode order を維持する。
presentation timestamp の非単調性だけを理由に入力を拒否しない。

### decode pacing と capture timeline

既存の duration 累積値による pacing は decode order のまま維持する。
`adapt_frame` に渡す capture timestamp、`VideoFrame::timestamp_us`、NTP time は既存の単調増加 timeline を維持し、MP4 presentation timestamp で置き換えない。
frame drop が発生しても後続 sample の presentation timestamp を詰めず、元の sample index と MP4 timestamp から求める。

reader 初期化時に immutable な timeline plan を構築する。
`plan_frame(loop_index, sample_index)` 相当の pure helper は、decode deadline、capture NTP offset、capture RTP offset、非負の presentation `Duration`、presentation RTP tick を返す。
capturer と passthrough encoder は同じ plan の値を消費し、thread の壁時計や adapter の結果から presentation timeline を再計算しない。

各 `VideoFrame` の `presentation_timestamp` には、plan の非負の presentation `Duration` を設定する。
passthrough sample object にも同じ plan の capture RTP offset と presentation RTP tick を保持する。

capturer は feeder 開始時に millisecond 境界の capture epoch を一度だけ決め、各 frame の `timestamp_us` を `capture epoch + decode deadline`、NTP time を `capture epoch ms + capture NTP offset` として明示的に設定する。
frame ごとに現在時刻を capture timestamp として取り直さず、送信前の pacing により planned timestamp が future にならないようにする。
固定 libwebrtc の `OnFrame` は明示された capture NTP time の 90 倍を入力 RTP timestamp にする。
capture NTP offset は `floor(decode_deadline_us / 1_000)`、capture RTP offset はその 90 倍とする。
固定 libwebrtc は同一または過去の NTP time を持つ frame を drop するため、decode deadline が sample 間と loop 間で同じ millisecond に量子化される入力は feeder 開始前に timestamp error とする。

`Mp4PassthroughEncoder` は frame ごとに、libwebrtc が設定した入力 RTP timestamp から sample の capture RTP offset を wrapping subtraction して session origin を再導出する。
`EncodedImage` の RTP timestamp は、その origin と sample の presentation RTP tick の wrapping addition で設定する。
session origin を encoder handler の mutable state に保持しないため、同一 stream 中の `release` / `init_encode` と handler object の再生成後も、入力 capture timeline から同じ origin を再導出できる。
capture RTP offset と presentation RTP tick はそれぞれの timestamp domain から直接変換し、差分を microseconds 経由で丸め直さない。

固定中の libwebrtc では encoder の出力 RTP timestamp と入力 frame の RTP timestamp が一致しない場合も encoded frame は配送される。
一方、B frame の非単調 PTS では `FrameEncodeMetadataWriter` が一部 input metadata を drop 扱いにし、encoder drop observer の通知と encode timing metadata の欠落が発生し得る。
これは m150 の metadata accounting 上の制約であり、encoded payload 自体の drop ではない。
本 issue は RTP timestamp と受信映像の正しさを対象とし、この metadata accounting を正しい presentation order 対応へ変更しない。
実装コメントと変更履歴には、この制約、固定 commit、該当ファイル、libwebrtc 更新時の再検証条件を記載する。
libwebrtc 更新時は、`VideoStreamEncoder::OnFrame`、`FrameEncodeMetadataWriter`、`VideoStreamEncoder::OnEncodedImage`、`RTPSenderVideo` の挙動を再検証する。

固定中の `shiguredo_webrtc` は `VideoStreamEncoder` / `RTPSenderVideo` の test harness や encoded RTP packet observer を公開していない。
また本リポジトリの組み込み decoder capability は B slice を受理する H.264 format を広告しないため、受信 decoder を含む local PeerConnection test を完了条件にしない。
本 issue では、実 `VideoEncoderEncodedImageCallback` までを自動テストし、その後の固定 libwebrtc 経路は上記 commit の source audit と固定 revision によって保証する。
依存 revision を更新する変更では、同じ source audit を必須にする。

### timescale 変換と RTP timestamp

presentation timeline の変換は pure helper に集約し、sample ごとに壁時計から再計算しない。

- presentation microseconds は `floor(media_time * 1_000_000 / timescale)` とする
- RTP timestamp は `floor(media_time * 90_000 / timescale)` とする
- 中間乗算には変換先より広い整数型と checked arithmetic を使う
- `timescale == 0` は reader 初期化時の timestamp error とする
- RTP timestamp の `u32` wrap は仕様上の modulo `2^32` として許容し、最終変換時だけ明示的に切り詰める
- modulo 前の timeline を固定幅整数の上限まで無制限に増加させず、`timescale * 2^32` に基づく bounded remainder で wrap 後も同じ RTP timestamp を得る

presentation microseconds と RTP timestamp は同じ media-time 値から独立に変換する。
microseconds から 90 kHz へ再変換して二重丸めしない。

### loop epoch

1 loop の presentation stride は、次の大きい方とする。

- decode order の全 sample duration の合計
- `max(normalized presentation timestamp + sample duration)`

これにより presentation span が decode duration を超える入力でも、次 loop の presentation timestamp が前 loop の表示区間へ重ならない。
loop `N` の media time は `N * presentation stride + normalized presentation timestamp` とする。
decode pacing の loop 開始間隔も同じ stride に揃え、presentation span が長い場合は loop 末尾で必要な残り時間を待つ。
loop 境界をまたぐ frame drop があっても loop epoch を再基準化しない。

presentation timestamp の値順は B frame により loop 内で非単調になり得る。
90 kHz 量子化後に複数 frame が同じ RTP timestamp になっても、本 issue では入力を拒否しない。
初期ファイルだけから決まる loop stride、duration 合計、`PTS + duration`、正規化、span は feeder thread の起動前に検証し、失敗時は reader または capturer の構築を timestamp error にする。
`plan_frame` は loop epoch、loop deadline、presentation `Duration`、`shiguredo_webrtc` setter が受理する `i64` microseconds 上限までを checked arithmetic で求める。
無限 loop の長期実行で loop index が表現範囲を超えた場合だけ、panic や飽和を避け、英語の production log を残して feeder thread を正常停止する。

### test fixture

実際に B frame を含み、DTS と PTS が異なる小さな H.264 MP4 fixture を `testdata/` に追加する。
fixture はリポジトリへ commit し、生成に使用した ffmpeg の version と command、H.264 profile、DTS、composition time offset、PTS、timescale をテストコメントへ記録する。
fixture が Main または High Profile になるため、issue 0141 の profile-level-id 抽出とセットで reader が正常初期化できることも確認する。
CI で ffmpeg を起動したり、ネットワークから fixture を取得したりしない。
fixture の payload が decode order であることは demux 結果の sample index と timestamp で検証し、画素の presentation order を外部 decoder の挙動だけに依存して判定しない。

## 変更対象

- `src/video_codecs/mp4.rs`
- `testdata/` の B frame fixture
- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/tests.rs`
- `CHANGES.md`

## 完了条件

- B frame fixture の reader test で、sample payload と index が decode order のまま、DTS、signed composition time offset、PTS、duration が期待値と一致する
- pure timeline helper の table-driven test で次を確認する
  - 負の composition time offset と最小 PTS の正規化
  - `None` と `Some(0)` の同値性
  - decode order では PTS が非単調になる B frame 列
  - timescale から microseconds と 90 kHz への floor 変換
  - microseconds を経由しない RTP timestamp の期待値
  - 異なる PTS が同じ 90 kHz tick になる場合も floor 変換結果を保持すること
  - decode deadline が同じ capture NTP millisecond になる入力の拒否
  - `timescale == 0`、signed 加算、正規化、span、loop epoch の各 overflow の拒否
  - `shiguredo_webrtc` の presentation timestamp setter が受理する `i64` microseconds 上限直前と超過
  - 2 loop 以上の presentation stride と、loop 境界で表示区間が重ならないこと
  - `u32::MAX` をまたぐ RTP timestamp の modulo wrap
- B frame fixture を実 `Mp4PassthroughEncoder` と実 `VideoEncoderEncodedImageCallback` に通し、次を確認する
  - callback の payload と呼出順が decode order と一致する
  - callback 回数が sample 数と一致する
  - `EncodedImage::RtpTimestamp()` が sample object の PTS 由来の期待値と一致する
  - 入力 `VideoFrame` の RTP timestamp を別の値にしても、出力 RTP timestamp が入力値へ戻らない
  - 各入力 RTP timestamp と capture RTP offset から同じ session origin を再導出し、presentation RTP tick と wrap を反映する
  - B frame 列の途中で `release` / `init_encode` した場合と、新しい handler object を生成した場合も、前後の RTP timestamp が同じ MP4 timeline に一致する
  - 異なる media PTS が同じ 90 kHz tick に量子化される 2 sample も別々に callback し、payload、順序、同一 RTP timestamp を保持する
- pure `plan_frame` test で、decode deadline、capture NTP offset、capture RTP offset が単調増加し、presentation `Duration` と presentation RTP tick は非単調な MP4 PTS と loop epoch を反映する
- pure `plan_frame` test で sample index を飛ばし、frame adaptation で一部 sample が drop されても後続 frame の presentation timestamp と RTP timestamp が元の MP4 timeline から詰められない
- `presentation_stride > decode_duration` の table test で、loop 内の sample 間 deadline は DTS duration のまま、loop 末尾だけに差分の gap が入り、次 loop の開始 deadline が presentation stride と一致する
- 実 `VideoFrame` builder を使う wiring test で次を確認する
  - millisecond 境界の capture epoch と plan の decode deadline から capture timestamp を設定する
  - plan の capture NTP offset を frame の NTP time へ明示的に設定する
  - capture timestamp と NTP time が sample 間および loop 間で単調増加する
  - plan の presentation `Duration` が frame に設定される
  - planned capture timestamp を送信時刻より未来にしない
- 固定 libwebrtc commit の source audit により次を確認し、production comment に該当 file / function と revision を記録する
  - `VideoStreamEncoder::OnFrame` が capture NTP time の 90 倍を入力 RTP timestamp にし、presentation timestamp を保持する
  - `FrameEncodeMetadataWriter` の mismatch は metadata drop notification / 欠落を発生させるが、encoded payload の return path を中断しない
  - `VideoStreamEncoder::OnEncodedImage` が callback の RTP timestamp を sink へ渡す
  - `RTPSenderVideo::SendVideo` が各 `EncodedImage` に指定 RTP timestamp を設定し、独立した packetizer を生成する
  - `modules/rtp_rtcp/source/rtp_format_h264.cc` の `RtpPacketizerH264::NextPacket` が access unit の最終 packet に marker を設定する
  - 異なる access unit が同じ 90 kHz tick でも別 callback / marker で区切られるため、reader の入力制限を追加しない
- 既存の composition time offset が 0 の fixture について次の回帰テストが成功する
  - sample の payload、decode order、duration pacing が変わらない
  - `None` と `Some(0)` で presentation timestamp と RTP timestamp が一致する
  - fixture の timescale と duration から求めた既知の RTP timestamp 差分と一致する
- H.265、VP8、VP9、AV1 は、全 sample の composition time offset が 0 の synthetic table では codec 判定を通過し、非ゼロ offset の table では codec 名を含む unsupported timestamp error になる
- B frame fixture の reader、timeline helper、direct encoder unit test は mock / stub、sleep、`#[ignore]`、外部 command、ネットワークを使用しない
- RFC 3551 Section 5 と RFC 6184 Sections 5.1、6.3 を参照し、decode order と presentation timestamp の分離の根拠を production code の日本語コメントに記載する
- libwebrtc commit `1f975dfd761af6e5d76d28333191973b258d82a8` の timestamp 上書き、metadata accounting の制約、callback timestamp 配送、`RtpPacketizerH264::NextPacket` の marker 設定の前提と、更新時の再検証条件を production code の日本語コメントに記載する
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## pending にした理由

- B frame 対応は libwebrtc の `VideoStreamEncoder::OnFrame` が入力 RTP timestamp を `90 * ntp_time_ms` で上書きする制約を回避する必要があり、reader / capturer / encoder の presentation timeline 全体改修を含む大規模な作業になる
- 現時点では B frame 入り MP4 を送信する需要がなく、優先度が低い
- B frame 入力を明確に拒否する対応（非ゼロ composition time offset の検出）は別 issue で先に実施する
- 対応再開時は本 issue の設計方針をそのまま実装の起点にできる
- codec 非依存の reader / capability 結合基盤と sample entry 一貫性検証は issue 0140 に切り出し、AV1 対応（issue 0097）が本 issue の pending に巻き添えで止まらない構成にした
- H.264 profile-level-id 抽出と negotiation は issue 0141 に切り出し、B frame 需要とは独立に SDP capability の正しさを先行して修正できるようにした
