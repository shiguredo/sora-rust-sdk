# connection.rs を責務ごとのモジュールに分割する

- Created: 2026-08-20
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-split-connection-module
- Polished: {YYYY-MM-DD}

## 目的

保守性とレビュー性を向上させる。`src/connection.rs` は 5,231 行に達し、シグナリング・DataChannel・プロキシ・TLS・タイマーという複数の責務が 1 ファイルに凝集しており、変更時の影響範囲の特定とコードレビューが難しい。

## 現状

`src/connection.rs` に以下の責務が同居している:

- `SoraConnectionBuilder`（ビルダー）
- `SoraConnectionHandle`（外部制御ハンドル）
- `SoraConnection` 本体と `run()`（約 700 行。シグナリング状態遷移・SDP 処理・DataChannel 切替・RPC 相関）
- DataChannel 管理（`HandleDataChannelMessageResult` / `ManagedDataChannel` / 切替 readiness ヘルパー群）
- `SecureRandom`（マスキングキー・nonce 生成）
- `ParsedProxyInfo` / `ProxyStream`（HTTP プロキシ CONNECT トンネリング）
- `TimerManager` / `SdpOperationTimeoutTimer`（タイマー管理）
- `SetDescriptionObserverHandler` と関数内定義の Observer（`PcObserverHandler` / `AnsObsHandler` / `DcObsHandler`）

公開 API は `SoraConnection` / `SoraConnectionBuilder` / `SoraConnectionHandle` / `ParsedProxyInfo`（`src/lib.rs` の `pub use crate::connection::{...}`）であり、これらは維持する必要がある。

## 設計方針

- `src/connection.rs` を `src/connection/` ディレクトリのモジュール群に分割する
- 分割先は責務単位（builder / handle / run / data_channel / proxy / timer / secure_random 等）とし、`mod.rs` で再エクスポートする
- 公開 API のシンボルと挙動は変更しない（SemVer 非影響のリファクタリングのみ）
- 関数内定義の Observer 構造体はモジュールレベルへ切り出す
- 挙動変更・機能追加・バグ修正は行わない。既存テストが回帰の検証になる

## 完了条件

- `src/connection.rs` の責務がモジュールへ分割されている
- 公開 API のシンボルと挙動が不変である（`src/lib.rs` の re-export が変更されない）
- `cargo fmt --all -- --check` が通る
- `cargo clippy --workspace --all-targets -- -D warnings` が通る
- `cargo test -p sora_sdk` / `cargo test -p pbt` / `cargo test -p sumomo` が通る（既存テストが回帰検証になる）
