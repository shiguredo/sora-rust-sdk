# MP4 パススルーでフレーム欠落後に次のキーフレームへ復帰する

- Created: 2026-09-11
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-mp4-passthrough-keyframe-recovery
- Polished: 2026-09-14

## 目的

MP4 パススルーで圧縮済み sample の連続性が失われた場合と、libwebrtc から keyframe request を受けた場合に、後続の delta sample をそのまま送らず、次の実在する keyframe から送信を再開する。

再エンコードできない入力でも、参照先を失った delta frame を受信側へ送り続ける状態を防ぐ。

## 現状

`src/video_codecs/mp4.rs` の `Mp4VideoCapturer::new` は、MP4 sample index ごとに `AdaptedVideoTrackSource::adapt_frame` を呼ぶ。
`applied=false` の場合は `Mp4SampleReader::get_sample` を呼ばず、現在の圧縮済み sample を読み飛ばす。

libwebrtc の `VideoStreamEncoder` にも、queue overload や congestion window pushback などにより `Mp4PassthroughEncoder::encode` より前で frame を破棄する経路がある。

通常の raw frame encoder は、実際に encode した frame から codec dependency を構築する。
MP4 パススルーは圧縮済み sample をそのまま送るため、reference frame だけが破棄されると後続の delta frame をデコードできない。

`Mp4EncodedSample` は sample index と、capturer 内の再生順を表す serial を保持しない。
そのため、`Mp4PassthroughEncoder` は encode 前に sample が欠落しても検出できない。

`Mp4PassthroughEncoder::encode` は `frame_types` を参照せず、MP4 sample の `is_keyframe` だけで出力 frame type を決める。
このため、PLI などによる keyframe request を受けても、ファイル上の次の sample が delta ならそのまま送信する。

`docs/INPUT_MP4.md` にも、再送とキーフレーム要求を無視する制約が記載されている。

Sora の issue 0190 では、VP9 録画の末尾に `P-frame -> timestamp gap -> P-frame` が残り、後続 sample をデコードできない事例を確認した。
失敗時の sample index ログと RTP capture はないため、この事例の直接原因が `adapt_frame` または libwebrtc の encode 前 drop だったことは未確定である。
一方、上記のコード経路が圧縮済み sample の参照関係を壊し得ることはコードから確認できる。

## 設計方針

### sample の連続性

`Mp4EncodedSample` に sample index と、capturer ごとに単調増加する sample serial を保持する。
sample index は sync sample の検索とログに使用し、sample serial は encoder に届いた sample の連続性判定に使用する。

capturer は `adapt_frame` の結果にかかわらず、再生対象として処理した sample ごとに serial を進める。
jump で読み飛ばした sample と loop 境界でも serial を巻き戻さない。
これにより、`adapt_frame` と libwebrtc の encode 前 drop のどちらも、encoder に届く serial の欠落として検出できる。

`Mp4PassthroughEncoder` は、encoded image callback が直前に成功した sample の serial を保持する。
通常状態で次に受け取った serial が `last_serial + 1` でなければ、欠落、重複、巻き戻りのいずれかが発生したものとして keyframe 待ち状態へ移行する。
encoded image callback が `ErrorSendFailed` を返した sample は出力済みとして扱わず、keyframe 待ち状態へ移行する。
この場合は、失敗した sample の serial を要求元として jump command を送る。

`Mp4PassthroughEncoder` は生成時と `init_encode` のたびに keyframe 待ち状態から開始する。
`Mp4VideoCapturer` は生成と同時に再生を開始するため、encoder の接続時点で再生位置が GOP の途中にある可能性があり、最初に受け取った delta sample を連続しているとは判断できない。

keyframe 待ち状態では delta sample に対して encoded image callback を呼ばない。
実在する keyframe の encoded image callback が成功した時点で送信を再開し、待ち状態を解除する。

delta sample の metadata だけを keyframe に書き換えてはならない。

### keyframe request

`Mp4PassthroughEncoder::encode` は `frame_types` を確認する。
keyframe が要求され、現在の sample が delta の場合は keyframe 待ち状態へ移行する。

libwebrtc の `VideoStreamEncoder` は、`encode` が非負の status を返すと、keyframe が出力されなかった場合でも次回の `frame_types` を delta に戻す。
そのため、keyframe 要求は実在する keyframe の encoded image callback が成功するまで encoder 内に保持する。

`frame_types` の先頭要素が `Empty` の場合は encoded image callback を呼ばず、`VideoCodecStatus::NoOutput` を返す。
この sample は出力済みとして扱わないため、次に送信を再開するときは実在する keyframe を要求する。

### 次の sync sample への移動

`Mp4SampleReader` は、次に供給する位置以降にある最も近い sync sample index を検索できる metadata を保持する。
現在の loop に未供給の keyframe がない場合は、次の loop の先頭側にある最初の keyframe を選ぶ。
全コーデックで sync sample が 1 件もない MP4 は復帰不能なため、reader の初期化時に拒否する。
AV1 に対する既存の「先頭 sample も sync sample である」という検証は維持する。

encoder と capturer の連携には、capturer の feeder thread が所有する再生位置へ command を送る `std::sync::mpsc` channel を使う。
共有状態を `Mutex` で保護しない。

各 `Mp4EncodedSample` は、その sample を生成した capturer 固有の command sender を内部情報として保持する。
これにより、共有 `Mp4SampleReader` に再生位置を持たせず、`Mp4PassthroughEncoder` から対応する capturer だけへ jump を要求する。

jump command は、要求を発生させた sample serial を保持する。
capturer は、複数の未処理 command がある場合は最も大きい要求元 serial へまとめる。
要求元 serial より後の keyframe をすでに供給している場合は、新たに jump しない。
まだ供給していない場合は、現在の未供給位置から次の sync sample を選ぶ。
これにより、encoder の処理遅延で古くなった command が、すでに供給した keyframe より後へ再生位置を進めることを防ぐ。

encoder は keyframe が必要な状態を latch するが、jump command 自体は一回限りにしない。
keyframe 待ち状態で delta sample を受け取るたびに、その sample の serial を要求元として jump を要求する。
jump 先の keyframe が encode 前に破棄された場合は、後続の delta sample が新しい serial で jump を再要求する。
capturer 側の供給済み keyframe 判定と command の集約により、同じ keyframe を待っている間に jump を繰り返さない。

capturer は command を受け取ったら中間の delta sample を読み飛ばし、選択した sync sample を速やかに供給する。
再生 deadline は新しい sample 位置へ rebase する。

feeder thread の deadline 待機は command receiver の `recv_timeout` を使い、停止確認だけを行う `thread::sleep` にはしない。
これにより、長い sample duration の待機中でも次の定期的な停止確認を待たずに jump command を処理する。

`VideoFrame` の timestamp は jump の前後で厳密に増加させる。
libwebrtc は同一または過去の NTP timestamp を持つ frame を encode 前に破棄するため、連続する frame の timestamp が同じミリ秒にならないようにする。
RTP timestamp は 32 bit の wrap を許容しつつ RTP clock 上で前進させ、jump による巻き戻りを発生させない。
MP4 の sample index や PTS をそのまま巻き戻した timestamp として使用しない。

### 既存 API と文書

`Mp4SampleReader` の clone 間では demux 結果と file I/O だけを共有し、再生位置、sample serial、keyframe 待ち状態は capturer / encoder の組ごとに分離する。

sample serial、再生位置、jump command の処理状態は capturer ごとに分離する。
keyframe 待ち状態と最後に正常出力した serial は encoder ごとに分離する。

既存の `Mp4SampleReader::passthrough_capability` と `Mp4VideoCapturer::new` の利用手順を維持する。
内部の command sender は公開 API に露出させない。

`docs/INPUT_MP4.md` の「再送やキーフレーム要求は無視する」という制約は、RTP 再送と keyframe 要求への応答を分けて記載する。
本対応では keyframe 要求への復帰動作だけを変更し、RTP 再送を MP4 パススルーエンコーダーが処理するとは記載しない。

## 完了条件

- encode 前に sample が欠落して serial が不連続になった場合、次の実在する keyframe まで delta sample が encoded image callback へ渡らないこと
- encoder が GOP の途中から最初の sample を受け取った場合、実在する keyframe まで delta sample が encoded image callback へ渡らないこと
- keyframe request を受けた場合、次の実在する keyframe から送信を再開すること
- jump 先の keyframe が encode 前に欠落した場合、その後の delta sample を送信せず、次の keyframe への jump を再試行できること
- encoder の処理遅延で要求元より後の keyframe が供給済みの場合、古い jump command によってその次の keyframe まで読み飛ばさないこと
- 現在の loop に後続 keyframe がない場合、次の loop の最初の keyframe へ移動できること
- sync sample が 1 件もない MP4 を reader の初期化時に拒否すること
- delta sample を keyframe として通知しないこと
- encoded image callback が失敗した sample を連続性の基準にせず、keyframe 待ち状態へ移行すること
- jump の前後で `VideoFrame` の timestamp が厳密に増加し、RTP timestamp が wrap を含む RTP clock 上で前進すること
- 複数の `Mp4VideoCapturer` が 1 つの `Mp4SampleReader` を共有しても、再生位置と keyframe 待ち状態が干渉しないこと
- H.264 / H.265 / VP8 / VP9 / AV1 に共通の復帰規則として動作すること
- `docs/INPUT_MP4.md` と関連 rustdoc に新しい復帰動作が記載されていること
- 実 MP4 fixture を使ったテストで初回 delta sample、sample 欠落、keyframe request、jump 先 keyframe の欠落、古い jump command、loop 境界を確認すること
- モックやスタブを使用しないこと
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功すること
- `CHANGES.md` の `develop` セクションに `[FIX]` を追記すること

## 変更対象

- `src/video_codecs/mp4.rs` の `Mp4EncodedSample`
- `src/video_codecs/mp4.rs` の `Mp4SampleReader`
- `src/video_codecs/mp4.rs` の `Mp4VideoCapturer`
- `src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder`
- `docs/INPUT_MP4.md`
- `CHANGES.md`

## 関連 issue

- sora-rust-sdk の closed 0150: `Mp4SampleReader` を共有し、再生位置を capturer ごとに分離した。
- sora-rust-sdk の 0152: 1 つの `VideoTrackSource` を複数の PeerConnection で共有する可否を扱う。
- Sora の issue 0190: VP9 E2E のデコード失敗を調査し、MP4 パススルー固有の sample 欠落経路を切り分けた。
