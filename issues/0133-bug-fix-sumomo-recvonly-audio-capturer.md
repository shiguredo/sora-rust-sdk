# sumomo の recvonly で --audio-input-device のキャプチャーが起動し続ける問題を直す

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-recvonly-audio-capturer
- Polished: {YYYY-MM-DD}

## 目的

`--role recvonly --audio-input-device X` を指定しても、音声キャプチャーが起動しないようにする。

## 現状

`examples/sumomo/src/main.rs` の `main` と `run_with_raw_player` の音声キャプチャー (`AudioDeviceCapturer`) の起動条件は、`audio_enabled`（`--audio false` のガード）と `--audio-input-device` の指定のみで、role を確認していない。`--role recvonly --audio-input-device X` と指定するとマイクデバイスが起動するが、`attach_sender_tracks` の音声トラック添付は `args.role.wants_send()` を確認するため、トラックは添付されず録音データはどこにも使われない。

`--audio false` のガードは 0117 の対応で追加されたが、role のガードはない。映像側のキャプチャー生成は `attach_sender_tracks` 内で `args.role.wants_send() && video_enabled` により role を確認しているため、audio 側とは非対称。

## 設計方針

- 音声キャプチャーの起動条件に `args.role.wants_send()` を追加する（映像側のトラック添付と同様の扱い）
- または `validate_args` で recvonly と `--audio-input-device` の併用をエラーにする

## 完了条件

- recvonly + `--audio-input-device` 指定時に音声キャプチャーが起動されない
- sendonly / sendrecv での挙動が変わらない
- `cargo test --workspace` が成功する

## 変更対象

- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/args.rs`（`validate_args` での併用エラーを採用する場合）
