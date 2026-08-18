# MP4 AV1 の configOBUs を各 sync sample に付与する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-av1-config-obus
- Polished: 2026-07-30
- Updated: 2026-08-18

## 目的

AV1CodecConfigurationBox の `configOBUs` を保持し、AV1 ISOBMFF が定める順序で各 sync sample の前へ付与してから passthrough encoder へ渡す。

## 優先度根拠

High。
現在は `configOBUs` をすべて破棄するため、Sequence Header OBU と併せて sample entry に格納された静的 Metadata OBU などが random access point で送信されない。
AV1 を対応 codec として広告しているにもかかわらず、AV1 ISOBMFF の規範的な bitstream 再構成手順と異なる payload を送信する。

## 現状

`Mp4SampleReader::extract_track_info` の `SampleEntry::Av01` 分岐は `av01.av1c_box.config_obus` を保存しない。
`Mp4SampleReader::get_sample` は AV1 sample data だけを `Mp4EncodedSample` にコピーする。

AV1 Codec ISO Media File Format Binding v1.3.0 Section 2.3.4 は、任意の sync sample から bitstream を構成するとき、AV1CodecConfigurationBox の全 OBU を先に出力し、その後に sample の全 OBU を順番どおり出力するよう定めている。
`configOBUs` と sync sample の双方に同一の Sequence Header OBU がある場合は 2 個を連続して出力し、準拠 decoder はその重複を処理する。
したがって sample 側の Sequence Header OBU を削除したり、`configOBUs` から一部 OBU だけを選別したりしてはならない。

同仕様 Section 2.4 は、sync sample 自体が最初の Frame Header OBU より前に Sequence Header OBU を含むことを要求する。
関連 sample に sync sample が存在しない場合に `configOBUs` の Sequence Header OBU が必須になるが、現在の capturer は track の先頭からループ再生し、AV1ForwardKeyFrameSampleGroupEntry による delayed random access を解釈しない。
そのため `configOBUs` だけに Sequence Header OBU がある no-sync track を「先頭 key frame から再生可能」とは扱えない。

固定する `shiguredo_webrtc` の libwebrtc は `m150.7871.3.1`、対応 commit は `1f975dfd761af6e5d76d28333191973b258d82a8` である。
同 commit の `modules/rtp_rtcp/source/rtp_packetizer_av1.cc` にある `RtpPacketizerAv1::ParseObus` は、encoder callback の payload を Low Overhead Bitstream Format の OBU 列として再解析する。
malformed payload では OBU 列が空になり、packet を生成しない。
また aggregation header の N bit は key frame かつ先頭 OBU が Sequence Header OBU の場合だけ設定する。

## 設計方針

### 対象範囲

- `src/video_codecs/mp4.rs` の AV1 track configuration、AV1 sample framing、passthrough encoder callback payload を対象とする
- AV1 Codec ISO Media File Format Binding v1.3.0 Sections 2.3.4、2.4 と AV1 RTP Payload Format Sections 4.4、4.5 を根拠にする
- AV1 OBU の framing と、AV1CodecConfigurationRecord に必要な Sequence Header field の検証までを対象とする
- 固定 packetizer が operating point の選択・除去を行わないため、Sequence Header が operating point を 1 個だけ持ち、その `operating_point_idc[0] == 0` である bitstream に限定する
- sync sample の random access 条件に必要な先頭 Frame Header の `show_existing_frame`、`frame_type`、`show_frame` だけを検証し、それ以外の Frame Header、Tile Group、Metadata payload の完全な AV1 意味論検証は対象外とする
- AV1ForwardKeyFrameSampleGroupEntry、AV1SwitchFrameSampleGroupEntry、AV1MultiFrameSampleGroupEntry の解釈は別 issue とし、本 issue では no-sync track を拒否する
- MP4 presentation timestamp は issue 0096（pending）で扱う。sample offset と duration の一般的な算術検証は issue 0098（closed 済み）で対応済み

### OBU parser

外部 command や decoder に依存しない pure helper で、Low Overhead Bitstream Format の OBU 列を reader 初期化時に解析する。
parser は OBU ごとに byte range、OBU type、extension header の有無、`obu_has_size_field`、payload range を返す。

共通して次を検証する。

- 空でない OBU header がある
- `obu_forbidden_bit == 0`、`obu_reserved_1bit == 0` である
- `obu_extension_flag == 1` なら extension header があり、その reserved 3 bit が 0 である
- Sequence Header OBU は non-layer-specific であるため、`obu_extension_flag == 0` でなければならない
- LEB128 は AV1 specification Section 4.10.5 に従い、最大 8 byte、8 byte 目の continuation bit は 0、値は `u32::MAX` 以下とし、shift、加算、`usize` 変換を checked arithmetic で行う
- LEB128 の非最短表現は同 Section が許容するため受理する
- 宣言 payload size が残り byte 数以下であり、truncation、overflow、解釈不能な trailing byte を拒否する
- OBU type と byte offset を error に含める

`configOBUs` と sample data では size field の規則が異なるため、parse mode を分ける。

- `configOBUs` は空を許容する
- `configOBUs` 内の全 OBU は `obu_has_size_field == 1` でなければならない
- `configOBUs` の Sequence Header OBU は最大 1 個で、存在する場合は先頭 OBU でなければならない
- `configOBUs` 内の Tile List OBU は固定 libwebrtc の packetizer が送信対象から除外するため、unsupported AV1 RTP OBU error とする
- AV1CodecConfigurationRecord の `seq_profile`、`seq_level_idx_0`、`seq_tier_0`、`high_bitdepth`、`twelve_bit`、`monochrome`、`chroma_subsampling_x`、`chroma_subsampling_y`、`chroma_sample_position` と、`configOBUs` 内 Sequence Header OBU の対応 field を pure bit reader で比較する
- sample data は空を拒否し、最後以外の全 OBU に `obu_has_size_field == 1` を要求する
- sample の最後の OBU だけは size field の省略を許可し、省略時は sample の末尾までを payload とする
- sample 内の Tile List OBU は AV1 Codec ISO Media File Format Binding v1.3.0 Section 2.4 が禁止するため、invalid AV1 sample OBU error とする
- sample の byte 列は書き換えず、parser は検証と OBU 順序の確認にだけ使う

全 AV1 sample を feeder thread の開始前に解析し、malformed OBU は sample index、sample 内 byte offset、理由を含む `Mp4Error` で reader 初期化を失敗させる。
`get_sample` の hot path で初めて framing error を検出したり、固定 libwebrtc の packetizer log だけに失敗を委ねたりしない。

### sync sample と bitstream 再構成

AV1 track に `sample.keyframe == true` の sync sample が 1 件以上あることを reader 初期化時に要求する。
sync sample が 0 件なら、track の先頭および loop 境界からの random access を保証できないため、unsupported AV1 random access error とする。
capturer は sample index 0 から再生と loop 再開を行うため、最初の AV1 sample が sync sample でない track も同じ error で拒否する。

各 sync sample について、sample 自身の OBU 列に Sequence Header OBU があり、最初の Frame Header OBU または Frame OBU より前に現れることを検証する。
`configOBUs` の Sequence Header OBU が存在しても、この sync sample 自身の条件を省略しない。

MP4 の sync flag だけで AV1 random access point を保証しない。
Sequence Header bit reader が返す `reduced_still_picture_header` などの必要な context を用い、各 sync sample の最初の Frame Header OBU または Frame OBU の uncompressed header 冒頭を pure bit reader で解析する。
AV1 Codec ISO Media File Format Binding v1.3.0 Section 2.4 に従い、次をすべて要求する。

- `show_existing_frame == 0`
- `frame_type == KEY_FRAME`
- `show_frame == 1`

`reduced_still_picture_header == 1` で AV1 specification が暗黙に定める値も同じ条件として扱う。
入力不足、条件不一致、最初の frame より前に解釈不能な frame header がある場合は、sample index と OBU offset を含む invalid AV1 sync sample error で reader 初期化を失敗させる。
Frame Header の残りは本 issue で意味解析せず、上記 field を得るために必要な分岐だけを読む。

encoder callback へ渡す AV1 payload は次の byte 列とする。

```text
sync sample:     configOBUs || sample data
non-sync sample: sample data
```

`configOBUs` は全 byte を格納順のまま付与する。
sample data も全 byte を格納順のまま保持する。
同一の Sequence Header OBU が連続しても deduplicate しない。
`configOBUs` が空なら既存 payload を変更しない。

固定 libwebrtc の N bit 設定条件に合わせ、再構成後の各 sync sample について、`RtpPacketizerAv1::ParseObus` が RTP 送信対象に残す最初の OBU が Sequence Header OBU になることを reader 初期化時に要求する。
固定 packetizer は Temporal Delimiter、Tile List、Padding OBU を除去する。
Tile List は前段で拒否するため、`configOBUs || sample data` の先頭から Temporal Delimiter と Padding OBU を読み飛ばした最初の OBU が Sequence Header でなければ unsupported AV1 RTP packetizer order error とする。
Temporal Delimiter / Padding より後に Sequence Header がある入力は受理する。
AV1 ISOBMFF では Sequence Header より前の Metadata OBU も許容されるが、Metadata は固定 packetizer が除去せず N bit を設定できないため、本 issue では明示的に拒否し、仕様外の並べ替えは行わない。

### AV1 SDP format

AV1 RTP Payload Format Section 7.2 では、`profile`、`level-idx`、`tier` の省略値はそれぞれ 0、5、0 である。
現行の bare `AV1` capability はこの既定値を広告するため、AV1CodecConfigurationRecord の値が既定値を超える bitstream をそのまま送ると、受信側が宣言した能力を超える可能性がある。

issue 0140 で導入した reader 由来の required `SdpVideoFormat` 経路（`Mp4SampleReader::required_sdp_format` から `Mp4SampleReader::passthrough_capability()` を通じて `Mp4PassthroughVideoCodecCapability` に伝える）を AV1 にも拡張する。
AV1 required format には AV1CodecConfigurationRecord の `seq_profile`、`seq_level_idx_0`、`seq_tier_0` を 10 進文字列の `profile`、`level-idx`、`tier` parameter として必ず設定し、省略値へ fallback しない。

`Mp4PassthroughVideoCodecCapability` は AV1 reader について required format だけを encoder format として返す。
preference の codec type 利用可否は issue 0140 の `is_supported` override 経路（Encoder かつ reader の codec type と一致する場合のみ true）で判定し、bare `AV1` を実 negotiated format の代用にしない。

なお 0140 で当初提案されていた bitstream identity + `Arc::ptr_eq` による reader 照合は同 issue で実装見送りとなったため、本 issue でも identity 経路は使わない（`Mp4SampleReader::passthrough_capability()` 経由での required format 伝搬だけを利用する）。

`resolve_sdp_format(Encoder, incoming)` は AV1 RTP Payload Format Section 7.2.3 に従い、次を検証する。

- `profile`、`level-idx`、`tier` が省略されている場合は 0、5、0 として解釈する
- parameter は ASCII 10 進整数として parse し、profile は 0 から 2、level-idx は 5 bit の 0 から 31、tier は 0 または 1 だけを受理する
- 固定 libwebrtc の `AV1IsSameProfile` に合わせ、profile は required と incoming の完全一致を要求する
- required bitstream の level と tier が incoming の receiving capability 以下でなければ拒否する
- 互換な incoming format は supported format へ置き換えず、検証済みの parameter を保持して encoder handler へ渡す
- Decoder は未対応のままとし、異なる codec type と malformed parameter を拒否する

固定 libwebrtc commit の `api/video_codecs/av1_profile.cc` にある `ParseSdpForAV1Profile` は profile の省略を 0 と解釈し、`AV1IsSameProfile` は profile の一致だけを判定する。
level と tier の上限は libwebrtc の profile matching に委ねず、SDK の `resolve_sdp_format` で検証する。
固定 dependency の source audit と更新時の再検証条件に、この file と function も含める。

### sample description の一貫性

issue 0142 で導入した全 `sample.sample_entry` の一貫性検証（`Mp4Error::InconsistentSampleDescription` + `collect_mismatched_track_info_fields`）を AV1 に拡張し、次を最初の configuration と比較する。

- codec type、width、height（0142 で既に対象）
- AV1CodecConfigurationRecord の全 field（本 issue で `Mp4VideoTrackInfo` に追加した AV1 固有 field を `collect_mismatched_track_info_fields` の比較対象へ含める）
- `configOBUs` の全 byte（同じく本 issue で追加した field を比較対象へ含める）

後続 sample entry が byte-for-byte 同一なら受理する。
いずれかが変わる場合は、古い `configOBUs` を新しい sample に付与せず、sample index と相違 field を含む `Mp4Error::InconsistentSampleDescription` で reader 初期化を失敗させる。

### Sequence Header の field 検証

AV1 Sequence Header OBU の pure bit reader は AV1CodecConfigurationRecord との比較に必要な field だけを返すが、対象 field へ到達するまでの条件分岐と可変長構造を AV1 specification に従って読み飛ばす。
入力不足、予約値、算術 overflow は invalid AV1 sequence header error とする。
decoder を起動して検証の代用にしない。

AV1 RTP の SDP `level-idx` / `tier` は RTP stream で使用され得る最大値を表すが、AV1CodecConfigurationRecord が直接保持するのは operating point 0 の値だけである。
複数 operating point の能力を過小広告しないため、全 Sequence Header で `operating_points_cnt_minus_1 == 0` かつ `operating_point_idc[0] == 0` を要求する。
それ以外は unsupported AV1 operating points error で reader 初期化を失敗させる。
single operating point では `seq_level_idx_0` / `seq_tier_0` が送信 stream の required SDP 値になる。

全 sample 内の全 Sequence Header OBU について同じ field を抽出し、その sample entry の AV1CodecConfigurationRecord と一致することを確認する。
Sequence Header の存在と Frame Header / Frame OBU より前の配置だけを sync sample 固有の条件とする。

AV1 bitstream specification の Ordering of OBUs は、同一 coded video sequence で繰り返す Sequence Header OBU の内容について、`operating_parameters_info` を除く bit-identical を要求する。
本 issue では例外 field だけを normalize する複雑な比較を導入せず、OBU framing の header と LEB128 size field を除いた Sequence Header payload 全体の byte 一致を保守的に要求する。

- `configOBUs` に Sequence Header があれば、sync / non-sync を問わず全 sample 内の全 Sequence Header payload が config の payload と一致しなければならない
- `configOBUs` に Sequence Header がなければ、各 sync sample の最初の Sequence Header payload を新しい coded video sequence の基準にする
- 次の sync sample の直前までに現れる sync / non-sync sample 内の全 Sequence Header payload は、その coded video sequence の基準と一致しなければならない
- 次の sync sample は新しい coded video sequence を開始するため、異なる coded video sequence 間の Sequence Header payload 全体の一致は要求せず、AV1CodecConfigurationRecord field の一致だけを要求する
- `operating_parameters_info` だけが異なる仕様上有効な入力も本 issue では unsupported とし、内容を normalize したり一方を削除したりしない

不一致は sample index と各 OBU offset を含む inconsistent AV1 sequence header error で reader 初期化を失敗させる。

### 固定 libwebrtc との境界

`Mp4PassthroughEncoder` は再構成済み payload を `EncodedImage` へ byte-for-byte 渡し、RTP aggregation header や OBU element length を SDK 側で生成しない。
RTP packetization は固定 libwebrtc の `RtpPacketizerAv1` に委ねる。

固定 revision の source audit で次を確認し、production comment に file、function、revision、依存更新時の再検証条件を記載する。

- `RtpPacketizerAv1::ParseObus` が Low Overhead Bitstream Format の size field を解釈する
- size field を RTP OBU element length へ変換し、送出 OBU header では `obu_has_size_field` を除去する
- Temporal Delimiter、Tile List、Padding OBU を RTP 送信対象から除外する
- malformed input では packet を生成しない
- key frame で、除去対象 OBU を除いた先頭 OBU が Sequence Header の場合に N bit を設定する
- temporal unit の最終 packet に marker を設定する
- `api/video_codecs/av1_profile.cc` の `ParseSdpForAV1Profile` と `AV1IsSameProfile` が profile の省略と一致を判定し、level / tier の上限を判定しない

固定 libwebrtc の AV1 packetizer test harness は本 crate に公開されていないため、RTP packet byte の直接検証は完了条件にしない。
実 `VideoEncoderEncodedImageCallback` までを自動テストし、その後段は固定 revision の source audit で保証する。

### test fixture

非空の `configOBUs` と、Sequence Header OBU を含む複数の sync sample を持つ小さな AV1 MP4 fixture を `testdata/` に追加する。
fixture はリポジトリへ commit し、生成に使用した encoder / muxer の version と command、AV1CodecConfigurationRecord の期待 field、`configOBUs` の OBU type と byte 列、各 sample の OBU type 列と sync flag をテストコメントへ記録する。
CI で外部 command を起動したり、ネットワークから fixture を取得したりしない。

## 変更対象

- `src/video_codecs/mp4.rs`
- `testdata/` の AV1 fixture
- `CHANGES.md`

## 完了条件

- real AV1 fixture の reader test で、AV1CodecConfigurationRecord の全 field、`configOBUs` の OBU type、byte range、byte 列が期待値と一致する
- real AV1 fixture の全 sample について、sample index、sync flag、sample 単体の OBU type 順序が期待値と一致する
- 各 sync sample の callback payload が `configOBUs || sample data` と byte-for-byte 一致する
- non-sync sample の callback payload が元の sample data と byte-for-byte 一致する
- `configOBUs` と sample の双方に同一 Sequence Header OBU がある fixture で、callback payload に両方が格納順のまま残る
- 空の `configOBUs` では sync / non-sync sample の payload が従来と一致する
- callback の frame type は sync sample で Key、non-sync sample で Delta のまま変わらない
- pure OBU parser の table-driven test で次を確認する
  - config の複数 OBU と sample の複数 OBU
  - sample の最終 OBUだけが size field を省略する入力
  - 空 config の受理と空 sample の拒否
  - forbidden bit、reserved bit、欠落 extension header、extension reserved bit の拒否
  - extension header を持つ Sequence Header OBU の拒否
  - 8 byte 目で終端する LEB128 と非最短表現の受理
  - 8 byte 目の continuation bit、unterminated LEB128、`u32::MAX` 超過、size 算術 overflow、宣言長超過、trailing byte の拒否
  - config 内 OBU の size field 省略と、sample の最終以外の OBU の size field 省略を拒否する
  - config 内の複数 Sequence Header と、先頭以外の Sequence Header を拒否する
  - config と sample の Tile List OBU をそれぞれの error で拒否する
- Sequence Header bit reader の table-driven test で AV1CodecConfigurationRecord の各 field の一致と不一致、truncation、予約値を確認する
- single operating point かつ `operating_point_idc[0] == 0` を受理し、複数 operating point と非 0 の `operating_point_idc[0]` を unsupported AV1 operating points error で拒否する
- sync sample の先頭 Frame Header OBU と Frame OBU の双方で `show_existing_frame == 0`、`frame_type == KEY_FRAME`、`show_frame == 1` を受理する
- `show_existing_frame == 1`、KEY_FRAME 以外、`show_frame == 0`、truncated uncompressed header、必要な Sequence Header context の欠落を invalid AV1 sync sample error で拒否する
- `reduced_still_picture_header == 1` で暗黙に成立する key / shown frame を受理する
- sync sample の Sequence Header がない場合と、最初の Frame Header / Frame OBU より後にある場合を reader 初期化時に拒否する
- sync / non-sync を問わず、各 Sequence Header の field が sample entry と不一致の場合を reader 初期化時に拒否する
- config と任意の sample、または一つの sample 内で Sequence Header payload が異なる場合を OBU offset 付きで拒否する
- non-sync sample 内の Sequence Header についても AV1CodecConfigurationRecord と config Sequence Header との不一致を拒否する
- Sequence Header OBU の size field の LEB128 表現だけが異なり、payload が一致する場合は受理する
- config Sequence Header がない fixture で、各 sync sample から次の sync sample の直前までを一つの coded video sequence とし、途中の non-sync sample にある異なる Sequence Header payload を拒否する
- config Sequence Header がない fixture で、異なる coded video sequence 間の Sequence Header payload 差は AV1CodecConfigurationRecord field が一致すれば受理する
- sync sample が 0 件の track と、sample index 0 が sync sample でない track を unsupported AV1 random access error で拒否する
- 再構成 payload の先頭に Temporal Delimiter / Padding があり、その後の最初の送信対象 OBU が Sequence Header の場合は受理する
- 最初の送信対象 OBU が Metadata または Frame Header / Frame OBU の場合は unsupported AV1 RTP packetizer order error で拒否する
- AV1 format test で次を確認する
  - required encoder format が `av1C` 由来の `profile`、`level-idx`、`tier` を 10 進文字列で明示する
  - parameter 省略を 0、5、0 として解釈し、required profile が 0 以外の場合と required level / tier が既定値を超える場合に拒否する
  - profile 0 から 2、level-idx 0 から 31、tier 0 から 1 の境界値を受理し、負数、範囲外、非 10 進文字列を拒否する
  - profile は required と incoming の完全一致を要求する
  - required 以上の level / tier は受理し、required 未満は拒否する
  - 互換な incoming format の parameter が encoder handler まで保持される
  - bare `AV1` は preference の codec type 判定にだけ使用し、実 format 解決で required parameter を省略しない
- 後続 sample entry の AV1CodecConfigurationRecord または `configOBUs` が変わる場合を、sample index と相違 field を含む error で拒否する
- malformed fixture / synthetic byte table は reader 構築時に失敗し、feeder thread と encoder callback を開始しない
- bounded な input byte 列を対象に、pure OBU parser が成功した場合の OBU range が入力を順序どおり隙間なく被覆し、各 header / payload range が入力範囲内にあり、各 range の元 byte を連結すると入力と一致する property test がある
- direct encoder test は実 `Mp4PassthroughEncoder` と実 `VideoEncoderEncodedImageCallback` を使い、mock / stub、sleep、`#[ignore]`、外部 command、ネットワークを使用しない
- 固定 libwebrtc commit `1f975dfd761af6e5d76d28333191973b258d82a8` の `RtpPacketizerAv1::ParseObus`、aggregation header の N bit、marker 設定、`ParseSdpForAV1Profile`、`AV1IsSameProfile` を source audit し、production code の日本語コメントへ記録する
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
