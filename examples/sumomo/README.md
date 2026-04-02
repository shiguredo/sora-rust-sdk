# sumomo

Sora WebSocket シグナリングの最小サンプル

## Linux での依存ライブラリ

ビルドには以下のパッケージが必要です。

```bash
sudo apt install libssl-dev pkg-config
```

`media-device` feature を使用する場合は、追加で以下のパッケージが必要です。

```bash
sudo apt install pipewire-pulse libpulse-dev
```

`pipewire-pulse` が起動していない場合は、事前に起動してください。

```bash
systemctl --user enable --now pipewire pipewire-pulse
```

## オプション

| オプション | 種別 | 説明 |
|---|---|---|
| `--signaling-url` | 必須 | Sora の WebSocket シグナリング URL |
| `--channel-id` | 必須 | Sora のチャネル ID |
| `--role` | 必須 | Sora のロール (`sendonly` / `recvonly` / `sendrecv`) |
| `--audio` | 任意 (値: `true`/`false`) | 音声の有効/無効 |
| `--video` | 任意 (値: `true`/`false`) | 映像の有効/無効 |
| `--video-codec-type` | 任意 (値: `vp8`/`vp9`/`av1`/`h264`/`h265`) | 映像コーデック |
| `--openh264-path` | 任意 (値: ライブラリパス) | OpenH264 の動的ライブラリパス |
| `--data-channel-signaling` | 任意 (値: `true`/`false`) | DataChannel 経由でシグナリングを行う |
| `--ignore-disconnect-websocket` | 任意 (値: `true`/`false`) | DataChannel 使用時に WebSocket 切断を無視する |
| `--simulcast` | 任意 (値: `true`/`false`) | サイマルキャストを有効にする |
| `--duration` | 任意 (値: 秒数) | 接続を維持する秒数 (省略時は無制限) |
| `--raw-player` | 任意 (フラグ、`raw-player` feature 有効時のみ) | raw-player でビデオを表示する |
| `--list-devices` | 任意 (フラグ、`media-device` feature 有効時のみ) | 利用可能なデバイス一覧を表示して終了する |
| `--video-input-device` | 任意 (値: デバイス ID、`media-device` feature 有効時のみ) | 使用するビデオ入力デバイスの ID |
| `--audio-input-device` | 任意 (値: デバイス名または ID、`media-device` feature 有効時のみ) | 使用するオーディオ入力デバイスの名前または ID |
| `--help` | フラグ | ヘルプ表示 |
| `--version` | フラグ | バージョン表示 |

## 制約

- `--input-mp4` と `--openh264-path` は同時に指定できません

## 実行例

### sendonly で接続する

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora-test.shiguredo.co.jp/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --duration 60
```

### DataChannel シグナリングを有効にして sendonly で接続する

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora-test.shiguredo.co.jp/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --data-channel-signaling true \
  --ignore-disconnect-websocket true \
  --duration 60
```

### recvonly で接続する

ターミナルに ANSI でビデオを表示します。

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora-test.shiguredo.co.jp/signaling \
  --channel-id your-channel-id \
  --role recvonly
```

### OpenH264 を使って H.264 で接続する

```bash
cargo run -p sumomo -- \
  --signaling-url wss://sora-test.shiguredo.co.jp/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --video-codec-type h264 \
  --openh264-path /path/to/libopenh264.so \
  --duration 60
```

### デバイス一覧を表示する

`media-device` feature を有効にして `--list-devices` フラグを指定すると、利用可能なデバイス一覧を JSON で表示します。

```bash
cargo run -p sumomo --features media-device -- \
  --list-devices
```

### ビデオ入力デバイスを指定して sendonly で接続する

`--list-devices` で確認したデバイス ID を `--video-input-device` に指定します。省略時は FakeVideoCapturer を使用します。

```bash
cargo run -p sumomo --features media-device -- \
  --signaling-url wss://sora-test.shiguredo.co.jp/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --video-input-device /dev/video0
```

### オーディオ入力デバイスを指定して sendonly で接続する

`--list-devices` で確認したデバイス名または ID を `--audio-input-device` に指定します。

```bash
cargo run -p sumomo --features media-device -- \
  --signaling-url wss://sora-test.shiguredo.co.jp/signaling \
  --channel-id your-channel-id \
  --role sendonly \
  --audio-input-device alsa_input.pci-0000_00_1f.3.analog-stereo
```

### raw-player でビデオを表示する

`raw-player` feature を有効にして `--raw-player` フラグを指定すると、raw-player ウィンドウでビデオを表示します。

```bash
cargo run -p sumomo --features raw-player -- \
  --signaling-url wss://sora-test.shiguredo.co.jp/signaling \
  --channel-id your-channel-id \
  --role recvonly \
  --raw-player
```
