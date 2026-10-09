# libwebrtc の worker_thread 削除に追随する

- Created: 2026-09-15
- Completed: {YYYY-MM-DD}
- Branch: feature/remove-worker-thread
- Polished: {YYYY-MM-DD}

## 目的

libwebrtc の issue 558821261「Deprecate and remove PeerConnectionFactoryDependencies::worker_thread」の削除系 CL がマージされると、`PeerConnectionFactoryDependencies::worker_thread` が無くなる。

削除系の CL は 499302 / 501640 / 501720 / 502000 / 502500 / 502860 / 502940 / 502960 である。sora-rust-sdk に残っている worker thread の参照を削除し、削除後の libwebrtc に追随した `shiguredo_webrtc` でビルドできるようにする。

## 現状

- 0160-refactor-unify-worker-thread で専用の worker thread は廃止済みだが、`src/connection_context.rs` の `SoraConnectionContext::new_with_config` に `deps.set_worker_thread(&network)` が残る
- 依存は crates.io の `shiguredo_webrtc = "~0.154"` であり、git 依存ではない
- 前提条件は webrtc-rs が 558821261 を実装した libwebrtc に追随した `shiguredo_webrtc` をリリースしていることである。現時点では存在しないため、本 issue には着手できない
- 0160-refactor-unify-worker-thread の完了後に `git grep worker_thread` で残るのは 1 件のみである

## 設計方針

- `Cargo.toml` の `shiguredo_webrtc` を 558821261 の削除系 CL を含む libwebrtc に追随したバージョンへ更新する (`Cargo.lock` も更新する)
- `src/connection_context.rs` の `deps.set_worker_thread(&network)` を削除する。CL 501620 により worker thread を設定しない場合は network thread が使われる
- 公開 API のシンボルと挙動は変更しない
- 挙動変更・機能追加・バグ修正は行わない

## 完了条件

- `shiguredo_webrtc` が 558821261 の削除系 CL を含む libwebrtc に追随したバージョンに更新されている
- `git grep worker_thread` が issues 以外で 0 件になる
- `cargo fmt --all --check` が通る
- `cargo clippy --workspace -- -D warnings` が通る
- `cargo test --workspace` が通る
- `CHANGES.md` の `## develop` に追記する

## 変更対象

- `Cargo.toml` / `Cargo.lock`
- `src/connection_context.rs` の `SoraConnectionContext::new_with_config`
- `CHANGES.md`
