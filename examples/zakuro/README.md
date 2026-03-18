# zakuro

Sora WebRTC SFU 負荷試験ツール

将来的に [zakuro](https://github.com/shiguredo/zakuro) (C++ 版) を置き換える予定です。

## オプション

| オプション | 種別 | 説明 |
|---|---|---|
| `--sora-signaling-url` | 必須 | Sora の WebSocket シグナリング URL |
| `--sora-channel-id` | 必須 | Sora のチャネル ID |
| `--sora-role` | 必須 | Sora のロール (`sendonly` / `recvonly` / `sendrecv`) |
| `--vcs` | 任意 (値: 1-1000, デフォルト: 1) | 仮想クライアント数 |
| `--vcs-hatch-rate` | 任意 (値: 秒あたりの起動数, デフォルト: 1.0) | 仮想クライアントの起動レート |
| `--duration` | 任意 (値: 秒数) | 仮想クライアントの接続維持秒数 (省略時は無制限) |
| `--repeat-interval` | 任意 (値: 秒) | duration 経過後の再接続間隔 |
| `--max-retry` | 任意 (値: 回数, デフォルト: 0) | 接続失敗時の最大リトライ回数 |
| `--retry-interval` | 任意 (値: 秒, デフォルト: 60.0) | リトライ間隔 |
| `--no-video-device` | フラグ | 映像デバイスを使用しない |
| `--no-audio-device` | フラグ | 音声デバイスを使用しない |
| `--resolution` | 任意 (値: QVGA/VGA/HD/FHD/4K または WxH, デフォルト: VGA) | 映像解像度 |
| `--framerate` | 任意 (値: 1-60, デフォルト: 30) | 映像フレームレート |
| `--sandstorm` | フラグ | 砂嵐映像を生成する |
| `--sora-video-codec-type` | 任意 (値: `vp8`/`vp9`/`av1`/`h264`/`h265`) | 映像コーデック |
| `--sora-video-bit-rate` | 任意 (値: kbps) | 映像ビットレート |
| `--sora-audio` | 任意 (値: `true`/`false`, デフォルト: `true`) | 音声の有効/無効 |
| `--sora-audio-codec-type` | 任意 (値: `opus`) | 音声コーデック |
| `--sora-audio-bit-rate` | 任意 (値: kbps) | 音声ビットレート |
| `--sora-data-channel-signaling` | 任意 (値: `true`/`false`) | DataChannel 経由でシグナリングを行う |
| `--sora-ignore-disconnect-websocket` | 任意 (値: `true`/`false`) | DataChannel 使用時に WebSocket 切断を無視する |
| `--help` | フラグ | ヘルプ表示 |
| `--version` | フラグ | バージョン表示 |

## 実行例

### sendonly で負荷試験を行う

```bash
cargo run -p zakuro -- \
  --sora-signaling-url wss://sora.example.com/signaling \
  --sora-channel-id zakuro-test \
  --sora-role sendonly \
  --vcs 10
```

### 砂嵐映像で負荷試験を行う

```bash
cargo run -p zakuro -- \
  --sora-signaling-url wss://sora.example.com/signaling \
  --sora-channel-id zakuro-test \
  --sora-role sendonly \
  --vcs 10 \
  --sandstorm
```

### duration と repeat-interval を指定して繰り返し接続する

```bash
cargo run -p zakuro -- \
  --sora-signaling-url wss://sora.example.com/signaling \
  --sora-channel-id zakuro-test \
  --sora-role sendonly \
  --vcs 5 \
  --duration 30 \
  --repeat-interval 5
```
