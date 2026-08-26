# MP4 ファイル入力 (パススルー送信)

## 概要

sumomo の `--input-mp4` オプションを使用すると、MP4 ファイルからエンコード済みビデオフレームを抽出し、再エンコードなしに WebRTC で送信できる。

## 対応コーデック

- H.264
- H.265
- VP8
- VP9
- AV1

## 制約

- 映像のみ送信する (音声は無視される)
- B フレームを含む MP4 は初期化時に拒否する
- 再送やキーフレーム要求は無視する
- MP4 の末尾に到達すると先頭に戻りループ再生する
- `--video-input-device` との同時指定はできない

## 実行方法

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora.example.com/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --input-mp4 /path/to/video.mp4 \
  --video-bit-rate 30000 \
  --audio false
```

MP4 の実コーデックはファイルから自動で検出され、`--video-codec-type` で指定する必要はない。`--video-codec-type` は `--input-mp4` と併用できない。

## 仕組み

1. `shiguredo_mp4` クレートで MP4 ファイルを読み込み、ビデオトラックのエンコード済みサンプルを抽出する
2. カスタムの `VideoCodecCapability` (パススルーエンコーダー) を WebRTC のエンコーダーパイプラインに登録する
3. パススルーエンコーダーは `encode()` 呼び出し時に、事前抽出したエンコード済みデータをそのまま `EncodedImage` として出力する
4. H.264 の場合は AVCC フォーマットから Annex B フォーマットへの変換と、IDR フレーム前への SPS/PPS 付与を行う
5. H.265 の場合は HVCC フォーマットから Annex B フォーマットへの変換と、IDR フレーム前への VPS/SPS/PPS 付与を行う

## SDK での利用

SDK を直接利用する場合、MP4 パススルーは次の手順で使います。

```rust
let reader = Mp4SampleReader::new(path)?;
let capability = reader.passthrough_capability();
let capturer = Mp4VideoCapturer::new(reader)?;
```

### 1 つの video_source を複数 encoder に渡せない

同じ `Mp4VideoCapturer` の `video_source()` を、複数の PeerConnection の映像 encoder に渡してはなりません。1 つの `VideoTrackSource` が運ぶ native `VideoFrameBuffer` を複数の encoder スレッドが処理することになります。debug ビルド (`debug_assertions` 有効時) では次のメッセージで abort します。

```
assertion `left == right` failed: video_frame_buffer callback called from multiple threads
```

release ビルドではこの assertion はありませんが、同じ使い方は非対応のままです。

### capturer を分ける

複数の PeerConnection に送る場合は、PeerConnection ごとに `Mp4VideoCapturer` を分けます。各 capturer の `video_source()` だけを、その接続の encoder に渡してください。1 つの `video_source()` を使い回さないでください。

### reader は共有できる

capturer を分けても、同じファイルなら `Mp4SampleReader` は `clone` して共有できます。

```rust
let reader = Mp4SampleReader::new(path)?;
let capturer1 = Mp4VideoCapturer::new(reader.clone())?;
let capturer2 = Mp4VideoCapturer::new(reader.clone())?;
```

demux とファイル読み出しは 1 つの reader にまとまり、再生位置と再生タイミングは capturer ごとに独立します。ファイルが接続ごとに違う場合は、ファイルごとに `Mp4SampleReader::new` してください。
