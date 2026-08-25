# `src/` の `.unwrap()` を `.expect("MESSAGE")` に置き換える

- Priority: Low
- Created: 2026-07-23
- Completed: 2026-07-29
- Model: Composer
- Branch: feature/refactor-replace-src-unwrap-with-expect
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S7 のうち、規約違反の `.unwrap()` 置換分。

## 目的

`shiguredo-rust` / AGENTS.md の方針に合わせ、`src/` 内のパニック経路をメッセージ付き `expect` にし、障害時の原因特定を容易にする。

## 優先度根拠

Low。

- 動作変更を意図しない機械的な置換が中心
- 件数は多いが 1 カテゴリに閉じている
- 正式リリースブロッカーではない

## 現状

`src/` 配下の `.unwrap()` 件数（2026-07-23）:

| ファイル | 件数 |
|---|---|
| `connection.rs` | 1 |
| `video_codec.rs` | 4 |
| `video_codecs/amf.rs` | 8 |
| `video_codecs/nvcodec.rs` | 8 |
| `video_codecs/v4l2.rs` | 17 |
| `video_codecs/vpl.rs` | 8 |
| 合計 | 46 |

本 issue の対象外:

- `examples/sumomo/src/main.rs` の日本語 `expect` 英語化（3 箇所。規模が小さく、必要なら直接修正する）
- `rust-toolchain.toml` / MSRV CI（#0050）
- `CODEBASE.md`（#0051）
- `issues/pending/0003` / `0007` の旧名更新（issue ファイルのみの軽微な修正）
- ホットパスの `.expect("encoder should exist")` 自体の設計見直し

## 設計方針

- パニックが仕様上あり得る箇所だけを対象にし、安易に `unwrap` を消して `Option` 地獄にしない
- メッセージは英語（ログ規約に合わせる）
- Mutex ロックなど「毒されない限り成功する」箇所は、失敗理由が分かる短いメッセージにする

## 完了条件

- `src/` から本番経路の `.unwrap()` が消える（テストモジュール内は方針を明示してよい）
- 置換後も挙動が変わらない
- `cargo test --workspace` / `clippy` が通る

## 解決方法

本番経路の `.unwrap()` を `.expect("...")` に置換した。

- `connection.rs`: `video_sender.as_mut()` を `expect("video_sender must exist after is_none check")` に変更
- `video_codec.rs` / `amf.rs` / `nvcodec.rs` / `vpl.rs` / `v4l2.rs`: `Mutex::lock()` の毒化を `expect("... should not be poisoned")` に変更
- `mp4.rs` のテスト内 `.unwrap()` / `.unwrap_err()` は対象外のまま残した
- `cargo test -p sora_sdk` と `cargo clippy -p sora_sdk --all-targets -- -D warnings` が通ることを確認した
