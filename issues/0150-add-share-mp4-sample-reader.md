# Mp4SampleReader を複数の Mp4VideoCapturer で共有できる仕組みを追加する

- Created: 2026-08-24
- Completed: {YYYY-MM-DD}
- Branch: feature/add-share-mp4-sample-reader
- Polished: 2026-08-26
- Reporter: @voluntas

## 目的

1 つの MP4 ファイルを複数の `Mp4VideoCapturer` で共有し、同じファイルから独立したパススルー映像を N 個の PeerConnection に送れるようにする。

SDK が提供するのは「demux とファイル I/O の共有」と「capturer ごとの独立した再生」である。再生時計の統一や feeder スレッドの一本化は、汎用 SDK の必須機能にはしない。zakuro-rs の `--vcs N` × `--input-mp4` は、同じファイルを多数の仮想クライアントから送る動機の参考であり、その負荷試験専用の API は置かない。

## 現状

`src/video_codecs/mp4.rs` の MP4 パススルーは次の 4 部品で、1 ファイル 1 本の直線である。

- `Mp4SampleReader`: ファイルを開き、demux してサンプル表を持つ。再生時計は持たない。`get_sample` が `read_bytes_at` で seek + 読み出しし、H.264 / H.265 なら Annex B に変換する
- `Mp4VideoCapturer`: reader を move で消費し、専用スレッドで sleep しながら `on_frame` する。公開するのは `video_source()` だけ
- `Mp4PassthroughVideoCodecCapability`: `Mp4SampleReader::passthrough_capability` から作り、encoder だけを登録する
- `Mp4PassthroughEncoder`: `VideoFrame` 内の `Mp4EncodedSample` を `EncodedImage` に載せ替える。実 encode はしない

`Mp4SampleReader` は 1 つの `Mp4VideoCapturer` からしか使えない。

- `Mp4SampleReader::new` はコンストラクタで MP4 ファイル全体を demux し、全サンプルの metadata を `Vec<Mp4SampleMeta>` に展開する
- `Mp4SampleReader` は `Clone` を実装しておらず、`BufReader<File>` を 1 つだけ保持する
- `Mp4VideoCapturer::new(mut reader: Mp4SampleReader)` は reader を消費 (move) するため、1 つの reader から作れる capturer は 1 つに限られる

さらに、`Mp4VideoCapturer::video_source()` が返す単一の `VideoTrackSource` を複数 PeerConnection の encoder に共有すると、同じ native `VideoFrameBuffer` を複数 encoder スレッドが触ることになる。`shiguredo_webrtc` の `api/video_codec_common.rs` では、`VideoFrameBufferHandlerState` インスタンス 1 個ごと (バッファ 1 個ごと) に `callback_thread` を記録し、C callback (`type` / `width` / `height` / `ToI420` / `CropAndScale`) から `assert_video_frame_buffer_handler_thread` を呼ぶ。この検査は `#[cfg(debug_assertions)]` 付きなので、debug ビルドでは次のメッセージで abort する。release では同 assertion は無い。

```
assertion `left == right` failed: video_frame_buffer callback called from multiple threads
```

`Mp4PassthroughEncoder::encode` の `as_native_ref` はこの assertion を呼び出さない。ただし safety コメントは同一実体への同時アクセスを禁止しており、release でも同じ `video_source()` を複数 encoder に配ることは非対応である。SDK 利用者から見ると **「1 つの MP4 パススルー `video_source` を複数 encoder に配れない」** という制約になる。

現状の SDK API では、この制約を回避するには VC の数だけ `Mp4SampleReader::new` と `Mp4VideoCapturer::new` を実行するしかない。副作用として:

- demux コストが N 倍 (`Mp4FileDemuxer::next_sample()` を全サンプル分ループする処理を N 回実行する)
- オープン FD 数が N 個 (各 reader が `BufReader<File>` を保持)
- feeder OS スレッド数が N 個 (各 `Mp4VideoCapturer` が `thread::spawn` する)

`--vcs` が数百のオーダーになると、WebRTC のソケットと追加の MP4 FD がプロセスの FD 上限と競合しやすく、feeder スレッド数も N に比例する。ただし当該 panic 自体は reader を共有しなくても、VC ごとに capturer を分ければ回避できる。reader 共有の性能上の価値は、主に demux の 1 回化と、同じ index をまとめて読む余地である。

### 起票時の報告

起票時の報告として、shiguredo_webrtc 0.150.3 と sora_sdk 2026.1.0-canary.21 の組で、zakuro-rs の `--input-mp4 --vcs 10 --sora-video-codec-type h264` を Linux 上で実行したところ、2 番目の VC の encoder が動き出したタイミングで上記メッセージにより abort した、とある。現行 SDK の `shiguredo_webrtc` は `~0.152.1-canary.0` である。zakuro-rs のソースと実行は本リポジトリ外であり、本 issue の polish では再検証していない。回避手段として VC ごとに `Mp4SampleReader::new` + `Mp4VideoCapturer::new` する形は、現行 SDK の公開 API（reader 非 `Clone`、capturer が move 消費）と一致する。

## 設計方針

共有するのは `Mp4SampleReader` （demux 結果とファイル I/O）である。再生位置・再生時計・sleep・ループ・`adapt_frame` は今どおり各 `Mp4VideoCapturer` が持つ。型名は `Mp4SampleReader` のままにする。スケジューラや Player ではない。

### 公開 API

- `Mp4SampleReader` を `Clone` 可能にする。clone は cheap で、内部状態は `Arc` 等で共有する。再生 cursor は reader に持たせない （index は capturer のループが渡す）
- `get_sample` は公開しない。現状どおりモジュール非公開のまま、内部で `&self` に変える。戻り値の `Mp4EncodedSample` も `pub(crate)` のまま再公開しない
- `Mp4VideoCapturer::new` は引き続き reader を受け取る。共有するときは `reader.clone()` して渡す
- 既存の 1 対 1 (`Mp4SampleReader::new` → `Mp4VideoCapturer::new` → 単一の `video_source()`) は破壊しない。内部が I/O スレッドになっても、利用者から見た手順は同じでよい
- `passthrough_capability()` は共有されたどの clone からでも取れる。codec 登録は今どおり 1 回でよい
- `video_source()` を複数 PeerConnection に配る使い方は、今どおり非対応とする。公開ドキュメントに、debug での abort 条件、capturer を分ける回避策、その場合でも reader は共有できることを書く（後述）
- 対象コーデックは H.264 / H.265 / VP8 / VP9 / AV1 すべて。共有点は `get_sample` より手前なので、初期から限定しない
- `shiguredo_webrtc` の変更はしない

利用イメージ:

```rust
let reader = Mp4SampleReader::new(path)?;
let capturer1 = Mp4VideoCapturer::new(reader.clone())?;
let capturer2 = Mp4VideoCapturer::new(reader.clone())?;
```

ファイルが VC ごとに違う場合は、ファイルごとに reader を作る。今と同じである。

### 内部構造

- `Mp4SampleReader::new` の demux は 1 回だけ行う
- ファイル I/O は reader 内部のスレッド 1 本に限る。今の `BufReader` + `seek` を複数スレッドで奪い合わない。I/O をそのスレッドに閉じれば、位置指定読み出し (`pread`) は必須ではない
- capturer から reader への受け渡しは request / response にする。公開 API に mpsc は出さない。内部の `get_sample(index)` (`&self`) が I/O スレッドへ依頼し、応答を待って返す。複数 capturer スレッドから同時に呼んでよい
- 最後の clone が drop されたら I/O スレッドを止める
- capturer スレッドは今どおり sleep し、`adapt_frame` が `applied=false` なら `get_sample` しない。捨てるフレームまで読まない
- 各 capturer は受け取ったサンプルから **自分の** `VideoFrameBuffer` を作って `on_frame` する。サンプルバイトを `Arc` で共有する場合でも、handler / `VideoFrameBuffer` は capturer ごとに新規作成する。バッファを使い回すと debug では per-buffer スレッド固定 assertion に当たり、release でも同一実体への同時アクセスとして非対応である

この形なら、zakuro が VC ごとに capturer を分ける使い方のまま panic を回避できる。直るのは「同じ `video_source` を複数 encoder に載せる」ことではなく、「1 つの reader から N 個の capturer を作れる」ことである。

### I/O のまとめ方と先読み

汎用の先読み窓は初期実装に入れない。1 capturer は今も deadline 到来後に読んでおり、逐次読みで足りている。

同一 index の連続・同時依頼を 1 回読んで配ること、および直近サンプルの短いキャッシュは、入れてよいが初期実装の必須条件ではない。完了条件にも含めない。I/O を 1 スレッドに直列化するだけだと、同じ index を N 回変換するコストは残る。

sleep 中に次の 1 枚だけ依頼するパイプラインは必須にしない。必要なら後から足せる。

パススルーではキーフレーム以外の欠落がデコードを壊すので、先読みや合流のためにフレームを間引かない。

### スレッド数と FD

- demux: 1 回
- オープン FD: reader 1 つにつき 1 本
- I/O スレッド: reader 1 つにつき 1 本
- capturer の feeder スレッド: capturer 数に比例したまま。本 issue では capturer スレッドをワーカーへまとめない

複数 capturer インスタンスを単一ワーカーで回すこと自体は、`VideoTrackSource` と `VideoFrameBuffer` を capturer ごとに分ければ per-buffer 制約には抵触しない。ただし I/O 待ちや `on_frame` が同じスレッドだと他 capturer の時計が遅れうる。本 issue の対象外とし、今の「capturer ごとに feeder スレッド」を維持する。

### 採用しない方針

- metadata だけを `Arc` し、capturer ごとに `BufReader` を開く分割。利用者には「reader を共有する」ではなく内部構造が漏れる。FD も N のままになる
- 1 つの `Mp4VideoCapturer` から `fork_video_source` し、feeder 1 本・再生時計 1 本で N encoder に配る形。zakuro の同時送信には合うが、開始時刻のずれやファイルが複数の普通の使い方が例外になる。汎用 SDK としては持たない
- reader が購読ごとの再生時計を持ち、unbounded でフレームを push する形。`adapt_frame` で捨てるフレームまで読む、先読みとスケジューラが reader に集まりすぎる。依頼型の方が境界がきれいである
- panic 回避のために `shiguredo_webrtc` の assertion を緩めること。本 issue の範囲外である

### 公開ドキュメントに書くこと

`docs/INPUT_MP4.md` に SDK 向けの節を追加する。sumomo の CLI 制約と混ぜない。あわせて `Mp4VideoCapturer::video_source` と `Mp4SampleReader` の rustdoc からも、同じ制約と回避策へ辿れるようにする。

読者に伝える事実は次の 3 点である。実装時の文言はこれに沿う。

1. **いつ失敗するか**
   同じ `Mp4VideoCapturer` の `video_source()` を、複数の PeerConnection の映像 encoder に渡してはならない。1 つの `VideoTrackSource` が運ぶ native `VideoFrameBuffer` を複数の encoder スレッドが処理することになる。debug ビルド (`debug_assertions` 有効時) では次のメッセージで abort する。release ではこの assertion は無いが、同じ使い方は非対応のままである。

   ```
   assertion `left == right` failed: video_frame_buffer callback called from multiple threads
   ```

2. **回避方法**
   PeerConnection ごとに `Mp4VideoCapturer` を分ける。各 capturer の `video_source()` だけを、その接続の encoder に渡す。1 つの `video_source()` を使い回さない。

3. **reader は共有できる**
   capturer を分けても、同じファイルなら `Mp4SampleReader` は `clone` して共有してよい。demux とファイル読み出しは 1 つの reader にまとまり、再生位置と再生タイミングは capturer ごとに独立する。ファイルが接続ごとに違う場合は、ファイルごとに `Mp4SampleReader::new` する。

掲載するコード例は、公開 API の利用イメージと同じ形にする (`reader.clone()` して capturer を 2 つ作る)。内部の I/O スレッドや request / response は書かない。

## 完了条件

- 1 つの `Mp4SampleReader` を `clone` し、N 個の `Mp4VideoCapturer` を作れる公開 API がある
- debug ビルドで、各 capturer の `video_source()` を別々の PeerConnection の encoder に接続しても `video_frame_buffer callback called from multiple threads` の panic が発生しない
- 共有時のリソースは次のとおりである。実装後に本 issue へ実測または根拠付きで記録する
  - demux 回数: 1
  - オープン FD 数: 共有 reader 1 つにつき 1
  - capturer feeder スレッド数: capturer 数に比例 (削減しない)
- 既存の 1 対 1 (`Mp4SampleReader::new` → `Mp4VideoCapturer::new` → 単一の `VideoTrackSource`) の利用形態は破壊せず引き続き動作する
- `docs/INPUT_MP4.md` の SDK 向け節に、次の 3 点が利用者向けの言葉で書かれている
  - 同じ `video_source()` を複数 encoder に渡してはならない。debug ビルドでは上記メッセージで abort し、release でも非対応である
  - 回避は PeerConnection ごとに `Mp4VideoCapturer` を分けること
  - capturer を分けても、同じファイルの `Mp4SampleReader` は `clone` して共有できる (`reader.clone()` の例付き)
- `Mp4VideoCapturer::video_source` と `Mp4SampleReader` の rustdoc が、この制約と回避策を案内する
- 実 MP4 fixture (`testdata/red-320x320-h264.mp4` など) で、同一 reader から複数 capturer を同時に動かし、それぞれが正常なフレームを供給できることの回帰テストを追加する
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- コメントは日本語、ログメッセージは英語、テストの assertion message は日本語で書く
- モックやスタブは使用しない
- `CHANGES.md` の develop セクションに `[ADD]` エントリを追記する

## 変更対象

- `src/video_codecs/mp4.rs` (`Mp4SampleReader` の共有と I/O スレッド、`get_sample` の request / response、`Mp4VideoCapturer` は clone した reader を受け取る)
- `docs/INPUT_MP4.md` (SDK 向け節: 同じ `video_source` を複数 encoder に渡せないこと、debug での abort メッセージ、capturer 分離による回避、reader の共有)
- `Mp4VideoCapturer::video_source` / `Mp4SampleReader` の rustdoc
- `testdata/` (必要に応じて共有動作確認用の fixture 追加)
- `CHANGES.md`
