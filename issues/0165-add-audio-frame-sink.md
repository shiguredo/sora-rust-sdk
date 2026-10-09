# 受信音声フレームを取得できるようにする

- Created: 2026-10-04
- Completed: {YYYY-MM-DD}
- Branch: feature/add-audio-frame-sink
- Polished: {YYYY-MM-DD}

## 目的

受信音声をアプリケーションで処理 (解析・独自再生・録音) できるようにする。受信音声の取得をコアの API として提供し、各 SDK の sink 管理を不要にする。

## 現状

sora_sdk は受信音声の PCM を公開していない。

ブリッジ側では `on_track` で受け取ったトラックに `shiguredo_webrtc::AudioTrackSink` を接続すれば回避できる。ただしトラックごとの sink 登録・解除と、受信トラック削除との同期を利用者 (またはブリッジ) が管理する必要があり、iOS / Android の両ブリッジで同じ実装を重複させることになる。

## 設計方針

`SoraConnectionEventHandler` に受信音声フレームのコールバックを追加する。

- PCM データとサンプルレート / チャンネル数 / フレーム数を渡す
- 複数トラックを識別できるようにする
- 既存の `on_track` はそのまま残す
- 公開 API に shiguredo_webrtc の型は出さない

接続に音声 sink を登録する API を追加する方式でもよい。

## 完了条件

- SDK の API だけで受信音声 PCM を取得できる
- 複数トラックを識別できる

## 変更対象

- `src/connection_event_handler.rs` の `SoraConnectionEventHandler`
- `src/connection.rs` (音声 sink の登録と解除)
- `skills/sora-rust-sdk/SKILL.md` (イベント一覧の更新)
