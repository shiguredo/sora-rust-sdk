# sumomo の recvonly で --audio-input-device のキャプチャーが起動し続ける問題を直す

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-recvonly-audio-capturer
- Polished: 2026-08-16

## 目的

`--role recvonly --audio-input-device X` を指定しても、音声キャプチャーが起動しないようにする（外部 ADM も生成しない）。

## 現状

`examples/sumomo/src/main.rs` の `build_and_run_connection` は、`args.audio_enabled()`（`--audio false` のガード）と `--audio-input-device` の指定のみで音声キャプチャー (`AudioDeviceCapturer`) と外部 ADM (`SumomoAdm`) を生成しており、role を確認していない。`--role recvonly --audio-input-device X` と指定するとマイクデバイスが起動するが、`attach_sender_tracks` の音声トラック添付は `args.role.wants_send()` を確認するため、トラックは添付されず録音データはどこにも使われない。

`--audio false` のガードは 0117 の対応で追加されたが、role のガードはない。映像側のキャプチャー生成は `attach_sender_tracks` 内で `args.role.wants_send() && video_enabled` により role を確認しているため、audio 側とは非対称。

## 設計方針

- 音声キャプチャー (`AudioDeviceCapturer`) と外部 ADM (`SumomoAdm`) の生成条件の両方に `args.role.wants_send()` を追加する
  - 映像側のキャプチャー生成 (`attach_sender_tracks` 内の `args.role.wants_send() && video_enabled`) と同様の扱い
  - 音声キャプチャーは外部 ADM の state に依存する (`build_and_run_connection` 内の `external_adm.as_ref().expect(...)`) ため、両方を同時にゲートする。外部 ADM だけをゲートすると音声キャプチャー生成が `expect` で panic し、音声キャプチャーだけをゲートすると recvonly で不要な外部 ADM が生成される
- `validate_args` は変更しない（recvonly と `--audio-input-device` の併用をエラーにしない）
  - recvonly では `--video-input-device` を静かに無視する映像側の扱いと揃える
  - `--audio false` と `--audio-input-device` の併用をエラーにしない 0117 の判断と整合する

## 完了条件

- recvonly + `--audio-input-device` 指定時に音声キャプチャーが起動されない
- recvonly + `--audio-input-device` 指定時に外部 ADM も生成されない
- recvonly + `--audio-input-device` 指定時に起動エラーにならず、指定は静かに無視される
- sendonly / sendrecv での挙動が変わらない
- `cargo test --workspace` が成功する

## 変更対象

- `examples/sumomo/src/main.rs`
  - `build_and_run_connection`: 外部 ADM と音声キャプチャーの生成条件に `args.role.wants_send()` を追加する
- `CHANGES.md`（`[FIX]` エントリを追加）

注: open issue 0128 も `examples/sumomo/src/main.rs` の外部 ADM 生成条件を変更予定のため、実装順によっては併合が必要になる。
