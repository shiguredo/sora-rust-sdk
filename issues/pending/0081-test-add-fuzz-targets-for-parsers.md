# バイナリパーサ向け fuzz ターゲットを将来検討する

- Priority: Low
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/test-add-fuzz-targets-for-parsers

## 目的

`Makefile` に `fuzzing` / `fuzzing-list` がある一方で `fuzz/` が無く、ターゲットは機能していない。
将来、バイナリパーサ（MP4 / NALU / zlib 等）向けに fuzz を入れる価値があるかを検討する。
シグナリング JSON や `ParsedProxyInfo` など Sora / アプリ設定前提の経路は対象外とする。

## 優先度根拠

Low。本 SDK の主な入力は Sora シグナリングとアプリ設定であり、インターネット上の任意バイト列をパースするサーバとは脅威モデルが異なる。
バイナリ系（ローカル MP4 等）には将来の余地があるが、リリースブロッカーではない。
Makefile の空参照は `fuzzing` ターゲット削除で足り、fuzz 整備の根拠にはならない。

## 現状

- `fuzz/` ディレクトリが存在しない。
- `Makefile` に `fuzzing` / `fuzzing-list` があるが、`cargo fuzz list` は失敗する。
- MP4 / NALU 周りでは過去に panic 経路が見つかっている（長さ付き NALU・OOB 等）。それらは個別バグ修正と単体テスト / PBT で扱える。
- 対象候補の多くは `pub(crate)` / private のため、fuzz 用に可視性ハックが必要になる。

## 設計方針（着手時のたたき台）

着手する場合も、次に絞る。

1. 対象はバイナリ系のみ（例: `length_prefixed_nalu_to_annex_b` / `decompress_zlib` / MP4 demux 周辺）。
2. `IncomingMessage` / `RpcResponse` / `ParsedProxyInfo` は fuzz 対象にしない（PBT・単体テストで足りる想定）。
3. 可視性は最小限の `test-internals` 等に留め、公開 API を広げない。
4. CI 常時実行は必須としない。ローカル / 手動の短時間実行で足りるかから検討する。
5. Makefile の壊れた `fuzzing` ターゲットは、着手前なら削除してよい。

## 完了条件

- バイナリ系に限定した fuzz ターゲットの要否を判断し、要なら最小構成で導入する。
- 不要と判断した場合は、Makefile の空 `fuzzing` を削除して本 issue を closed にする。

## pending 理由

Sora SDK 前提ではシグナリング・設定経路の任意入力 fuzz の投資対効果が薄い。
バイナリ系にも余地はあるが優先度は低く、可視性ハックと nightly / CI のコストが見合わない。
リリース前の必須作業ではないため、当面保留する。
