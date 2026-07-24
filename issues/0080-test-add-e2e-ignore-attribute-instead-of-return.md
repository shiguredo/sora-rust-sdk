# e2e / SDK 内テストの「環境不足時に return する」誤合格経路を `#[ignore]` に統一する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/test-add-e2e-ignore-attribute-instead-of-return
- Polished: {YYYY-MM-DD}

## 目的

e2e-tests 配下と SDK 内のいくつかのテストで、環境変数不足や capability 生成失敗時に `println!` / `eprintln!` した上で `return` するパターンが 20+ 箇所ある。これらは cargo test から成功扱いになり、CI で環境が揃わない場合も緑になる (誤合格)。`#[ignore = "..."]` に統一して、明示的にスキップ扱いにする。

## 優先度根拠

High。「CI が緑」の信頼性が損なわれる。実際には環境依存で 1 度もテストされていないケース (OpenH264 / NvCodec / AMF / VPL / V4L2 の各機能テスト、redirect の複数 URL 前提テスト、TURN-TLS 検証テスト等) が多数あり、issue 0046 の完了条件でも触れられている。

## 現状

該当箇所 (代表例):

- `e2e-tests/tests/redirect.rs:16-24` — `TEST_SECRET_KEY` / URL 数不足で return
- `e2e-tests/tests/audio_capturer.rs` / `video_capturer.rs` — デバイス列挙失敗で return (15+ 件)
- `e2e-tests/tests/openh264_video_codec.rs:44-48, 139-143` — `OPENH264_PATH` 未設定で return
- `e2e-tests/tests/nvcodec_video_codec.rs:87, 108, 346, 385` — capability 生成失敗で return
- `e2e-tests/tests/amf_video_codec.rs:67, 85` — 同上
- `e2e-tests/tests/vpl_video_codec.rs:70, 93, 117, 141, 387, 426, 462` — 同上
- `e2e-tests/tests/simulcast.rs:171-174, 215-218, 236-239, 304-307` — 同上
- `src/video_codecs/openh264.rs:613-676, 727-748` — `OPENH264_PATH` 未設定で return

## 設計方針

1. すべての「環境不足で return」箇所に `#[ignore = "<環境の説明>"]` を追加する。
2. `#[ignore]` 付きテストは `cargo test -- --ignored` で明示実行できる。CI job 側で `--ignored` 対応の別 job を追加する (issue 0050 で対応検討)。
3. `parse_stats_lossy` (`e2e-tests/src/lib.rs:191-208`) の「stats パース失敗を握り潰して空 Vec を返す」経路は別 issue (stats パース設計見直し) として扱うが、少なくとも「パース失敗時 panic」に変更するか、フォールバックする場合は `eprintln!` ではなく明示的にエラーを伝播する。
4. 「env 不足時 return」パターンを polish/レビュー時に検出する仕組み (grep-based CI job 等) の追加は別 issue で検討。

## 完了条件

- 「環境不足で return して成功扱い」になるテストが `#[ignore]` 付きに変更されている。
- `#[ignore]` の理由が引数で明示されている (`#[ignore = "OPENH264_PATH required"]` など)。
- `parse_stats_lossy` の握り潰し経路が明示的に失敗を伝播するようになっている (もしくは別 issue 対応方針が明記されている)。
- `cargo test --workspace` が通り、`cargo test --workspace -- --ignored` でスキップされているテストを個別実行できる。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
