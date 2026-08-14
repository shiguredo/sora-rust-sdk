# sumomo の --features raw-player ビルドがコンパイルエラーになる問題を直す

- Priority: Medium
- Created: 2026-08-10
- Completed: 2026-08-15
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-raw-player-build
- Polished: {YYYY-MM-DD}

## 目的

`--features raw-player` で sumomo をビルドできるようにする。

## 現状

`examples/sumomo/src/main.rs` の `run_with_raw_player` 内で `raw_player::quit()` を unsafe ブロックなしで呼んでいる。`quit()` は unsafe 関数のため、`cargo build -p sumomo --features raw-player` が `E0133: call to unsafe function is unsafe and requires unsafe block` で失敗する。develop の最新でも再現する pre-existing の問題で、0117 の対応とは無関係。

## 設計方針

- `raw_player::quit()` の呼び出しを unsafe ブロックで包む
- この API が unsafe である理由（呼び出し条件）をコメントで明記する

## 完了条件

- `cargo build -p sumomo --features raw-player` が成功する
- `cargo test --workspace` が成功する

## 変更対象

- `examples/sumomo/src/main.rs`

## 解決方法

- 2026-08-10 のコミット `43ec626` (PR #52) で、`examples/sumomo/src/main.rs` の `run_with_raw_player` を廃止し、raw-player 描画を `examples/sumomo/src/raw_player_renderer.rs` の `RawPlayerRenderer` に移した
- `raw_player::quit()` は `SdlCleanupGuard` の `Drop` 内で呼ぶように変更し、unsafe ブロックで包んで `quit()` を呼び出す条件を Safety コメントで明記した
- これにより `examples/sumomo/src/main.rs` の `run_with_raw_player` 内で unsafe なしに `raw_player::quit()` を呼ぶ経路が消滅し、`cargo build -p sumomo --features raw-player` が成功するようになった
