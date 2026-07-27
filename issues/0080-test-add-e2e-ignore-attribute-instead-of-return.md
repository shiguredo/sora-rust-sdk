# e2e / SDK 内テストの「環境不足時に return する」誤合格経路を `#[ignore]` に統一する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-27
- Model: Opus 4.7
- Branch: feature/test-add-e2e-ignore-attribute-instead-of-return
- Polished: {YYYY-MM-DD}
- Updated: 2026-07-24

## 目的

e2e-tests 配下と SDK 内のいくつかのテストで、環境変数不足や capability 生成失敗時に `println!` / `eprintln!` した上で `return` するパターンが 20+ 箇所ある。これらは cargo test から成功扱いになり、CI で環境が揃わない場合も緑になる (誤合格)。`#[ignore = "..."]` に統一して、明示的にスキップ扱いにする。

## 優先度根拠

High。「CI が緑」の信頼性が損なわれる。実際には環境依存で 1 度もテストされていないケース (OpenH264 / NvCodec / AMF / VPL / V4L2 の各機能テスト、redirect の複数 URL 前提テスト、TURN-TLS 検証テスト等) が多数あり、issue 0046 の完了条件でも触れられている。

## 現状

該当箇所 (代表例。行番号は 2026-07-24 の実測値)。skip 経路は「(a) テスト本体で `let Some(...) else { return; };` が発火する行 = 誤合格の直接原因」と「(b) helper が `return None;` を返す行 = (a) の入力側」を区別して列挙する。

- `e2e-tests/tests/redirect.rs:16-24` — `TEST_SECRET_KEY` / URL 数不足で return (テスト本体)
- `e2e-tests/tests/audio_capturer.rs` / `video_capturer.rs` — デバイス列挙失敗で return (15+ 件)
- `e2e-tests/tests/openh264_video_codec.rs:44-48, 139-143` — `OPENH264_PATH` 未設定で return
- `e2e-tests/tests/nvcodec_video_codec.rs`:
  - (a) テスト本体の skip 経路 `:330,367,380` (`let Some(codec_types) = nvcodec_*() else { return; };`)
  - (b) helper の `return None;`: `nvcodec_fully_supported_codecs` `:87`、`nvcodec_decoder_supported_only_codecs` `:108`
  - ループ内 skip (誤合格ではないが方針検討対象): `:389 continue` (encoder unsupported)
- `e2e-tests/tests/amf_video_codec.rs:67, 85` — 同上パターン (要精査。skip 経路と helper の分離は未確認)
- `e2e-tests/tests/vpl_video_codec.rs`:
  - (a) テスト本体の skip 経路 `:371,408,457` (`let Some(codec_types) = vpl_*() else { return; };`)
  - (b) helper の `return None;` / eprintln + Err 経路: `vpl_capability` `:70` (eprintln)、`vpl_fully_supported_codecs` `:93`、`vpl_decoder_supported_only_codecs` `:117`、`vpl_encoder_supported_only_codecs` `:141`
  - ループ内 skip: `:421,466 return`、`:430,466 continue` (要精査)
- `e2e-tests/tests/simulcast.rs:171-174, 215-218, 236-239, 304-307` — 同上パターン
- `src/video_codecs/openh264.rs:613-676, 727-748` — `OPENH264_PATH` 未設定で return

`#[ignore]` 化の第一対象は各テストの (a) 側 (テスト本体の skip 経路)。helper 側の `return None;` は (a) が `#[ignore]` 化されれば呼び出し元が消えるため、そのまま残してよいか (a) と一緒に整理するかは実装時に判断する。

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

## 解決方法

`#[ignore]` はコンパイル時属性であり実行時の環境を動的に評価できないため、OpenH264 が入っているマシンでも常にスキップされてしまう。テストを実行するには `--ignored` が必須となり、ローカル開発における煩わしさが大きい。
`panic!` に変更する案も環境不足のたびに失敗扱いになり実用に耐えない。
現状の `eprintln!` + `return` によるスキップパターンが実用上最もバランスが取れているため、本 issue は修正を見送り closed とする。
