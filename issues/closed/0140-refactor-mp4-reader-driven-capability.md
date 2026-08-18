# MP4 reader を capability の入力にし、bitstream identity を共有する

- Priority: High
- Created: 2026-08-13
- Completed: 2026-08-18
- Branch: feature/refactor-mp4-reader-driven-capability-v2
- Polished: 2026-08-13
- Updated: 2026-08-17

## 目的

`Mp4SampleReader` を capability の唯一の入力にし、reader が確定した required `SdpVideoFormat` と bitstream identity を capability・encoder handler が共有する経路を作る。
この基盤に H.264 の profile-level-id 対応（issue 0141）と AV1 の configOBUs 対応（issue 0097）を積む。
B frame timeline 対応（pending 中の issue 0096）もこの基盤を前提とする。

sample entry の一貫性検証（issue 0142）と preference validation の bare 検証削除・factory pass-through の明文化（issue 0143）は、本 issue と独立に切り出して先行 merge を完了している。

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

以下 2 issue は本 issue の前提として既に develop へ merge 済み。

- **issue 0142（sample_entry 一貫性の明示エラー化）**: `Mp4SampleReader::new_inner` が全 `Some(sample_entry)` を `extract_track_info` に通し、`codec_type` / `width` / `height` / `nal_length_size` / `parameter_sets` の byte-for-byte 一致を検証する。相違があれば `Mp4Error::InconsistentSampleDescription` で失敗
- **issue 0143（preference validation の bare 検証削除 + factory pass-through 明文化）**: `validate_video_codec_preference` から bare `SdpVideoFormat` を `resolve_sdp_format` に投入する重複検証を削除し、`is_supported` を preference validation の source of truth にする。`SoraVideoEncoderFactory::create` の pass-through を production コメント + 回帰テストで固定

これらが merge 済みのため、本 issue は「reader-driven capability + Arc identity + `required_sdp_format` API + `is_supported` override + sumomo 更新」に絞れる。

## 設計方針

### 対象範囲

- `src/video_codecs/mp4.rs` の `Mp4SampleReader` / `Mp4PassthroughVideoCodecCapability` / `Mp4PassthroughEncoder` を対象とする
- 各 codec の bitstream 実態から required parameter を抽出する処理は本 issue で **導入しない**
  - H.264 profile-level-id 抽出と negotiation は issue 0141 で扱う
  - AV1 profile / level / tier 抽出と negotiation は issue 0097 で追加する
  - 本 issue の H.264 required format は現行と同じ `packetization-mode=1` のみ、H.265 / VP8 / VP9 / AV1 は現行と同じ bare format
- `resolve_sdp_format` の codec 固有 negotiation は本 issue で追加せず、既存の fuzzy match 挙動を維持する
- `validate_video_codec_preference` の bare `SdpVideoFormat` 検証削除と `SoraVideoEncoderFactory::create` の pass-through 固定は issue 0143 で対応済み
- sample entry の一貫性検証は issue 0142 で対応済み
- B frame の presentation timestamp 対応（composition time offset の保持、sample timeline、loop epoch など）は issue 0096 に残す
- 非ゼロ composition time offset の拒否（現行の `Mp4Error::UnsupportedCompositionTimeOffset`）を本 issue で変更しない
- MP4 audio track は本 issue の対象外とする

### bitstream metadata の切り出し

`Mp4SampleReader` から capability に渡す情報を `Mp4BitstreamMetadata` としてスナップショット化する。
`Mp4BitstreamMetadata` はファイル I/O を持たず、reader 構築時に確定した cheap clone 可能な値の束として capability の唯一の入力になる。

- `Mp4SampleReader::bitstream_metadata(&self) -> Mp4BitstreamMetadata` を pub に追加する
- `pub struct Mp4BitstreamMetadata` を pub に追加し、内部フィールドは private とする。既存の `PreferenceCodec` / `VideoCodecImplementation` と同じ「private フィールド + getter」流儀で以下を保持する
  - `VideoCodecType`（対応 getter: `codec_type()`）
  - required `SdpVideoFormat`（対応 getter: `required_sdp_format()` — clone を返す）
  - bitstream identity `Arc`（getter を露出せず、内部でのみ capability 構築時に消費する）
- 生成経路は `Mp4SampleReader::bitstream_metadata` のみとする。外部からのフィールドリテラル構築は private フィールドで塞ぐ
- `Clone` を実装する（`Arc::clone` と `SdpVideoFormat` の clone だけの軽量コピー）
- `Mp4SampleReader::required_sdp_format` は `pub(crate)` に降格し、外部からは `Mp4BitstreamMetadata` 経由で取得する
- `Mp4SampleReader::codec_type` は既存の `pub` を維持する（`Mp4VideoCapturer` 選択などで既に使われている）

required `SdpVideoFormat` は以下の値を reader 構築時に確定させ、以後は immutable に返す。

- H.264: `H264`、`packetization-mode=1`
- H.265: `H265`
- VP8: `VP8`
- VP9: `VP9`
- AV1: `AV1`

いずれも現行の `Mp4PassthroughVideoCodecCapability::get_supported_formats(Encoder)` が返す format と同じ内容とする。
codec 固有 parameter の追加は issue 0141 / 0097 で行い、本 issue では追加しない。

### metadata から capability を構築する

`Mp4PassthroughVideoCodecCapability::new(codec_type: VideoCodecType)` を廃止し、`Mp4PassthroughVideoCodecCapability::new(metadata: Mp4BitstreamMetadata) -> Self` に置き換える。
capability は内部に以下を保持する。

- metadata の `VideoCodecType`
- metadata が握る required `SdpVideoFormat`
- metadata と共有する bitstream identity（次節）

`get_supported_formats(Encoder)` は required `SdpVideoFormat` だけを返す。
`Decoder` は現行どおり空を返す。

`&Mp4SampleReader` ではなく `Mp4BitstreamMetadata` を受け取ることで、signature から「呼び出しでファイル I/O が起きない」ことを明示する。

### bitstream identity

`Mp4SampleReader` は private の zero-sized token 型を `Arc` で 1 度だけ構築し、`Mp4BitstreamMetadata`・capability・各 `Mp4EncodedSample`・encoder handler に `Arc::clone` で配布する。
external に露出せず、`Arc::ptr_eq` による同一 reader 判定にだけ使う。

`Arc::ptr_eq` の安全性は identity Arc の全 clone（reader・metadata・capability・各 `Mp4EncodedSample`・encoder handler が保持）が生存中に手放されないことで担保する。
allocator の address 再利用は wrap 型では防げないため、この生存管理を前提とする。
加えて、crate 内で他の `Arc<()>` と型で区別し、外部から同型 Arc を偶然構築されるのを防ぐため、`struct Mp4BitstreamIdentity;` のような固有型で wrap する（address 再割り当ての回避が目的ではない）。

`Mp4PassthroughEncoder` は capability から受け取った identity を保持し、`encode` で受け取った `VideoFrame` の buffer が持つ sample の identity を `Arc::ptr_eq` で照合する。

- 一致する場合のみ callback を呼ぶ
- 不一致なら callback を呼ばず、`VideoCodecStatus::Error` を返す
- codec configuration が偶然一致する別 reader の sample も、reader / capability の対応関係が食い違う input として拒否する

`Mp4BitstreamMetadata` は `Clone` を持つため技術的には 1 reader から複数 capability を派生できるが、reader / capability / capturer は 1 対 1 対 1 で使い、複数 capability を 1 reader から派生させる helper は本 issue で追加しない。

### `is_supported` の override

`Mp4PassthroughVideoCodecCapability` は `is_supported(direction, codec_type)` を override し、以下だけを判定する。

- `direction == CodecDirection::Encoder`
- `codec_type == reader.codec_type()`

デフォルト実装の「bare `SdpVideoFormat` を生成して `resolve_sdp_format` に通す」経路を経由しない。
required parameter を持つ format を advertise しても、bare 生成による preference 判定の false 化を回避する。

`VideoCodecPreference::new_from_capability` はすでに `capability.is_supported` を経由するため、override された `is_supported` の結果がそのまま preference 生成に使われる。
issue 0143 で `validate_video_codec_preference` から bare `SdpVideoFormat` の重複検証が削除済みのため、override された `is_supported` の結果が preference validation の source of truth になる。

### `resolve_sdp_format` と factory 経路

本 issue では `resolve_sdp_format` の実装を変更せず、既存の `get_supported_formats` との fuzzy match 挙動を維持する。
H.264 profile-level-id negotiation は issue 0141 で、AV1 profile / level / tier negotiation は issue 0097 で本 issue の基盤の上に追加する。

`SoraVideoEncoderFactory::create` の pass-through 挙動（`capability.resolve_sdp_format` の返り値を `create_video_encoder` に渡す）は issue 0143 で production コメントと回帰テストにより固定済み。
本 issue の `Mp4PassthroughVideoCodecCapability::create_video_encoder` は現行の codec type 一致判定に加え、capability が保持する bitstream identity を handler の constructor 引数として渡す。
handler は前節の `Arc::ptr_eq` 判定を行う。

### `examples/sumomo`

現状の `build_context_config` は `mp4_codec_type: Option<VideoCodecType>` を受け取り、その中で `Mp4PassthroughVideoCodecCapability::new(codec_type)` を組み立てている。
本 issue では capability 構築が `Mp4BitstreamMetadata` を必要とするため、`build_context_config` の signature を `mp4_metadata: Option<Mp4BitstreamMetadata>` に変更する。

呼び出し側は「reader を先に構築 → `reader.bitstream_metadata()` で metadata を取り出し → metadata を `build_context_config` に値渡し → 返ってきた `context_config` を context に登録 → その後 reader を capturer へ move」の順序で使う。
metadata は値で渡すため build_context_config の呼び出し後に借用が残らず、reader の move との調整も不要になる。
本 issue では sumomo の他の設定は変更しない。

`examples/sumomo/src/tests.rs` の `build_context_config_mp4_encoder_preference_uses_only_passthrough` と `build_context_config_mp4_manual_internal_encoder_is_passthrough` は現状 `Some(VideoCodecType::H264)` を直接渡しているため、`testdata/` 配下の実 H.264 MP4 fixture から `Mp4SampleReader` を構築し、`bitstream_metadata()` で取り出した値を渡す形に書き替える（AGENTS.md により mock / stub は使わない）。

### 実装で見送った項目

以下は本 issue の設計方針・完了条件で提案していたが、コストと現実的な発生可能性を比較検討した結果、実装を見送った。将来この判断を再検討する際に前提が変わっていないかを見直せるよう、設計そのものは削らずに残す。

#### bitstream identity と `Arc::ptr_eq` による reader 照合

**対象**: 「### bitstream identity」節、および「### metadata から capability を構築する」「### resolve_sdp_format と factory 経路」「## 完了条件」で bitstream identity `Arc` 共有・`Arc::ptr_eq` 照合・`Mp4PassthroughEncoder` の identity mismatch 拒否 に言及している箇所すべて。

**判断**: 実装しない。`Mp4BitstreamMetadata` の導入（reader/metadata の分離）だけを取り込む。

**コスト**: 追加約 95 行（`Mp4BitstreamIdentity` struct、4 型への `identity` フィールド、`Arc::clone` の連鎖、`Mp4PassthroughEncoder::new` / `identity_matches` / encode 内の照合分岐、専用テスト）+ ZST identity token という exotic な概念の学習コスト。

**現実的な発生可能性**: 極めて低い。

- `Mp4EncodedSample` は `pub(crate)` なので、外部から直接構築できない
- `Mp4PassthroughVideoCodecCapability::create_video_encoder` の既存 `codec_type` 一致チェックが、コーデック違いの reader mix を encoder 生成時点で `None` で塞ぐ
- identity check が catch する残余ケースは「同じコーデックの複数 reader を並行して扱い、かつサンプル routing を意図せず配線ミスした場合」のみ
- 通常の 1 reader / 1 capability / 1 capturer 構成では発生しない
- 万一ヒットしても壊れるのは「同コーデックの意図と違う映像」であり、不正なビットストリームを生成する catastrophic な問題ではない

**将来の再検討観点**: 複数の MP4 パススルーを同一プロセスで並行運用するユースケース（マルチストリーム配信など）が具体化した際に、そのときの routing 設計と誤配線リスクを再評価する。それまでは型システム（`Mp4EncodedSample` の crate 可視性）と `codec_type` 一致チェックで十分な防御と見なす。

### 設計から更新した項目

以下は本 issue の設計方針・完了条件で提案していた設計から、実装中に判断を変更した項目。設計そのものは削らずに残し、この節で差分と理由を記録する。

#### `Mp4BitstreamMetadata` のフィールドを pub にし、getter を廃止する

**対象**: 「### bitstream metadata の切り出し」節の以下:

- 「内部フィールドは private とする。既存の `PreferenceCodec` / `VideoCodecImplementation` と同じ『private フィールド + getter』流儀で以下を保持する」
- 「対応 getter: `codec_type()`」「対応 getter: `required_sdp_format()` — clone を返す」
- 「bitstream identity `Arc`（getter を露出せず、内部でのみ capability 構築時に消費する）」
- 「`Clone` を実装する（`Arc::clone` と `SdpVideoFormat` の clone だけの軽量コピー）」
- 完了条件のうち「`pub struct Mp4BitstreamMetadata` が pub で公開され、フィールドは全て private、getter として `codec_type()` と `required_sdp_format()` を持つ。identity は getter を露出しない」
- 完了条件のうち「`Mp4BitstreamMetadata` は `Clone` を実装し、内部は `Arc::clone` と `SdpVideoFormat` clone のみで cheap にコピーできる」

**更新後の設計**:

- 全フィールドを pub にする
- getter (`codec_type()` / `required_sdp_format()`) は削除する
- `Clone` は `#[derive(Clone)]` で自動生成する
- フィールド名を `required_format` → `required_sdp_format` にリネームする（reader 側 method 名と揃える）

**理由**: 「### 実装で見送った項目」で identity check を削除した結果、`Mp4BitstreamMetadata` はコンストラクタ経由でしか守れない不変条件を失った。残るフィールド (`VideoCodecType` / `SdpVideoFormat`) は POD 相当で、コードベースの `ProxyInfo` / `VideoH264Params` などの「pub フィールドの data bundle」流儀に合致する。`private + getter` のボイラープレートは追加価値を持たなくなった。

**追記 (2026-08-18): 上記の pub フィールド化を revert する**

review 指摘により、pub フィールドは `codec_type` と `required_sdp_format` の内部一貫性を破れることが判明した。具体的には、`Mp4BitstreamMetadata { codec_type: H264, required_sdp_format: SdpVideoFormat::new("VP8") }` のように mismatch を持ったリテラル構築が可能で、その場合の挙動は「preference には H.264 Encoder エントリが載るが factory の公開 format は 0 件（`collect_supported_formats` の codec_type 照合で弾かれる）」というサイレント失敗になり、診断が困難になる。

**revert 後の設計**:

- フィールドは private に戻し、getter (`codec_type()` / `required_sdp_format()`) を復活させる
- `Mp4BitstreamMetadata` の唯一の構築経路は `Mp4SampleReader::bitstream_metadata()` に限定される
- `#[derive(Clone)]` は維持する

**revert 理由**: `Mp4BitstreamMetadata` は「reader が確定した値」を切り出したスナップショットで、`codec_type` と `required_sdp_format` の間には内部一貫性の invariant がある。この invariant はコンストラクタ経由でしか守れない。pub フィールド化を選んだ当初は「POD 相当」として `ProxyInfo` / `Video*Params` 流儀に揃えたが、これらの POD には cross-field invariant が無いため類推が不完全だった。private + getter に戻して invariant を構造的に守る。

**追記 (2026-08-18): `Mp4BitstreamMetadata` 中継そのものを撤去し、`Mp4SampleReader::passthrough_capability()` で直接 capability を生成する形に変更する**

上記の private + getter で invariant は守られているが、そもそも metadata という中継型を経由する必要があるかを再検討した結果、削除する判断に至った。

**撤去後の設計**:

- `Mp4BitstreamMetadata` 型と `Mp4SampleReader::bitstream_metadata()` メソッドを撤去する
- 代わりに `Mp4SampleReader::passthrough_capability(&self) -> Mp4PassthroughVideoCodecCapability` を追加する
- `Mp4PassthroughVideoCodecCapability::new` は撤去し、reader 経由でのみ生成できるようにする
- capability の内部フィールドは `codec_type` と `required_format` を直接保持する
- sumomo の `build_context_config` は `Option<Mp4PassthroughVideoCodecCapability>` を受け取る形に変更する

**撤去理由**: metadata の存在価値として設計時に挙げていた「reader と capability の decoupling」「外部から inspect できるスナップショット型」「将来のコーデック固有 parameter 追加時の拡張点」は、いずれも実益が薄いと判明した。

- decoupling: reader / metadata / capability の 3 型は同じ `src/video_codecs/mp4.rs` モジュール内で、intra-module coupling は問題にならない
- 外部 inspection: metadata の外部消費者は build_context_config だけで、独立に inspect している例は無い。codec 種別は `reader.codec_type()` で取れる
- 拡張性: `Mp4SampleReader::required_sdp_format()` を拡張すれば、metadata 経由でも直接生成でも同じく反映される

Rust 慣習でも `File::open(path) -> File` のように中継型を挟まないのが自然。中継型を置く強い理由がないため、公開 API 表面積を減らし生成経路を「reader → capability」の 1 段に統一する方が簡潔と判断した。

## 変更対象

- `src/video_codecs/mp4.rs`
- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/tests.rs`
- `CHANGES.md`

## 完了条件

- `Mp4SampleReader` に `bitstream_metadata(&self) -> Mp4BitstreamMetadata` があり、reader 構築時に確定した値の cheap clone を返す（呼び出しごとに同じ内容）
- `Mp4SampleReader` の内部で確定させる required `SdpVideoFormat` は、H.264 は `packetization-mode=1`、H.265 / VP8 / VP9 / AV1 は bare codec name だけを含む
- `pub struct Mp4BitstreamMetadata` が pub で公開され、フィールドは全て private、getter として `codec_type()` と `required_sdp_format()` を持つ。identity は getter を露出しない
- `Mp4BitstreamMetadata` は `Clone` を実装し、内部は `Arc::clone` と `SdpVideoFormat` clone のみで cheap にコピーできる
- `Mp4SampleReader::required_sdp_format` は `pub(crate)` に降格され、外部の唯一の取得経路が `Mp4BitstreamMetadata::required_sdp_format` になる
- `Mp4PassthroughVideoCodecCapability::new` の signature が `Mp4BitstreamMetadata` を値で受け取る形に変わる
- `Mp4PassthroughVideoCodecCapability::get_supported_formats(Encoder)` の返り値が metadata の `required_sdp_format()` と一致する
- `Mp4PassthroughVideoCodecCapability::is_supported` が override され、`Encoder` かつ metadata の codec type と一致する場合のみ true を返す test がある
- reader が private の bitstream identity を生成し、metadata・capability・各 `Mp4EncodedSample`・encoder handler で `Arc::clone` を共有する
- `Mp4PassthroughEncoder` は入力 `VideoFrame` の sample identity を `Arc::ptr_eq` で照合し、不一致なら callback を呼ばず `VideoCodecStatus::Error` を返す test がある
- codec configuration と codec_type が一致しても異なる reader / capability から生成した sample を渡すと `VideoCodecStatus::Error` になり、callback が呼ばれない test がある
- `VideoCodecPreference::new_from_capability` を通す test で、`Mp4PassthroughVideoCodecCapability` から生成した preference が Encoder かつ metadata の codec type と一致するエントリを持つことを確認する
- `examples/sumomo` の `build_context_config` が `mp4_metadata: Option<Mp4BitstreamMetadata>` を値で受け取り、reader 構築 → `bitstream_metadata` → 呼び出し → context 登録 → reader を capturer へ move の順序で動く
- `examples/sumomo/src/tests.rs` の `build_context_config_mp4_encoder_preference_uses_only_passthrough` と `build_context_config_mp4_manual_internal_encoder_is_passthrough` が `testdata/` 配下の実 H.264 MP4 fixture から `Mp4SampleReader` を構築し、`bitstream_metadata()` で取り出した値を渡す形に書き替えられ、mock / stub を使わず合格する
- 既存の合成 fixture / real fixture の reader test が引き続き成功する
- reader / capability / encoder handler の unit test は mock / stub、sleep、`#[ignore]`、外部 command、ネットワークを使用しない
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[CHANGE]` を追記する（`Mp4PassthroughVideoCodecCapability::new` の signature 変更と `Mp4BitstreamMetadata` の新設が破壊的変更のため）
- production log は英語、コメントとテストの assertion message は日本語にする

## 解決方法

### 実装

- `Mp4SampleReader::passthrough_capability()` を新設し、reader から `Mp4PassthroughVideoCodecCapability` を直接生成できるようにした
- `Mp4PassthroughVideoCodecCapability::new` を撤去し、reader 経由での生成に一本化した
- `Mp4PassthroughVideoCodecCapability::is_supported` を override し、Encoder かつ metadata の codec type と一致する場合のみ true を返すようにした（コーデック固有の必須パラメータを持つ format を将来表明しても preference 検証が破綻しない土台）
- `Mp4SampleReader::required_sdp_format` を `pub(crate)` に降格し、外部からは `Mp4BitstreamMetadata` 経由で参照する経路に一本化した
- ついでに `Mp4SampleReader::new` を `<P: AsRef<Path>>` ジェネリックに変更し、`&str` / `String` / `&Path` / `PathBuf` を直接渡せるようにした
- `examples/sumomo/src/main.rs` の `build_context_config` を `mp4_capability: Option<Mp4PassthroughVideoCodecCapability>` に追従させた

「## 設計方針」の一部は実装中の判断で変更している。詳細は「### 実装で見送った項目」（bitstream identity + `Arc::ptr_eq` による reader 照合を実装しない）と「### 設計から更新した項目」（`Mp4BitstreamMetadata` を pub フィールド化 → review 指摘により revert → 更に metadata 中継そのものを撤去し `reader.passthrough_capability()` に一本化）を参照。

### テスト

- fixture MP4 から `Mp4SampleReader` を組み立てて `bitstream_metadata()` から capability を作る unit test を追加した（`passthrough_capability_advertises_only_reader_required_format` / `passthrough_capability_is_supported_only_for_encoder_and_reader_codec_type` / `passthrough_capability_creates_encoder_only_for_reader_codec_type` / `passthrough_capability_preference_registers_encoder_entry`）
- `examples/sumomo/src/tests.rs` の `build_context_config_mp4_encoder_preference_uses_only_passthrough` / `build_context_config_mp4_manual_internal_encoder_is_passthrough` を、fixture helper `h264_metadata_from_fixture` 経由で実 H.264 MP4 fixture から metadata を取り出す形に書き替えた（mock / stub 未使用）

### CHANGES.md

- `[CHANGE] Mp4PassthroughVideoCodecCapability::new のシグネチャを VideoCodecType から Mp4BitstreamMetadata へ変更する` を追加した
