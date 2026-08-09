# テストコードの英語メッセージを日本語に統一する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-japanese-test-messages
- Polished: {YYYY-MM-DD}

## 目的

AGENTS.md の「テストのログメッセージは全て日本語にすること」に違反するテストコードの英語メッセージを日本語に統一する。

## 現状

テストコードの assertion / panic / skip / 進捗メッセージに英語が多数残っており、同一ファイル内で日本語と英語が併存している箇所もある。

- `e2e-tests/tests/` 全体で約 283 件の英語メッセージ (nvcodec_video_codec.rs / vpl_video_codec.rs / amf_video_codec.rs / openh264_video_codec.rs / v4l2_video_codec.rs / simulcast.rs / proxy_sendrecv.rs / video_codec.rs 等)。`libcamera_video_capturer.rs` は日本語で統一されており、ファイル間で分裂している
- `e2e-tests/src/lib.rs` の `panic!("{name} did not finish within timeout")` 等
- `src/video_codec_preference.rs` / `src/video_codecs/mp4.rs` / `src/libcamera.rs` / `src/connection_context.rs` / `src/video_codec_capability.rs` / `src/video_codecs/*.rs` の各テストモジュール (expect / panic メッセージ)
- `examples/sumomo/src/tests.rs`

## 設計方針

- テストコードの assert / expect / panic / skip / 進捗メッセージをすべて日本語に書き換える
- テストが出力するログ・進捗表示 (println! / eprintln!) も日本語に統一する
- テスト内で使用するコード識別子・型名・エラーメッセージの引用はそのまま残してよい
- プロダクションコードのログ・エラーは対象外

## 完了条件

- テストコードに英語の assertion / panic / skip / 進捗メッセージが残っていない
- 同一ファイル内の言語が統一されている
- プロダクションコードのログ・エラーメッセージは変更しない
- `cargo test --workspace` が成功する

## 変更対象

- `e2e-tests/` 配下のテストコード
- `src/` 配下のテストモジュール
- `examples/sumomo/src/tests.rs`
