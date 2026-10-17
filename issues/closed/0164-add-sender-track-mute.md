# 送信トラックのミュート操作 API を追加する

- Created: 2026-10-04
- Completed: 2026-10-18
- Branch: feature/add-sender-track-mute
- Polished: {YYYY-MM-DD}

## 目的

アプリケーションから音声・映像のソフトミュートを操作できるようにする。利用者とブリッジから送信トラックの管理を隠蔽する。

## 現状

`SoraConnectionBuilder::sender_audio_track` / `sender_video_track` で送信トラックを渡すが、接続後にミュートを操作する API が無い。

ブリッジ側では sender track を保持して `shiguredo_webrtc::MediaStreamTrack::set_enabled` を呼べば回避できる。ただし `SoraConnectionBuilder` がトラックを消費する設計のため、ミュートのために利用者 (またはブリッジ) がトラックを複製・保持し続ける必要があり、トラックのライフサイクル管理が利用者側に漏れる。iOS / Android の両ブリッジで同じ実装を重複させることになる。

## 設計方針

`SoraConnectionHandle` に音声・映像のミュートを設定するメソッドを追加し、内部で sender track の enabled を切り替える。

- ミュート状態を取得できるようにする
- 送信トラックが無い場合の扱い (エラーか無視か) を定義する
- 公開 API に shiguredo_webrtc の型は出さない

## 完了条件

- ハンドル経由で音声・映像のソフトミュートを設定・取得できる
- 利用者が sender track を保持する必要がなくなる

## 変更対象

- `src/connection.rs` の `SoraConnectionHandle` と `SoraConnection` (送信トラックの保持)
- `skills/sora-rust-sdk/SKILL.md` (API 一覧の更新)

## 解決方法

対応しない。ミュートは利用者が送信トラックの複製を保持して `shiguredo_webrtc::MediaStreamTrack::set_enabled` を呼ぶことで実現でき、SDK に API を追加する必要がないと判断した。

- `VideoTrack` / `AudioTrack` は `Clone` でき、複製は `ScopedRef` により同じ実体を指す。`SoraConnectionBuilder::sender_video_track` / `sender_audio_track` にトラックを渡す前に複製を 1 つ残せば、接続後いつでも `set_enabled` を呼べる
- `set_enabled` は `&self` であり、`SoraConnection::run()` が別タスクで動いていても同じ実体の状態を書き換えるため、そのまま反映される
- `SoraConnection` がトラックを保持しなくてもこの操作は成立する。`RtpSender` からトラックを逆引きする API が無いことが「保持できない」理由にはなるが、「保持しなければならない」理由にはならない
- SDK 側にミュート状態を持つ設計は、送信トラックを持たない `Role::RecvOnly` や送信トラック未指定の接続での挙動、Re-offer による sender の再構築といった分岐を SDK 内部に増やす。利用者が複製を 1 つ保持する手間に見合わない
- ブリッジ間の実装重複はブリッジ側で共通化する話であり、SDK の公開 API を増やす根拠にはならない
