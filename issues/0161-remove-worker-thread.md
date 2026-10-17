# libwebrtc の worker_thread 削除に追随する

- Created: 2026-09-15
- Completed: 2026-10-17
- Branch: feature/remove-worker-thread
- Polished: {YYYY-MM-DD}

## 目的

libwebrtc の issue 558821261「Deprecate and remove PeerConnectionFactoryDependencies::worker_thread」の削除系 CL がマージされると、`PeerConnectionFactoryDependencies::worker_thread` が無くなる。

削除系の CL は 499302 / 501640 / 501720 / 502000 / 502500 / 502860 / 502940 / 502960 である。sora-rust-sdk に残っている worker thread の参照を削除し、削除後の libwebrtc に追随した `shiguredo_webrtc` でビルドできるようにする。

## 現状

- 0160-refactor-unify-worker-thread で専用の worker thread は廃止済みだが、`src/connection_context.rs` の `SoraConnectionContext::new_with_config` に `deps.set_worker_thread(&network)` が残っていた
- 依存は crates.io の `shiguredo_webrtc` であり、git 依存ではない
- 前提条件は webrtc-rs が 558821261 を実装した libwebrtc に追随した `shiguredo_webrtc` をリリースしていることであった
- 0160-refactor-unify-worker-thread の完了後、`git grep worker_thread` で残るのは 1 件のみであった

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

## 解決方法

`shiguredo_webrtc` を 558821261 の削除系 CL に追随したバージョンに更新し、worker thread の設定を削除した。

- `Cargo.toml` / `Cargo.lock` の `shiguredo_webrtc` を `0.156.1-canary.1` に更新した。このバージョンには削除系 CL が含まれており、`shiguredo_webrtc` 側に `set_worker_thread` は存在しない
- `src/connection_context.rs` の `SoraConnectionContext::new_with_config` から `deps.set_worker_thread(&network)` を削除した。`PeerConnectionFactoryDependenciesParams` には `network_thread` / `signaling_thread` / `adm` を渡しており、依存の組み立てはこれで完結する
- worker thread を設定しない場合は network thread が使われるため、挙動は変わらない
- `CHANGES.md` の `## develop` に `shiguredo_webrtc` のアップデートとして追記した

確認:

- `git grep worker_thread` が issues 以外で 0 件であることを確認した
- `cargo fmt --all --check` / `cargo clippy --workspace -- -D warnings` / `cargo test --workspace` が通ることを確認した
