# MP4 パススルーでフレーム欠落後に次のキーフレームへ復帰する

- Created: 2026-09-11
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-mp4-passthrough-keyframe-recovery
- Polished: {YYYY-MM-DD}

## 目的

MP4 パススルーで圧縮済み sample の連続性が失われた場合と、libwebrtc から keyframe request を受けた場合に、後続の delta sample をそのまま送らず、次の実在する keyframe から送信を再開する。

再エンコードできない入力でも、参照先を失った delta frame を受信側へ送り続ける状態を防ぐ。

## 現状

`src/video_codecs/mp4.rs` の `Mp4VideoCapturer::new` は、MP4 sample index ごとに `AdaptedVideoTrackSource::adapt_frame` を呼ぶ。
`applied=false` の場合は `Mp4SampleReader::get_sample` を呼ばず、現在の圧縮済み sample を読み飛ばす。

libwebrtc の `VideoStreamEncoder` にも、queue overload や congestion window pushback などにより `Mp4PassthroughEncoder::encode` より前で frame を破棄する経路がある。

通常の raw frame encoder は、実際に encode した frame から codec dependency を構築する。
MP4 パススルーは圧縮済み sample をそのまま送るため、reference frame だけが破棄されると後続の delta frame をデコードできない。

`Mp4EncodedSample` は sample index と loop generation を保持しない。
そのため、`Mp4PassthroughEncoder` は encode 前に sample が欠落しても検出できない。

`Mp4PassthroughEncoder::encode` は `frame_types` を参照せず、MP4 sample の `is_keyframe` だけで出力 frame type を決める。
このため、PLI などによる keyframe request を受けても、ファイル上の次の sample が delta ならそのまま送信する。

`docs/INPUT_MP4.md` にも、再送とキーフレーム要求を無視する制約が記載されている。

Sora の issue 0190 では、VP9 録画の末尾に `P-frame -> timestamp gap -> P-frame` が残り、後続 sample をデコードできない事例を確認した。
失敗時の sample index ログと RTP capture はないため、この事例の直接原因が `adapt_frame` または libwebrtc の encode 前 drop だったことは未確定である。
一方、上記のコード経路が圧縮済み sample の参照関係を壊し得ることはコードから確認できる。

## 設計方針

### sample の連続性

`Mp4EncodedSample` に sample index と loop generation を保持する。

`Mp4PassthroughEncoder` は、直前に出力した sample との index の連続性を確認する。
同じ loop 内の index 欠落と、想定外の loop 遷移を検出した場合は keyframe 待ち状態へ移行する。

keyframe 待ち状態では delta sample に対して encoded image callback を呼ばない。
実在する keyframe を受け取った時点で送信を再開し、待ち状態を解除する。

delta sample の metadata だけを keyframe に書き換えてはならない。

### keyframe request

`Mp4PassthroughEncoder::encode` は `frame_types` を確認する。
keyframe が要求され、現在の sample が delta の場合は keyframe 待ち状態へ移行する。

同じ要求に対して jump を繰り返さないよう、要求は実在する keyframe を出力するまで latch する。

### 次の sync sample への移動

`Mp4SampleReader` は、現在位置より後にある最も近い sync sample index を検索できる metadata を保持する。
現在の loop に後続 keyframe がない場合は、次の loop の先頭側にある最初の keyframe を選ぶ。

encoder と capturer の連携には、capturer の feeder thread が所有する再生位置へ command を送る `std::sync::mpsc` channel を使う。
共有状態を `Mutex` で保護しない。

各 `Mp4EncodedSample` は、その sample を生成した capturer 固有の command sender を内部情報として保持する。
これにより、共有 `Mp4SampleReader` に再生位置を持たせず、`Mp4PassthroughEncoder` から対応する capturer だけへ jump を要求する。

capturer は command を受け取ったら中間の delta sample を読み飛ばし、選択した sync sample を速やかに供給する。
再生 deadline は新しい sample 位置へ rebase する。

`VideoFrame` の timestamp と RTP timestamp は jump の前後で単調増加させる。
MP4 の sample index や PTS をそのまま巻き戻した timestamp として使用しない。

### 既存 API と文書

`Mp4SampleReader` の clone 間では demux 結果と file I/O だけを共有し、再生位置、loop generation、keyframe 待ち状態は capturer / encoder の組ごとに分離する。

既存の `Mp4SampleReader::passthrough_capability` と `Mp4VideoCapturer::new` の利用手順を維持する。
内部の command sender は公開 API に露出させない。

`docs/INPUT_MP4.md` の「再送やキーフレーム要求は無視する」という制約を、新しい復帰動作に合わせて更新する。

## 完了条件

- encode 前に sample index が欠落した場合、次の実在する keyframe まで delta sample が encoded image callback へ渡らないこと
- keyframe request を受けた場合、次の実在する keyframe から送信を再開すること
- 現在の loop に後続 keyframe がない場合、次の loop の最初の keyframe へ移動できること
- delta sample を keyframe として通知しないこと
- jump の前後で `VideoFrame` と RTP の timestamp が単調増加すること
- 複数の `Mp4VideoCapturer` が 1 つの `Mp4SampleReader` を共有しても、再生位置と keyframe 待ち状態が干渉しないこと
- H.264 / H.265 / VP8 / VP9 / AV1 に共通の復帰規則として動作すること
- `docs/INPUT_MP4.md` と関連 rustdoc に新しい復帰動作が記載されていること
- 実 MP4 fixture を使ったテストで sample 欠落、keyframe request、loop 境界を確認すること
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
