# `fuzz/` ディレクトリを新設し、パーサ層の fuzz ターゲットを整備する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/test-add-fuzz-targets-for-parsers
- Polished: {YYYY-MM-DD}

## 目的

`Makefile:16-24` に `fuzzing` / `fuzzing-list` ターゲットがあり `cargo +nightly fuzz run` を回す構成になっているが、リポジトリに `fuzz/` ディレクトリが存在せず `cargo fuzz list` はエラーになる。`fuzz/` を新設し、代表的なパーサ層の fuzz ターゲットを整備する。

## 優先度根拠

High。パーサ層 (`IncomingMessage::parse` / `RpcResponse::parse` / `ParsedProxyInfo::parse` / `length_prefixed_nalu_to_annex_b` 等) は外部から任意入力を受け取る経路で、`assert!` / `unwrap` / `expect` による panic が複数見つかっている (issue 0060 / 0061 / 0062 参照)。fuzz でこれらを網羅的に検出する仕組みが必要。Makefile が空を参照しており、機能不全のまま canary リリースは望ましくない。

## 現状

- `fuzz/` ディレクトリが存在しない (`ls fuzz` → not found)。
- `Makefile:16-24` に `fuzzing` / `fuzzing-list` ターゲット定義:

  ```make
  fuzzing:
      @for target in $$(cargo fuzz list); do \
          echo "=== Fuzzing $$target ==="; \
          cargo +nightly fuzz run $$target -- -max_total_time=30 || exit 1; \
      done
  fuzzing-list:
      cargo fuzz list
  ```
- 対象関数の可視性:
  - `IncomingMessage::parse`: pub(crate)
  - `RpcResponse::parse`: pub(crate)
  - `parse_signaling_url`: private
  - `length_prefixed_nalu_to_annex_b`: private
  - `decompress_zlib`: pub(crate)
  - `mask_url_userinfo`: private

対象関数の多くが pub(crate) / private のため、fuzz クレートから直接呼ぶには可視性の調整も必要。

## 設計方針

1. `cargo fuzz init` で `fuzz/` ディレクトリを新設する。
2. 以下の fuzz ターゲットを追加:
   - `fuzz_targets/incoming_message.rs` — `IncomingMessage::parse` (パニックしないこと)
   - `fuzz_targets/rpc_response.rs` — `RpcResponse::parse`
   - `fuzz_targets/parsed_proxy_info.rs` — `ParsedProxyInfo::parse`
   - `fuzz_targets/length_prefixed_nalu.rs` — `length_prefixed_nalu_to_annex_b`
   - `fuzz_targets/decompress_zlib.rs` — `decompress_zlib` (round-trip / 破損データ)
3. 対象関数の可視性は、`#[cfg(feature = "test-internals")] pub` を追加するか、`sora_sdk` の別途 `pub fn __fuzz_*` を追加する形で fuzz クレートから呼べるようにする (issue 0049 の pbt/tests 対応と併せて設計)。
4. `Makefile` は既存のまま活用できる。CI で `cargo +nightly fuzz run` を短時間 (30 秒) で回すジョブを追加するかは issue 0050 で検討。
5. コーパスは初期は空、CI で発見された seed は commit する運用にする。

## 完了条件

- `fuzz/` ディレクトリが存在し、`cargo fuzz list` が上記 5 ターゲットを列挙する。
- 各ターゲットが `cargo +nightly fuzz run <target> -- -max_total_time=30` でエラーなく起動できる。
- `make fuzzing` が意図通り動作する。
- 発見された panic があれば別 issue で追跡する。
