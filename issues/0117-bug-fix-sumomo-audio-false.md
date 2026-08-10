# sumomo の --audio false で音声トラックを添付しない

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-audio-false
- Polished: 2026-08-10

## 目的

`sumomo --audio false` を指定したときに音声トラックが SDP に含まれないようにする。

## 現状

`examples/sumomo/src/main.rs` の `attach_sender_tracks` は、映像側は `args.video.unwrap_or(true)` で分岐して `--video false` ならトラックを添付しないが、音声側は `args.audio` を一切確認せず `role.wants_send()` だけで音声トラックを常に添付する。

`apply_common_builder_options` が connect メッセージに `audio: false` を載せても、SDK 側の `SoraConnection::add_sender_tracks` (`src/connection.rs`) は `sender_audio_track` が設定されていれば無条件に `pc.add_track` するため、`--audio false` 指定時も SDP に音声 m-line が含まれる。

## 設計方針

- `attach_sender_tracks` で `args.audio` を確認し、false の場合は音声トラックを添付しない
- 映像側と同様に `args.audio.unwrap_or(true)` で判定する分岐形式に揃える
- `--audio false` 指定時は音声キャプチャーの起動も行わない（映像側の `--video false` と同じ扱いで、`--audio-input-device` との併用指定はエラーにしない）

## 完了条件

- `--audio false` 指定時に音声トラックが添付されず、SDP に音声 m-line が含まれない
- `--audio false` 指定時は音声キャプチャーが起動されない
- `--audio true` / 未指定時の挙動が変わらない
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `examples/sumomo/src/main.rs`
  - `attach_sender_tracks`: 音声トラック添付の分岐を追加する（`main` と `run_with_raw_player` の両方から呼ばれるため、この 1 箇所で両パスに効く）
  - `main` と `run_with_raw_player`: 音声キャプチャー (`AudioDeviceCapturer`) と外部 ADM (`SumomoAdm`) の生成・起動に `--audio false` の分岐を追加する
- `CHANGES.md`
