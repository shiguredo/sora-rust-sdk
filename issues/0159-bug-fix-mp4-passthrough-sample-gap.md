# MP4 パススルーで sample 欠落後の delta sample を抑止する

- Created: 2026-09-15
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-mp4-passthrough-sample-gap
- Polished: {YYYY-MM-DD}

## 目的

MP4 パススルーで圧縮済み sample が encoder へ到達する前に欠落した場合に、参照関係が壊れた可能性がある後続の delta sample を送信しないようにする。

再生位置と再生タイミングは変更せず、自然に次の実キーフレームが到着するまで送信を抑止する最小の安全策に限定する。

## 現状

`src/video_codecs/mp4.rs` の `Mp4VideoCapturer::new` は、MP4 sample ごとに `AdaptedVideoTrackSource::adapt_frame` を呼ぶ。
`applied=false` の場合は、その sample を `Mp4SampleReader::get_sample` で読み出さずに次へ進む。

libwebrtc の `VideoStreamEncoder::OnFrame` と `VideoStreamEncoder::MaybeEncodeVideoFrame` にも、queue overload、congestion window pushback、timestamp の異常、送信停止などによって `Mp4PassthroughEncoder::encode` より前で frame を破棄する経路がある。

通常の raw frame encoder は、実際に encode した frame から codec dependency を構築できる。
MP4 パススルーは圧縮済み sample をそのまま送るため、参照先の sample だけが欠落すると、後続の delta sample をデコードできない可能性がある。

現在の `Mp4EncodedSample` は再生順を示す serial を持たない。
そのため、`Mp4PassthroughEncoder::encode` は encode 前の sample 欠落を検出できず、後続の delta sample をそのまま encoded image callback へ渡す。

Sora の issue 0190 では、VP9 録画の末尾に `P-frame -> timestamp gap -> P-frame` が残り、後続 sample をデコードできない事例を確認した。
この並びは RTP packetize 前に sample 全体が欠落した場合と整合するが、失敗時の sample index ログと RTP capture がないため、直接原因だったとは断定しない。

## 設計方針

### sample の連続性

`Mp4EncodedSample` に capturer ごとの再生順を表す非公開の sample serial を追加する。

capturer は `adapt_frame` の結果にかかわらず、再生対象として処理した sample ごとに serial を進める。
serial は MP4 の loop 境界でも巻き戻さない。
これにより、`adapt_frame` または libwebrtc の encode 前 drop で sample が欠落すると、encoder に到着する serial が不連続になる。

encoder は生成時と `init_encode` のたびにキーフレーム待ち状態から開始する。
`Mp4VideoCapturer` は encoder の接続前から再生を開始するため、最初に到着した sample が GOP の途中の delta sample でも送信しない。

通常状態では、直前に encoded image callback へ渡した sample の serial と、次に到着した sample の serial が連続していることを確認する。
不連続を検出した場合は、欠落した sample が codec dependency に含まれていた可能性があるものとして、保守的にキーフレーム待ち状態へ移行する。
欠落した sample が実際には参照されない場合も delta sample を抑止することは、安全側の動作として許容する。

キーフレーム待ち状態では delta sample を encoded image callback へ渡さない。
自然に到着した実キーフレームを encoded image callback へ渡した時点で待ち状態を解除する。
delta sample の metadata だけをキーフレームへ書き換えてはならない。

### 復帰方法

capturer は再生位置を変更せず、MP4 の通常の再生順と再生タイミングを維持する。
次の sync sample の検索、encoder と capturer 間の command channel、再生時計の rebase、RTP timestamp の独自計算は追加しない。

復帰までの最大時間は入力 MP4 のキーフレーム間隔に依存する。
`docs/INPUT_MP4.md` には、復帰時間を短くするためにキーフレーム間隔を短めにすることを記載する。

### 対象外

本 issue では次を扱わない。

- PLI などによるキーフレーム要求の処理
- capturer の再生位置を次の sync sample へ進める最適化
- sync sample がない入力の新規拒否
- encoded image callback が非 `Ok` を返した場合の復帰
- `has_trusted_rate_controller` と rate-control 方針の変更
- encoded image callback 後の packetizer、pacer、ネットワークで生じる欠落
- Sora の issue 0190 で発生した VP9 デコード失敗の直接原因の確定

## 完了条件

- encoder が最初に delta sample を受け取った場合、実キーフレームまで encoded image callback へ渡さないこと
- 連続する keyframe と delta sample は従来どおり出力されること
- serial の不連続直後が delta sample の場合、その sample と後続の delta sample が出力されないこと
- serial の不連続直後がキーフレームの場合、そのキーフレームから出力を再開すること
- キーフレーム待ち状態で自然に到着した実キーフレームと、その後の連続する delta sample が出力されること
- MP4 の loop 境界でも serial が連続し、誤ってキーフレーム待ちへ移行しないこと
- 複数の capturer と encoder の間で serial とキーフレーム待ち状態を共有しないこと
- delta sample の metadata をキーフレームへ書き換えないこと
- 実 MP4 fixture を使い、初回 delta sample、serial の不連続、実キーフレームでの復帰を確認すること
- モックやスタブを使用しないこと
- Sora の flaky E2E が再現しないことを完了条件にしないこと
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功すること
- `docs/INPUT_MP4.md` と関連 rustdoc に復帰動作とキーフレーム間隔の制約が記載されていること
- `CHANGES.md` の `develop` セクションに `[FIX]` を追記すること

## 変更対象

- `src/video_codecs/mp4.rs` の `Mp4EncodedSample`
- `src/video_codecs/mp4.rs` の `Mp4VideoCapturer::new`
- `src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder`
- `docs/INPUT_MP4.md`
- `CHANGES.md`

## 関連 issue

- sora-rust-sdk の 0157: MP4 パススルーのキーフレーム要求への対応を検討する。
- sora-rust-sdk の 0158: MP4 パススルーの rate-control 契約を調査する。
- Sora の issue 0190: VP9 E2E のデコード失敗と MP4 パススルー固有の sample 欠落経路を調査した。
