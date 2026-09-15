# SoraConnectionContext の worker thread を network thread に統一する

- Created: 2026-09-15
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-unify-worker-thread
- Polished: {YYYY-MM-DD}

## 目的

libwebrtc の issue 558821261「Deprecate and remove PeerConnectionFactoryDependencies::worker_thread」で worker thread が廃止される。`PeerConnectionFactoryDependencies::worker_thread` と `PeerConnectionFactoryInterface::worker_thread()` は将来削除される。

CL 501620「Default worker thread to network thread」と CL 502480「Warn when a distinct worker thread is configured」はマージ済みであり、削除系の CL (499302 / 501640 / 501720 / 502000 / 502500 / 502860 / 502940 / 502960) はレビュー中である。

`SoraConnectionContext` は専用の worker thread を生成して libwebrtc に渡している。専用の worker thread をやめて network thread を使うようにし、worker thread の削除に備える。

worker thread の API を無くす対応は、558821261 を実装した libwebrtc に追随した `shiguredo_webrtc` をリリースした後に 0161-remove-worker-thread で行う。

## 現状

- `src/connection_context.rs` の `SoraConnectionContext` が `_network: Thread` / `_worker: Thread` / `_signaling: Thread` を保持している
- `SoraConnectionContext::new_with_config` が次の 3 つのスレッドを生成して `start()` し、`deps` に設定している
  - network: `Thread::new_with_socket_server()` で生成し、`deps.set_network_thread(&network)` に渡す
  - worker: `Thread::new()` で生成し、`deps.set_worker_thread(&worker)` に渡す
  - signaling: `Thread::new()` で生成し、`deps.set_signaling_thread(&signaling)` に渡す
- `PeerConnectionFactory` を生成するのは `SoraConnectionContext::new_with_config` の 1 箇所のみである
- worker thread は factory の生成以外で使っていない (`blocking_call` の呼び出しと `factory.worker_thread()` の呼び出しは無い)
- フィールドの宣言順が本質的な意味を持つ。`ConnectionContext::~ConnectionContext()` は `DCHECK(signaling_thread_->IsCurrent())` であり、`connection_context` を `factory` より後に置くと factory 破棄時に Rust 側のスレッド上で `ConnectionContext` が破棄されて SIGABRT する。この順序制約を壊してはならない
- `skills/sora-rust-sdk/SKILL.md` の `SoraConnectionContext` の説明に「内部スレッド (network / worker / signaling) をまとめて保持」とあり、既知の制限事項にも「`SoraConnectionContext::new()` は内部スレッドを 3 本起動する」とある。どちらも更新対象である

## 設計方針

- worker 用の `Thread::new()` / `start()` を削除し、`_worker` フィールドを削除する
- `deps.set_worker_thread` には network thread を渡す。`deps.set_worker_thread` を呼ばないと現行の libwebrtc では PeerConnectionFactory が内部で専用スレッドを生成するため、明示的に network thread を設定する
- フィールド順のコメントを実態に合わせて更新する。`connection_context` を `factory` より前に置く制約は維持し、宣言順は変えない
- `skills/sora-rust-sdk/SKILL.md` の内部スレッドの記述を、実際に起動する本数に合わせて更新する
- `deps.set_worker_thread` の行の削除は、558821261 を実装した libwebrtc に追随した `shiguredo_webrtc` をリリースした後に 0161-remove-worker-thread で行う
- 公開 API のシンボルと挙動は変更しない (SemVer 非影響のリファクタリングのみ)
- 挙動変更・機能追加・バグ修正は行わない

## 完了条件

- worker thread の生成 (`Thread::new()`) と `start()` が無くなり、`_worker` フィールドが無くなる
- `deps.set_worker_thread` には network thread を渡す
- `connection_context` が `factory` より前にあり、フィールドの宣言順が変わっていない
- フィールド順のコメントが実態と一致している
- `skills/sora-rust-sdk/SKILL.md` の内部スレッドの記述が実装と一致している
- `cargo fmt --all --check` が通る
- `cargo clippy --workspace -- -D warnings` が通る
- `cargo test --workspace` が通る
- `CHANGES.md` の `## develop` に追記する

## 変更対象

- `src/connection_context.rs` の `SoraConnectionContext` と `SoraConnectionContext::new_with_config`
- `skills/sora-rust-sdk/SKILL.md`
- `CHANGES.md`

## テスト方針

- モックやスタブを使用しない
- 既存のテストが回帰の検証になる (`SoraConnectionContext::new()` は `src/connection.rs` と `src/video_codecs/mp4.rs` のテストで生成しており、`cargo test --workspace` で実行される)
