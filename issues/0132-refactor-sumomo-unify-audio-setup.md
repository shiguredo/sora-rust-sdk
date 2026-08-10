# sumomo の音声キャプチャーと外部 ADM の生成を 1 箇所にまとめる

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/refactor-sumomo-unify-audio-setup
- Polished: {YYYY-MM-DD}

## 目的

`examples/sumomo/src/main.rs` の `main` と `run_with_raw_player` に同一コードで重複している音声キャプチャー (`AudioDeviceCapturer`) と外部 ADM (`SumomoAdm`) の生成ロジックを 1 箇所にまとめ、video 側との構造の非対称を解消する。

## 現状

- video 側は `attach_sender_tracks` 内で `create_video_capturer` を呼び、生成したキャプチャーを戻り値として `main` / `run_with_raw_player` に返す。生成ロジックは 1 箇所
- audio 側は `external_adm` (SumomoAdm) の生成、`AdmConfig` の決定、`AudioDeviceCapturer` の生成・起動が `main` と `run_with_raw_player` のそれぞれに同一コードで記述されており、2 箇所で重複している
- 0117 の対応で `audio_enabled` の分岐が両方に追加されたが、重複構造は解消されていない

## 設計方針

- 外部 ADM は `SoraConnectionContext` の構築 (`AdmConfig` の決定) より前に生成する必要がある制約を踏まえ、audio キャプチャーと外部 ADM の生成を 1 箇所にまとめる
- video 側との対称性を高めるため、`attach_sender_tracks` 内での生成・返却に揃えることを検討する（外部 ADM の生成タイミングの制約から、生成の一部を `attach_sender_tracks` の前段に残す必要がある場合はその旨を明記する）
- 挙動は変えない

## 完了条件

- `main` と `run_with_raw_player` の音声セットアップの重複が解消されている
- `--audio false` / `--audio true` / 未指定時の挙動が変わらない
- `cargo test --workspace` が成功する

## 変更対象

- `examples/sumomo/src/main.rs`
