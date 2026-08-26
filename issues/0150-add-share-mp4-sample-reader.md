# Mp4SampleReader を複数の Mp4VideoCapturer で共有できる仕組みを追加する

- Created: 2026-08-24
- Completed: {YYYY-MM-DD}
- Branch: feature/add-share-mp4-sample-reader
- Polished: {YYYY-MM-DD}
- Reporter: @voluntas

## 目的

1 つの MP4 ファイルを複数の `Mp4VideoCapturer` (または複数 encoder) で共有し、同じ MP4 パススルー映像を N 個の PeerConnection に配信できるようにする。

zakuro-rs の負荷試験 (`--vcs N` × `--input-mp4`) のように、同じ MP4 を N 個の仮想クライアントから同時に送信するユースケースで、demux コスト・オープン FD 数・feeder OS スレッド数が現状 N に比例している状態を SDK 側で解消することが動機。

## 現状

`src/video_codecs/mp4.rs` の以下の設計により、1 つの `Mp4SampleReader` は 1 つの `Mp4VideoCapturer` からしか使えない。

- `Mp4SampleReader::new` はコンストラクタで MP4 ファイル全体を demux し、全サンプルの metadata を `Vec<Mp4SampleMeta>` に展開する
- `Mp4SampleReader` は `Clone` を実装しておらず、`BufReader<File>` を 1 つだけ保持する
- `Mp4VideoCapturer::new(mut reader: Mp4SampleReader)` は reader を消費 (move) するため、1 つの reader から作れる capturer は 1 つに限られる

さらに、`Mp4VideoCapturer::video_source()` が返す単一の `VideoTrackSource` を複数 PeerConnection の encoder に共有すると、`shiguredo_webrtc` 側の per-buffer スレッド固定 assertion (`api/video_codec_common.rs` の `assert_video_frame_buffer_handler_thread` / `VideoFrameBufferHandlerState::callback_thread`) に引っかかり、以下の panic に至る。

```
assertion `left == right` failed: video_frame_buffer callback called from multiple threads
```

この assertion は `VideoFrameBufferHandlerState` インスタンス 1 個ごと (バッファ 1 個ごと) の `callback_thread` に対する検査で、SDK 利用者側から見ると **「1 つの MP4 パススルー video_source を複数 encoder に配れない」** という制約になる。

現状の SDK API では、SDK 利用者はこの制約を回避するために、VC の数だけ `Mp4SampleReader::new` と `Mp4VideoCapturer::new` を実行するしかない。副作用として:

- demux コストが N 倍 (`Mp4FileDemuxer::next_sample()` を全サンプル分ループする処理を N 回実行する)
- オープン FD 数が N 個 (各 reader が `BufReader<File>` を保持)
- feeder OS スレッド数が N 個 (各 `Mp4VideoCapturer` が `thread::spawn` する)

`--vcs` が数百のオーダーになると、`ulimit -n` の既定値 1024 のうち WebRTC のソケットと競合して枯渇したり、プロセスあたりのスレッド上限に触れやすくなる。

### 実測

- shiguredo_webrtc 0.150.3 と sora_sdk 2026.1.0-canary.21 の組で、zakuro-rs の `--input-mp4 --vcs 10 --sora-video-codec-type h264` を Linux 上で実行したところ、2 番目の VC の encoder が動き出したタイミングで上記 panic により abort した
- 同 zakuro-rs リポジトリでは `src/main.rs` の VC spawn ループで VC ごとに `Mp4SampleReader::new` + `Mp4VideoCapturer::new` を実行する per-VC 分離で回避しており、同ファイルの `TODO(sora_sdk)` コメントに本 issue と同趣旨の SDK 側改善希望が記載されている

## 設計方針

以下 2 系統の解決策が考えられる。どちらを採用するかは次回の方針確認で決める。

### 案 1: メタデータ共有 + サンプルストリームの個別発行

- `Mp4SampleReader` を「MP4 の metadata (`track_info` / `samples` / `cumulative`)」と「サンプルデータ読み出し用の `BufReader<File>` + 個別 cursor」に分割する
- metadata 部を `Arc` 等で共有し、`Mp4VideoCapturer` ごとに独立したファイルハンドルとサンプルストリームを持たせる公開 API を追加する (例: `Mp4SampleReader::clone_for_capturer() -> Self`、あるいは `Mp4SharedMetadata::open_stream() -> Mp4SampleStream`)
- demux は 1 回で済むが、FD と feeder スレッドは capturer ごとに増える
- サンプルデータの `Vec<u8>` を毎フレーム個別に読み出す構造は現行と同じ
- SDK 内部の変更は比較的小さく、`shiguredo_webrtc` 側の変更は不要

### 案 2: MP4 パススルー video_source の per-encoder フォーク

- `Mp4VideoCapturer` から複数 encoder 向けに独立した `VideoTrackSource` を発行できる API を追加する (例: `Mp4VideoCapturer::fork_video_source() -> VideoTrackSource`)
- 内部で 1 つの feeder スレッドがサンプルを読み、encoder ごとに `VideoFrameBufferHandlerState` を作り分けて配信する
- SDK 内部で per-buffer スレッド固定制約を吸収するため、`AdaptedVideoTrackSource` と `VideoFrameBufferHandler` の呼び出し規約と整合できるかの調査が必要
- demux・FD・feeder スレッドいずれも 1 のまま複数 encoder に配信できる
- 場合によっては `shiguredo_webrtc` 側の API 追加も必要

案 1 は変更範囲が小さいが FD・スレッドは減らない。案 2 は FD・スレッドまで 1 に抑えられるが SDK 内部の変更が大きい。zakuro-rs のように `--vcs` が数百に達するユースケースを想定するなら案 2 が本命。

## 完了条件

- 1 つの MP4 ファイルから N 個の `Mp4VideoCapturer` または N 個の独立した `VideoTrackSource` を発行できる公開 API がある
- 生成した各 `VideoTrackSource` を別々の PeerConnection の encoder に接続しても `video_frame_buffer callback called from multiple threads` の panic が発生しない
- 採用した設計方針で「demux 回数」「オープン FD 数」「feeder OS スレッド数」がそれぞれ N 倍からどこまで削減できたかが本 issue に記録されている
- 既存の 1 対 1 (`Mp4SampleReader::new` → `Mp4VideoCapturer::new` → 単一の `VideoTrackSource`) の利用形態は破壊せず引き続き動作する
- 共有 API の使用例を `docs/INPUT_MP4.md` (または相当ドキュメント) に追記する
- 実 MP4 fixture (`testdata/red-320x320-h264.mp4` など) で複数 capturer / encoder を同時に動作させ、それぞれが正常なフレームを供給できることの回帰テストを追加する
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- コメントは日本語、ログメッセージは英語、テストの assertion message は日本語で書く
- モックやスタブは使用しない
- `CHANGES.md` の develop セクションに `[ADD]` エントリを追記する

## 次回の方針確認で決めること

- 案 1 (メタデータ共有 + 個別サンプルストリーム) と案 2 (per-encoder video_source フォーク) のどちらを採用するか
- 案 2 を採用する場合、`shiguredo_webrtc` 側の変更が必要かどうか (per-buffer スレッド固定 assertion を SDK 側で吸収する経路の設計)
- 共有 API の型名と所有権設計 (`Arc<Mp4SharedMetadata>` / `Mp4SampleReader::clone_for_capturer()` / `Mp4VideoCapturer::fork_video_source()` 等)
- 対象コーデック (H.264 / H.265 / VP8 / VP9 / AV1) 全てで共有動作を保証するか、初期対応は一部コーデックに限定するか

## 変更対象

- `src/video_codecs/mp4.rs` (共有 API の追加、`Mp4SampleReader` と `Mp4VideoCapturer` の分割 / フォーク API)
- `docs/INPUT_MP4.md` (共有 API の使用例と制約の追記)
- `testdata/` (必要に応じて共有動作確認用の fixture 追加)
- `CHANGES.md`
- 場合によっては `shiguredo_webrtc` (別リポジトリ、案 2 で per-buffer スレッド固定制約を SDK 側で吸収する場合)
