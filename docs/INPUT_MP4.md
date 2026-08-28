# MP4 ファイル入力 (パススルー送信)

## 概要

MP4 パススルーは、MP4 ファイルからエンコード済みビデオフレームを抽出し、再エンコードせずに WebRTC で送信する機能である。
sumomo の `--input-mp4` オプションと Sora Rust SDK の API から利用できる。

## 対応コーデック

- H.264
- H.265
- VP8
- VP9
- AV1

## 制約

- 映像のみ送信する (音声は無視される)
- B フレームを含む MP4 は初期化時に拒否する
- 不正な H.264 トラックを含む MP4 は初期化時に拒否する
- 不正な AV1 トラックを含む MP4 は初期化時に拒否する
- 再送やキーフレーム要求は無視する
- MP4 の末尾に到達すると先頭に戻りループ再生する

## sumomo での利用

`--input-mp4` に MP4 ファイルのパスを指定する。
MP4 の実コーデックはファイルから自動で検出されるため、`--video-codec-type` を指定する必要はない。
`--input-mp4` と `--video-codec-type` は同時に指定できない。
`--video-input-device` も指定した場合は、`--input-mp4` が優先される。

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora.example.com/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --input-mp4 /path/to/video.mp4 \
  --video-bit-rate 30000 \
  --audio false
```

## SDK での利用

SDK から MP4 パススルーを利用する場合は、`Mp4SampleReader` で MP4 ファイルを読み込み、同じ `Mp4SampleReader` からパススルーエンコーダーの capability と `Mp4VideoCapturer` を生成する。
capability は `SoraConnectionContextConfig` に登録し、`Mp4VideoCapturer::video_source()` が返す `VideoTrackSource` は映像トラックの作成に使用する。

```rust
let reader = Mp4SampleReader::new(path)?;
let capability = reader.passthrough_capability();
let capturer = Mp4VideoCapturer::new(reader)?;
let video_source = capturer.video_source();
```

### 複数の PeerConnection への送信

同じ MP4 ファイルを複数の PeerConnection に送信する場合は、`Mp4SampleReader` を共有し、PeerConnection ごとに `Mp4VideoCapturer` を生成する。
`Mp4SampleReader` のクローンは demux の結果とファイル I/O を共有するが、再生位置と再生タイミングは `Mp4VideoCapturer` ごとに独立している。
接続ごとに異なる MP4 ファイルを送信する場合は、ファイルごとに `Mp4SampleReader::new` を呼び出す。

```rust
let reader = Mp4SampleReader::new(path)?;
let capturer1 = Mp4VideoCapturer::new(reader.clone())?;
let video_source1 = capturer1.video_source();
let capturer2 = Mp4VideoCapturer::new(reader.clone())?;
let video_source2 = capturer2.video_source();
```

各 `video_source()` は、それぞれ対応する PeerConnection の映像トラックでのみ利用する。
同じ `Mp4VideoCapturer` の `video_source()` を複数の PeerConnection で共有すると、1 つの `VideoTrackSource` が運ぶ native `VideoFrameBuffer` を複数の encoder スレッドが処理することになるため、この使い方には対応していない。

`debug_assertions` が有効なビルドでは、次のメッセージを出力して abort する。

```text
assertion `left == right` failed: video_frame_buffer callback called from multiple threads
```

`debug_assertions` が無効なビルドではこの assertion は発生しないが、同じ使い方は非対応である。

## 仕組み

1. `shiguredo_mp4` クレートで MP4 ファイルを読み込み、ビデオトラックのエンコード済みサンプルを抽出する
2. カスタムの `VideoCodecCapability` (パススルーエンコーダー) を WebRTC のエンコーダーパイプラインに登録する
3. パススルーエンコーダーは `encode()` 呼び出し時に、事前抽出したエンコード済みデータをそのまま `EncodedImage` として出力する
4. H.264 の場合は AVCC フォーマットから Annex B フォーマットへの変換と、IDR フレーム前への SPS/PPS 付与を行う
5. H.265 の場合は HVCC フォーマットから Annex B フォーマットへの変換と、IDR フレーム前への VPS/SPS/PPS 付与を行う
6. AV1 の場合は sync sample の先頭に configOBUs (Sequence Header 等) を付与する
