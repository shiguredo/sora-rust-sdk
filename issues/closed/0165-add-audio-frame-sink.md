# 受信音声フレームを取得できるようにする

- Created: 2026-10-04
- Completed: 2026-10-18
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

## 解決方法

対応しない。受信音声 PCM は利用者が `shiguredo_webrtc::AudioTrackSink` を登録することで取得でき、SDK に専用の API を追加する必要がないと判断した。

- 利用者が `SoraConnectionContextConfig::signaling_thread` に自身で生成したスレッドを渡し、そのスレッドへ `Thread::blocking_call` で sink の登録・解除を行えば、SDK を変更せずに正しい文脈で実行できる
- SDK に寄せても利用者が管理する項目は減らない。トラックごとの sink 生成、トラックと sink の対応表、受信トラック削除時の解除は、どちらに置いても同じだけ必要になる
- 差は「対応表を SDK が持つか利用者が持つか」だけで、SDK に寄せる場合は公開 API と内部状態が増える
- 複数トラックの識別は、`RemoteAudioSource::AddSink` が同一 sink の重複登録を許さないため、トラックごとに sink を分けることで自然に得られる
- 解除漏れによるダングリングポインタは、利用者側で `remove_sink` を drop 時に呼ぶ RAII のラッパーを用意すれば防げる。SDK の公開 API を増やさずに解決できる
- ブリッジ間の実装重複はブリッジ側で共通化する話であり、SDK の公開 API を増やす根拠にはならない
