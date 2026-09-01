# Sumomo (Rust SDK) と Sumomo (C++ SDK) の機能比較

## 概要

sumomo は [WebRTC Native Client Momo](https://github.com/shiguredo/momo) の sora モードを模したサンプルです。

本ドキュメントでは、Sora Rust SDK の sumomo と Sora C++ SDK の sumomo の機能差をまとめます。

比較対象の C++ SDK の sumomo は、`2026.2.0-canary.27` (develop ブランチ、2026-08-03 時点) を基準とします。

## 共通の必須オプション

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--signaling-url` | o | o | C++ SDK は 1 つのみ / Rust SDK はカンマ区切りで複数指定可能 |
| `--channel-id` | o | o | |
| `--role` | o | o | `sendonly` / `recvonly` / `sendrecv` |

## シグナリング

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--client-id` | o | 未実装 | |
| `--metadata` | o | 未実装 | SDK 側は対応済みだが sumomo に未公開 |
| `--spotlight` | o | 未実装 | SDK 側は対応済みだが sumomo に未公開 |
| `--spotlight-number` | o | 未実装 | Sora で非推奨のため Rust SDK は実装しない |
| `--simulcast` | o | o | C++ SDK は `true` / `false` / `none` / Rust SDK は `true` / `false` のみ |
| `--data-channel-signaling` | o | o | C++ SDK は `true` / `false` / `none` / Rust SDK は `true` / `false` のみ |
| `--ignore-disconnect-websocket` | o | o | C++ SDK は `true` / `false` / `none` / Rust SDK は `true` / `false` のみ |
| `--duration` | 未実装 | o | 接続を維持する秒数 (省略時は無制限) |

## メディア

### 映像

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--video` | o | o | |
| `--video-codec-type` | o | o | C++ SDK: `VP8` / `VP9` / `AV1` / `H264` / `H265` / Rust SDK: `vp8` / `vp9` / `av1` / `h264` / `h265` |
| `--video-bit-rate` | o | o | |
| `--video-h264-params` | o | 未実装 | SDK 側は対応済みだが sumomo に未公開 |
| `--video-h265-params` | o | 未実装 | SDK 側は対応済みだが sumomo に未公開 |
| `--resolution` | o | 未実装 | Rust SDK は 640x480 固定 |
| `--video-device` | o | o | Rust SDK は `--video-input-device` (media-device フィーチャー時のみ) |
| `--hw-mjpeg-decoder` | o | 未実装 | NVIDIA Jetson のみで利用可能 |
| `--input-mp4` | 未実装 | o | MP4 ファイルからエンコード済み映像をそのまま送信する。H.264 では connect の `h264_params.profile_level_id` を自動補完する（`--video-h264-params` CLI とは別） |

### 音声

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--audio` | o | o | |
| `--audio-codec-type` | o | 未実装 | Rust SDK は OPUS 固定 |
| `--audio-bit-rate` | o | 未実装 | |
| `--audio-recording-device` | o | o | Rust SDK は `--audio-input-device` (media-device フィーチャー時のみ) |
| `--audio-playout-device` | o | 未実装 | Rust SDK は音声出力に未対応 |

## 映像キャプチャー

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--fake-capture-device` | o | 相当機能あり | C++ SDK はビープ音を生成する / Rust SDK は映像デバイス未指定時に Fake キャプチャーがデフォルトで、ビープ音は生成しない |
| `--use-libcamera` | o | o | Rust SDK は `--libcamera` |
| `--use-libcamera-native` | o | o | Rust SDK は `--libcamera-native` |
| `--libcamera-control` | o | o | C++ SDK は `キー 値` / Rust SDK は `KEY=VALUE` |

## 映像表示

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--use-sdl` | o | 未実装 | Rust SDK は `--raw-player` が相当 |
| `--window-width` / `--window-height` | o | 未実装 | |
| `--fullscreen` | o | 未実装 | |
| `--show-me` | o | 未実装 | 送信している自分の映像を表示する |
| `--use-sixel` | o | 未実装 | |
| `--sixel-width` / `--sixel-height` | o | 未実装 | |
| `--use-ansi` | o | 常時有効 | C++ SDK は `--ansi-width` / `--ansi-height` でサイズを指定 / Rust SDK は 80x45 固定で選択不可 |
| `--raw-player` | 未実装 | o | raw-player でビデオを表示する (raw-player フィーチャー時のみ) |

## コーデック実装

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--vp8-encoder` / `--vp8-decoder` | o | 相当機能あり | Rust SDK はコーデック実装を 1 オプションで一括指定する |
| `--vp9-encoder` / `--vp9-decoder` | o | 相当機能あり | |
| `--h264-encoder` / `--h264-decoder` | o | 相当機能あり | |
| `--h265-encoder` / `--h265-decoder` | o | 相当機能あり | |
| `--av1-encoder` / `--av1-decoder` | o | 相当機能あり | |
| `--video-codec-implementation` | 未実装 | o | `auto` または `internal` / `internal-apple` / `amf` / `nvcodec` / `vpl` / `v4l2` / `openh264` のカンマ区切り |
| `--openh264` | o | o | C++ SDK はパスのみ指定 / Rust SDK は `--openh264-path` と実装指定のセット |
| `--show-video-codec-capability` | o | 相当機能あり | Rust SDK は `--video-codec-list` で利用可能な実装と選択優先順位を表示する |

指定可能なコーデック実装の対応は、C++ SDK が `internal` / `cisco_openh264` / `intel_vpl` / `nvidia_video_codec` / `amd_amf` / `raspi_v4l2m2m`、Rust SDK が `internal` / `internal-apple` / `amf` / `nvcodec` / `vpl` / `v4l2` / `openh264` です。

## 接続・セキュリティ

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--insecure` | o | o | |
| `--client-cert` / `--client-key` | o | o | |
| `--ca-cert` | o | o | |
| `--turn-tls-insecure` | 未実装 | o | |
| `--turn-tls-ca-cert` | 未実装 | o | |
| `--proxy-url` / `--proxy-username` / `--proxy-password` | o | 未実装 | SDK 側は対応済みだが sumomo に未公開 |
| `--degradation-preference` | o | 未実装 | |
| `--cpu-adaptation` | o | 未実装 | |

## その他

| オプション | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| `--log-level` | o | 未実装 | Rust SDK は固定で `Info` / raw-player 使用時は `Warning` |
| `--http-port` / `--http-host` | o | 未実装 | `/stats` エンドポイントで WebRTC 統計情報を JSON で取得できる |
| `--list-devices` | o | o | Rust SDK は media-device フィーチャー時のみ |

## ビルド時のフィーチャー

| フィーチャー | C++ SDK | Rust SDK | 備考 |
|---|---|---|---|
| AMF | o | o | Rust SDK は `amf` フィーチャー |
| libcamera | o | o | Rust SDK は `libcamera` フィーチャー |
| NVCodec | o | o | Rust SDK は `nvcodec` フィーチャー |
| VPL | o | o | Rust SDK は `vpl` フィーチャー |
| V4L2 | o | o | Rust SDK は `v4l2` フィーチャー |
| OpenH264 | o | o | C++ SDK は `--openh264` / Rust SDK は `--openh264-path` で動的ライブラリのパスを指定 |
| デバイス入出力 | o | o | Rust SDK は `media-device` フィーチャーで有効化 |
| ウィンドウ表示 | SDL 組み込み | o | Rust SDK は `raw-player` フィーチャーで有効化 |
