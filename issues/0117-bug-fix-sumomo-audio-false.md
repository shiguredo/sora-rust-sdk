# sumomo の --audio false で音声トラックを添付しない

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-audio-false
- Polished: {YYYY-MM-DD}

## 目的

`sumomo --audio false` を指定したときに音声トラックが SDP に含まれないようにする。

## 現状

`examples/sumomo/src/main.rs` の `attach_sender_tracks` は、映像側は `args.video.unwrap_or(true)` で分岐して `--video false` ならトラックを添付しないが、音声側は `args.audio` を一切確認せず `role.wants_send()` だけで音声トラックを常に添付する。

`apply_common_builder_options` が connect メッセージに `audio: false` を載せても、SDK 側の `SoraConnection::add_sender_tracks` (`src/connection.rs`) は `sender_audio_track` が設定されていれば無条件に `pc.add_track` するため、`--audio false` 指定時も SDP に音声 m-line が含まれる。

## 設計方針

- `attach_sender_tracks` で `args.audio` を確認し、false の場合は音声トラックを添付しない
- 映像側と同様の分岐形式に揃える

## 完了条件

- `--audio false` 指定時に音声トラックが添付されない
- 音声の詳細指定 (`--audio-opus-*` 等) がある場合は従来どおり添付される
- `--audio true` / 未指定時の挙動が変わらない
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `examples/sumomo/src/main.rs`
- `CHANGES.md`
