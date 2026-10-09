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

## pending にした理由

- ファイルが 5,300 行に達しており分割の必要性自体は変わらないが、どの分割案も分割の基準としてしっくりこないため、方針が固まるまで実装を保留にする
- 検討した案では、`SoraConnection` 本体の実装の一部（`run()` のシグナリング状態遷移、DataChannel 切替、ICE 設定、送信トラック設定）と、`SoraConnection` とは別の型（`SoraConnectionBuilder` / `SoraConnectionHandle`）が同じディレクトリに並ぶ。どのファイルが `SoraConnection` の一部なのかがパスから読み取れない
- さらに、SoraConnection に依存しない要素（`TimerManager`、`ClientStream` による TCP / TLS 接続、プロキシ CONNECT、`SecureRandom`、シグナリング URL 解析）も同じディレクトリに混在しており、「SoraConnection の分割」という意図と一致しない
- `observers.rs` に相当する Observer 実装は `SoraEvent` 経由でのみ接続本体と繋がり、`ParsedProxyInfo` は公開 API でありながらシグナリング接続の URL 解析と同じファイルに置かれており、責務の切り分けをどう表現するかが決まっていない
- 再開時は実装の前に、モジュール境界の基準（`SoraConnection` の実装 / 別の型 / 汎用ユーティリティのいずれに置くかとその命名）を先に確定させる必要がある
- 現状は保守性の課題が残るだけで、挙動やテストに問題はない
