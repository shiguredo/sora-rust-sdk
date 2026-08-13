# MP4 reader を capability の入力にし、bitstream identity を共有する

- Priority: High
- Created: 2026-08-13
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-mp4-reader-driven-capability
- Polished: {YYYY-MM-DD}

## 目的

`Mp4SampleReader` を capability の唯一の入力にし、reader が確定した required `SdpVideoFormat`、bitstream identity、sample entry 一貫性を capability・preference・encoder handler が共有する経路を作る。
この基盤に H.264 の profile-level-id 対応（issue 0096）と AV1 の configOBUs 対応（issue 0097）を積む。

## 優先度根拠

High。
issue 0097（High）と issue 0096（High）の双方が「reader が公開する required `SdpVideoFormat` と stream identity」「`is_supported` 経路の分離」「全 `sample.sample_entry` の一貫性検証」を前提としており、この基盤が無い状態では codec 固有の対応を単体で入れても目的を達成できない。
基盤を切り出さずに issue 0096 の中で片付けようとすると、B frame timeline 対応と絡んで pending が長引き、issue 0097 まで巻き添えで止まる。

## 現状

`Mp4PassthroughVideoCodecCapability::new` は `VideoCodecType` だけを受け取り、reader の bitstream 実態と切り離されている。
`get_supported_formats(Encoder)` は codec type だけを見て、H.264 は `packetization-mode=1` を付けた `H264`、H.265 / VP8 / VP9 / AV1 は parameter のない bare `SdpVideoFormat` を返す。
`create_video_encoder` は渡された `format` の name を `VideoCodecType` に変換し、reader の codec type と一致するかだけを見る。

`VideoCodecCapability::is_supported` のデフォルト実装は、bare `SdpVideoFormat` を生成して `resolve_sdp_format` に渡し、解決可否で判定する。
`resolve_sdp_format` のデフォルトは `get_supported_formats` との fuzzy match である。
`VideoCodecPreference::new_from_capability` はすでに `capability.is_supported(direction, codec_type)` を経由して preference を生成する。
一方で `validate_video_codec_preference` から呼ばれる `validate_codec` は、`is_supported` の結果を確認した後にさらに `capability.resolve_sdp_format(direction, bare SdpVideoFormat)` の解決可否を独立して検証する。
capability が required parameter を含む format を advertise すると bare 生成では match せず、`is_supported` を override して true を返しても preference validation の bare format 解決検証で拒否されるため、実 encoder factory の format 解決経路と preference 判定経路の分離が崩れる。

`SoraVideoEncoderFactory::create` はすでに `capability.resolve_sdp_format(direction, format)` の返り値を `capability.create_video_encoder(env, resolved.as_ref())` に渡す実装になっている（`SoraVideoDecoderFactory::create` も同様）。
本 issue 時点で factory 経路の code 変更は不要だが、issue 0096 / 0097 が negotiated parameter の保持を本 issue の基盤の上に積むため、この pass-through を回帰テストと production コメントで固定する。

`Mp4SampleReader::new_inner` の while ループは最初のサンプルの `Some(sample_entry)` からだけ `extract_track_info` を呼び、以後の `sample.sample_entry` を無視する。
`shiguredo_mp4::demux::Sample::sample_entry` は sample description が切り替わるサンプルで再度 `Some` を返すが、現行実装は sample description の切り替わりを silently 最初の configuration のまま送り続ける。

`Mp4PassthroughEncoder` は `callback` だけを保持し、入力 `VideoFrame` が正規の reader / capturer 経由の sample かどうかを識別しない。
異なる reader の sample を差し込まれても callback を呼び、`EncodedImage` を組み立ててしまう。
`Mp4EncodedSample` は `pub(crate)` struct のため、external code からは直接参照できない。
本 issue で追加する bitstream identity field は同じ crate 可視性の枠内で保持し、`VideoFrameBuffer::as_native_ref::<Mp4EncodedSample>` は crate 内の `Mp4PassthroughEncoder` からのみ呼ばれる前提を維持する。

## 設計方針

### 対象範囲

- `src/video_codecs/mp4.rs` の `Mp4SampleReader` / `Mp4PassthroughVideoCodecCapability` / `Mp4PassthroughEncoder` / `Mp4VideoCapturer` を対象とする
- 各 codec の bitstream 実態から required parameter を抽出する処理は本 issue で **導入しない**
  - H.264 profile-level-id 抽出と negotiation は issue 0096 に残す
  - AV1 profile / level / tier 抽出と negotiation は issue 0097 で追加する
  - 本 issue の H.264 required format は現行と同じ `packetization-mode=1` のみ、H.265 / VP8 / VP9 / AV1 は現行と同じ bare format
- `resolve_sdp_format` の codec 固有 negotiation は本 issue で追加せず、既存の fuzzy match 挙動を維持する
- B frame の presentation timestamp 対応（composition time offset の保持、sample timeline、loop epoch など）は issue 0096 に残す
- 非ゼロ composition time offset の拒否（現行の `Mp4Error::UnsupportedCompositionTimeOffset`）を本 issue で変更しない
- MP4 audio track は本 issue の対象外とする

### reader が公開する required SDP format

`Mp4SampleReader` に `required_sdp_format() -> SdpVideoFormat` を追加する。
以下の値を reader 構築時に確定させ、以後は immutable に返す。

- H.264: `H264`、`packetization-mode=1`
- H.265: `H265`
- VP8: `VP8`
- VP9: `VP9`
- AV1: `AV1`

いずれも現行の `Mp4PassthroughVideoCodecCapability::get_supported_formats(Encoder)` が返す format と同じ内容とする。
codec 固有 parameter の追加は issue 0096 / 0097 で行い、本 issue では追加しない。

### reader から capability を構築する

`Mp4PassthroughVideoCodecCapability::new(codec_type: VideoCodecType)` を廃止し、`Mp4PassthroughVideoCodecCapability::new(reader: &Mp4SampleReader) -> Self` に置き換える。
capability は内部に以下を保持する。

- reader の `VideoCodecType`
- reader から複製した required `SdpVideoFormat`
- reader と共有する bitstream identity（次節）

`get_supported_formats(Encoder)` は required `SdpVideoFormat` だけを返す。
`Decoder` は現行どおり空を返す。

### bitstream identity

`Mp4SampleReader` は private の zero-sized token 型を `Arc` で 1 度だけ構築し、capability・各 `Mp4EncodedSample`・encoder handler に `Arc::clone` で配布する。
external に露出せず、`Arc::ptr_eq` による同一 reader 判定にだけ使う。
判定を安定させるため、複数 `Arc<()>` の allocation が同じアドレスへ再割り当てされ得るのを避け、`struct Mp4BitstreamIdentity;` のような固有型で wrap する。

`Mp4PassthroughEncoder` は capability から受け取った identity を保持し、`encode` で受け取った `VideoFrame` の buffer が持つ sample の identity を `Arc::ptr_eq` で照合する。

- 一致する場合のみ callback を呼ぶ
- 不一致なら callback を呼ばず、`VideoCodecStatus::Error` を返す
- codec configuration が偶然一致する別 reader の sample も、reader / capability の対応関係が食い違う input として拒否する

reader / capability / capturer は 1 対 1 対 1 で使い、複数 capability を 1 reader から派生させる helper は本 issue で追加しない。

### `is_supported` の override と preference validation の分離

`Mp4PassthroughVideoCodecCapability` は `is_supported(direction, codec_type)` を override し、以下だけを判定する。

- `direction == CodecDirection::Encoder`
- `codec_type == reader.codec_type()`

デフォルト実装の「bare `SdpVideoFormat` を生成して `resolve_sdp_format` に通す」経路を経由しない。
required parameter を持つ format を advertise しても、bare 生成による preference 判定の false 化を回避する。

`VideoCodecPreference::new_from_capability` はすでに `capability.is_supported` を経由するため本 issue で変更しない。
override された `is_supported` の結果がそのまま preference 生成に使われることを test で確認する。

`validate_video_codec_preference` から呼ばれる `validate_codec` は、現状 `is_supported` の判定に加えて `capability.resolve_sdp_format(direction, bare SdpVideoFormat)` の解決可否を独立して検証する。
本 issue でこの追加検証を削除し、`is_supported` の結果を preference validation の source of truth にする。

- `validate_codec` の bare `SdpVideoFormat` を組み立てて `resolve_sdp_format` に渡し、None なら reject する分岐を削除する
- `is_supported == true` かつ `capability` の implementation 名が preference と一致することだけで validation を通過させる
- `codec_capability_summary` の呼び出しなど、削除する分岐に紐づくヘルパーの参照が孤立した場合は同時に整理する
- InternalVideoCodecCapability・InternalAppleVideoCodecCapability など、`is_supported` を override していない既存 capability は現行のデフォルト実装により従来どおり動作するため回帰しないことを test で確認する

これにより bare codec name の `SdpVideoFormat` は preference の生成・検証だけに使われず、実 encoder factory の format 解決経路（`SoraVideoEncoderFactory::create` が negotiated 実 format を `resolve_sdp_format` に渡す経路）と明確に分離する。

### `resolve_sdp_format` と factory 経路

本 issue では `resolve_sdp_format` の実装を変更せず、既存の `get_supported_formats` との fuzzy match 挙動を維持する。
H.264 profile-level-id negotiation は issue 0096 で、AV1 profile / level / tier negotiation は issue 0097 で本 issue の基盤の上に追加する。

`SoraVideoEncoderFactory::create` はすでに `capability.resolve_sdp_format(CodecDirection::Encoder, format)` の返り値を `capability.create_video_encoder(env, resolved.as_ref())` に渡す実装になっている。
本 issue では code 変更を追加せず、この pass-through 挙動を回帰テストで固定し、依存 issue（0096 / 0097）が negotiated parameter の保持を前提にできるよう production コメントで明文化する。
`SoraVideoDecoderFactory::create` も同様の pass-through 挙動になっているが、MP4 passthrough は Decoder 未対応なので Decoder factory は本 issue の回帰対象に含めない。

`Mp4PassthroughVideoCodecCapability::create_video_encoder` は現行の codec type 一致判定に加え、capability が保持する bitstream identity を handler の constructor 引数として渡す。
handler は前節の `Arc::ptr_eq` 判定を行う。

### sample entry の一貫性

`Mp4SampleReader::new_inner` の while ループを次のように変える。

- 最初の `Some(sample_entry)` で `extract_track_info` を呼び、`Mp4VideoTrackInfo` を確定する
- 2 個目以降の `Some(sample_entry)` でも `extract_track_info` を呼び、以下を最初の `Mp4VideoTrackInfo` と field 単位で比較する
  - `codec_type`
  - `width` / `height`
  - `nal_length_size`
  - `parameter_sets` の byte 列（`Option<Vec<u8>>` を byte-for-byte 比較）
  - `timescale` は track 単位で不変のはずだが、変わった場合は保守的に拒否する
- いずれかが変わった場合は sample index と相違項目を含む新設の `Mp4Error::InconsistentSampleDescription` で reader 初期化を失敗させる
- byte-for-byte 完全一致の sample entry の再掲は受理する

`Mp4VideoTrackInfo` の `PartialEq` は本 issue で導入せず、field を直接比較する。
codec 固有 field（H.264 の profile-level-id、AV1 の av1C / configOBUs など）の bit-identical 検証は各 codec 固有 issue で `Mp4VideoTrackInfo` を拡張する形で加える。

### `examples/sumomo`

`Mp4PassthroughVideoCodecCapability::new(codec_type)` を呼んでいる箇所を、reader を先に構築してから `Mp4PassthroughVideoCodecCapability::new(&reader)` を呼び、その後に reader を capturer へ move する順序に更新する。
本 issue では sumomo の他の設定は変更しない。

## 変更対象

- `src/video_codecs/mp4.rs`
- `src/video_codec_preference.rs`（`validate_codec` の bare `SdpVideoFormat` 検証を削除）
- `src/video_codec.rs`（`SoraVideoEncoderFactory::create` の pass-through を production コメントで固定し、必要に応じて回帰テストを追加）
- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/tests.rs`
- `CHANGES.md`

## 完了条件

- `Mp4SampleReader` に `required_sdp_format() -> SdpVideoFormat` があり、H.264 は `packetization-mode=1`、H.265 / VP8 / VP9 / AV1 は bare codec name だけを返す
- `Mp4PassthroughVideoCodecCapability::new` の signature が `&Mp4SampleReader` を受け取る形に変わる
- `Mp4PassthroughVideoCodecCapability::get_supported_formats(Encoder)` の返り値が reader の `required_sdp_format()` と一致する
- `Mp4PassthroughVideoCodecCapability::is_supported` が override され、`Encoder` かつ reader の codec type と一致する場合のみ true を返す test がある
- reader が private の bitstream identity を生成し、capability・各 `Mp4EncodedSample`・encoder handler で `Arc::clone` を共有する
- `Mp4PassthroughEncoder` は入力 `VideoFrame` の sample identity を `Arc::ptr_eq` で照合し、不一致なら callback を呼ばず `VideoCodecStatus::Error` を返す test がある
- codec configuration と codec_type が一致しても異なる reader / capability から生成した sample を渡すと `VideoCodecStatus::Error` になり、callback が呼ばれない test がある
- `Mp4SampleReader::new_inner` は最初の `Some(sample_entry)` 以外にも `extract_track_info` を呼び、`codec_type` / `width` / `height` / `nal_length_size` / `parameter_sets` / `timescale` のいずれかが最初と異なる場合は sample index と相違項目を含む `Mp4Error::InconsistentSampleDescription` で失敗する
- byte-for-byte 同一の sample entry の再掲は受理する合成 fixture / synthetic table test がある
- 2 個目以降の sample entry で `avcC` / SPS / PPS / 解像度 / `nal_length_size` を変えた合成 fixture / synthetic table で reader 初期化が sample index 付き error で失敗する test がある
- `validate_video_codec_preference` から bare `SdpVideoFormat` の `resolve_sdp_format` 解決可否検証が削除され、`is_supported` の結果だけで preference validation が通過することを test で確認する
- `VideoCodecPreference::new_from_capability` と `validate_video_codec_preference` を通す test で、`Mp4PassthroughVideoCodecCapability` から生成した preference が Encoder かつ reader の codec type と一致するエントリを持ち、validation を通過することを確認する
- `InternalVideoCodecCapability`・`InternalAppleVideoCodecCapability` など `is_supported` を override していない既存 capability について、`validate_video_codec_preference` の変更前後で判定結果が変わらないことを test で確認する
- `SoraVideoEncoderFactory::create` が `capability.resolve_sdp_format(direction, format)` の返り値をそのまま `capability.create_video_encoder(env, resolved.as_ref())` に渡す挙動を回帰テストで固定し、production コメントで明文化する
- `examples/sumomo` が reader を先に構築してから `Mp4PassthroughVideoCodecCapability::new(&reader)` で capability を作り、その後に reader を capturer へ move する順序で動く
- 既存の合成 fixture / real fixture の reader test が引き続き成功する
- reader / capability / encoder handler の unit test は mock / stub、sleep、`#[ignore]`、外部 command、ネットワークを使用しない
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[CHANGE]` を追記する（`Mp4PassthroughVideoCodecCapability::new` の signature 変更が破壊的変更のため）
- production log は英語、コメントとテストの assertion message は日本語にする
