# MP4 reader を capability の入力にし、bitstream identity を共有する

- Priority: High
- Created: 2026-08-13
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-mp4-reader-driven-capability
- Polished: 2026-08-13

## 目的

`Mp4SampleReader` を capability の唯一の入力にし、reader が確定した required `SdpVideoFormat` と bitstream identity を capability・encoder handler が共有する経路を作る。
この基盤に H.264 の profile-level-id 対応（issue 0141）と AV1 の configOBUs 対応（issue 0097）を積む。
B frame timeline 対応（pending 中の issue 0096）もこの基盤を前提とする。

sample entry の一貫性検証（issue 0142）と preference validation の bare 検証削除・factory pass-through の明文化（issue 0143）は、本 issue と独立に切り出して先行 merge する構成にした。

## 優先度根拠

High。
issue 0097（High）、issue 0141（Medium）、pending 中の issue 0096（High）のいずれも「reader が公開する required `SdpVideoFormat` と bitstream identity」「`is_supported` 経路の分離」を前提としており、この基盤が無い状態では codec 固有の対応を単体で入れても目的を達成できない。
基盤を issue 0096 の中に留めた場合、B frame timeline 対応の pending が長引くことで issue 0097 と issue 0141 まで巻き添えで止まる。
切り出すことでその連鎖を断つ。

## 現状

`Mp4PassthroughVideoCodecCapability::new` は `VideoCodecType` だけを受け取り、reader の bitstream 実態と切り離されている。
`get_supported_formats(Encoder)` は codec type だけを見て、H.264 は `packetization-mode=1` を付けた `H264`、H.265 / VP8 / VP9 / AV1 は parameter のない bare `SdpVideoFormat` を返す。
`create_video_encoder` は渡された `format` の name を `VideoCodecType` に変換し、reader の codec type と一致するかだけを見る。

`VideoCodecCapability::is_supported` のデフォルト実装は、bare `SdpVideoFormat` を生成して `resolve_sdp_format` に渡し、解決可否で判定する。
`resolve_sdp_format` のデフォルトは `get_supported_formats` との fuzzy match である。
capability が required parameter を含む format を advertise すると bare 生成では match せず、`is_supported` が false を返して preference 生成側から見た「その codec type を送れる capability」の検出が破綻する。
codec 固有 parameter を capability 側で advertise できるようにするため、`Mp4PassthroughVideoCodecCapability` は `is_supported` を override して bare 生成を経由しない経路を持つ必要がある。

`Mp4PassthroughEncoder` は `callback` だけを保持し、入力 `VideoFrame` が正規の reader / capturer 経由の sample かどうかを識別しない。
異なる reader の sample を差し込まれても callback を呼び、`EncodedImage` を組み立ててしまう。
`Mp4EncodedSample` は `pub(crate)` struct のため、external code からは直接参照できない。
本 issue で追加する bitstream identity field は同じ crate 可視性の枠内で保持し、`VideoFrameBuffer::as_native_ref::<Mp4EncodedSample>` は crate 内の `Mp4PassthroughEncoder` からのみ呼ばれる前提を維持する。

## 前提

以下 2 issue を先行して merge する構成とする。

- **issue 0142（sample_entry 一貫性の明示エラー化）**: `Mp4SampleReader::new_inner` が全 `Some(sample_entry)` を `extract_track_info` に通し、`codec_type` / `width` / `height` / `nal_length_size` / `parameter_sets` の byte-for-byte 一致を検証する。相違があれば `Mp4Error::InconsistentSampleDescription` で失敗
- **issue 0143（preference validation の bare 検証削除 + factory pass-through 明文化）**: `validate_video_codec_preference` から bare `SdpVideoFormat` を `resolve_sdp_format` に投入する重複検証を削除し、`is_supported` を preference validation の source of truth にする。`SoraVideoEncoderFactory::create` の pass-through を production コメント + 回帰テストで固定

上記が merge されていれば、本 issue は「reader-driven capability + Arc identity + `required_sdp_format` API + `is_supported` override + sumomo 更新」に絞れる。
0143 が未 merge のまま本 issue を先行 merge しても現行の Mp4 H.264 required format（`packetization-mode=1`）は default `resolve_sdp_format` の fuzzy match を通過するため、実害はない（ただし後発の 0141 / 0097 が preference validation で拒否される）。

## 設計方針

### 対象範囲

- `src/video_codecs/mp4.rs` の `Mp4SampleReader` / `Mp4PassthroughVideoCodecCapability` / `Mp4PassthroughEncoder` を対象とする
- 各 codec の bitstream 実態から required parameter を抽出する処理は本 issue で **導入しない**
  - H.264 profile-level-id 抽出と negotiation は issue 0141 で扱う
  - AV1 profile / level / tier 抽出と negotiation は issue 0097 で追加する
  - 本 issue の H.264 required format は現行と同じ `packetization-mode=1` のみ、H.265 / VP8 / VP9 / AV1 は現行と同じ bare format
- `resolve_sdp_format` の codec 固有 negotiation は本 issue で追加せず、既存の fuzzy match 挙動を維持する
- `validate_video_codec_preference` の bare `SdpVideoFormat` 検証削除と `SoraVideoEncoderFactory::create` の pass-through 固定は issue 0143 で扱う
- sample entry の一貫性検証は issue 0142 で扱う
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
codec 固有 parameter の追加は issue 0141 / 0097 で行い、本 issue では追加しない。

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

`Arc::ptr_eq` の安全性は identity Arc の全 clone（reader・capability・各 `Mp4EncodedSample`・encoder handler が保持）が生存中に手放されないことで担保する。
allocator の address 再利用は wrap 型では防げないため、この生存管理を前提とする。
加えて、crate 内で他の `Arc<()>` と型で区別し、外部から同型 Arc を偶然構築されるのを防ぐため、`struct Mp4BitstreamIdentity;` のような固有型で wrap する（address 再割り当ての回避が目的ではない）。

`Mp4PassthroughEncoder` は capability から受け取った identity を保持し、`encode` で受け取った `VideoFrame` の buffer が持つ sample の identity を `Arc::ptr_eq` で照合する。

- 一致する場合のみ callback を呼ぶ
- 不一致なら callback を呼ばず、`VideoCodecStatus::Error` を返す
- codec configuration が偶然一致する別 reader の sample も、reader / capability の対応関係が食い違う input として拒否する

reader / capability / capturer は 1 対 1 対 1 で使い、複数 capability を 1 reader から派生させる helper は本 issue で追加しない。

### `is_supported` の override

`Mp4PassthroughVideoCodecCapability` は `is_supported(direction, codec_type)` を override し、以下だけを判定する。

- `direction == CodecDirection::Encoder`
- `codec_type == reader.codec_type()`

デフォルト実装の「bare `SdpVideoFormat` を生成して `resolve_sdp_format` に通す」経路を経由しない。
required parameter を持つ format を advertise しても、bare 生成による preference 判定の false 化を回避する。

`VideoCodecPreference::new_from_capability` はすでに `capability.is_supported` を経由するため、override された `is_supported` の結果がそのまま preference 生成に使われる。
issue 0143 で `validate_video_codec_preference` から bare `SdpVideoFormat` の重複検証が削除されているため、override された `is_supported` の結果が preference validation の source of truth になる。
0143 が未 merge の状態でも、Mp4 の H.264 required format（`packetization-mode=1`）は default `resolve_sdp_format` の fuzzy match を通るため本 issue の Mp4 経路自体は動く（ただし後発の 0141 / 0097 が preference validation で拒否される）。

### `resolve_sdp_format` と factory 経路

本 issue では `resolve_sdp_format` の実装を変更せず、既存の `get_supported_formats` との fuzzy match 挙動を維持する。
H.264 profile-level-id negotiation は issue 0141 で、AV1 profile / level / tier negotiation は issue 0097 で本 issue の基盤の上に追加する。

`SoraVideoEncoderFactory::create` の pass-through 挙動（`capability.resolve_sdp_format` の返り値を `create_video_encoder` に渡す）は issue 0143 で production コメントと回帰テストにより固定される。
本 issue の `Mp4PassthroughVideoCodecCapability::create_video_encoder` は現行の codec type 一致判定に加え、capability が保持する bitstream identity を handler の constructor 引数として渡す。
handler は前節の `Arc::ptr_eq` 判定を行う。

### `examples/sumomo`

現状の `build_context_config` は `mp4_codec_type: Option<VideoCodecType>` を受け取り、その中で `Mp4PassthroughVideoCodecCapability::new(codec_type)` を組み立てている。
本 issue では capability 構築が `&Mp4SampleReader` を必要とするため、次のいずれかで書き替える。

- `build_context_config` のシグネチャを `mp4_reader: Option<&Mp4SampleReader>` へ変更し、呼び出し側で reader を先に構築する
- または、MP4 capability 追加ロジックを別関数（例: `add_mp4_passthrough_capability(&mut context_config, &reader)`）へ切り出し、`build_context_config` は MP4 非依存の設定だけを担当する

いずれの場合も、reader を先に構築 → capability を借用で作成 → capability を `context_config` へ登録 → その後 reader を capturer へ move、という順序を守る。
本 issue では sumomo の他の設定は変更しない。

`examples/sumomo/src/tests.rs` の `build_context_config_mp4_encoder_preference_uses_only_passthrough` と `build_context_config_mp4_manual_internal_encoder_is_passthrough` は現状 `Some(VideoCodecType::H264)` を直接渡しているため、`testdata/` 配下の実 H.264 MP4 fixture から `Mp4SampleReader` を構築して渡す形に書き替える（AGENTS.md により mock / stub は使わない）。

## 変更対象

- `src/video_codecs/mp4.rs`
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
- `VideoCodecPreference::new_from_capability` を通す test で、`Mp4PassthroughVideoCodecCapability` から生成した preference が Encoder かつ reader の codec type と一致するエントリを持つことを確認する
- `examples/sumomo` の `build_context_config`（またはそこから切り出した MP4 capability 登録関数）が reader を先に構築してから `Mp4PassthroughVideoCodecCapability::new(&reader)` で capability を作り、その後に reader を capturer へ move する順序で動く
- `examples/sumomo/src/tests.rs` の `build_context_config_mp4_encoder_preference_uses_only_passthrough` と `build_context_config_mp4_manual_internal_encoder_is_passthrough` が `testdata/` 配下の実 H.264 MP4 fixture から `Mp4SampleReader` を構築する形に書き替えられ、mock / stub を使わず合格する
- 既存の合成 fixture / real fixture の reader test が引き続き成功する
- reader / capability / encoder handler の unit test は mock / stub、sleep、`#[ignore]`、外部 command、ネットワークを使用しない
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[CHANGE]` を追記する（`Mp4PassthroughVideoCodecCapability::new` の signature 変更が破壊的変更のため）
- production log は英語、コメントとテストの assertion message は日本語にする
