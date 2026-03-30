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
- B フレームを含む MP4 には対応していない (映像がガクガクする)
- 再送やキーフレーム要求は無視する
- MP4 の末尾に到達すると先頭に戻りループ再生する
- `--video-input-device` との同時指定はできない

## 実行方法

### H.264 の MP4 ファイルを送信する

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora.example.com/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --input-mp4 /path/to/video.mp4 \
  --video-codec-type h264 \
  --video-bit-rate 30000 \
  --audio false
```

### H.265 の MP4 ファイルを送信する

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora.example.com/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --input-mp4 /path/to/video.mp4 \
  --video-codec-type h265 \
  --video-bit-rate 30000 \
  --audio false
```

### VP9 の MP4 ファイルを送信する

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora.example.com/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --input-mp4 /path/to/video.mp4 \
  --video-codec-type vp9 \
  --video-bit-rate 30000 \
  --audio false
```

### AV1 の MP4 ファイルを送信する

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora.example.com/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --input-mp4 /path/to/video.mp4 \
  --video-codec-type av1 \
  --video-bit-rate 30000 \
  --audio false
```

## 仕組み

1. `shiguredo_mp4` クレートで MP4 ファイルを読み込み、ビデオトラックのエンコード済みサンプルを抽出する
2. カスタムの `VideoCodecCapability` (パススルーエンコーダー) を WebRTC のエンコーダーパイプラインに登録する
3. パススルーエンコーダーは `encode()` 呼び出し時に、事前抽出したエンコード済みデータをそのまま `EncodedImage` として出力する
4. H.264 の場合は AVCC フォーマットから Annex B フォーマットへの変換と、IDR フレーム前への SPS/PPS 付与を行う
5. H.265 の場合は HVCC フォーマットから Annex B フォーマットへの変換と、IDR フレーム前への VPS/SPS/PPS 付与を行う
