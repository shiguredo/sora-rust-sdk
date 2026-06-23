# SoraConnection / TimerManager / PendingRpcRequest に `Drop` 実装を追加して `JoinHandle` を `abort()` する

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/add-drop-impl-to-connection-and-timer-manager
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`SoraConnection` / `TimerManager` の `Drop` 実装」分。

## 目的

`src/connection.rs` の `TimerManager` および `PendingRpcRequest` は `tokio::task::JoinHandle<()>` を保持しているが、`impl Drop` が無いため、保持しているオブジェクトが drop されただけでは内部のタスクは `abort()` されない。正常系では `clear_timer()` や RPC 応答処理で `abort()` が呼ばれるため通常は露見しないが、エラー経路 / シグナリング切断のタイミング / `SoraConnection` 自体の早期 drop ではタスクが残り続ける。SDK 利用者が `SoraConnection` を作り直して繰り返し利用する用途で残存タスクが累積し、tokio ランタイムの task 数とメモリが緩やかに増える。

本 issue では `TimerManager` / `PendingRpcRequest` (および必要に応じて `SoraConnection`) に `impl Drop` を追加して、残存 `JoinHandle` を確実に `abort()` する。

## 優先度根拠

Medium。

- 通常の clean shutdown 経路では `clear_timer()` / RPC 応答処理で `abort()` されるため、即座に問題化はしない
- ただし長時間稼働や `connect()` / `disconnect()` の繰り返し利用、エラー経路での切断では残存タスクが累積する
- SDK 利用者から見ると `SoraConnection` を drop した時点で内部リソースは解放されると期待するのが自然 (RAII の前提に反する)
- 修正は `impl Drop` を 2〜3 件追加するだけで完結し、API 互換性も維持される
- 正式リリース後でも追加可能だが、`Drop` の挙動はライブラリの暗黙の契約に近く、後から「drop で abort する」挙動を入れると利用者の予期しないタイミングで動作が変わる可能性があるため、`2026.1.0` の段階で確定させたい

## 現状

### `TimerManager` (`src/connection.rs:2087-2127`)

```rust
struct TimerManager {
    ping: Option<JoinHandle<()>>,
    pong_timeout: Option<JoinHandle<()>>,
    close_timeout: Option<JoinHandle<()>>,
    sender: mpsc::Sender<TimerId>,
}

impl TimerManager {
    fn clear_timer(&mut self, id: TimerId) {
        let handle = match id { ... };
        if let Some(handle) = handle.take() {
            handle.abort();
        }
    }
}
```

`clear_timer()` を明示的に呼ばずに `TimerManager` 自体が drop された場合、`ping` / `pong_timeout` / `close_timeout` の `JoinHandle` は `tokio::spawn` で起動したタスクへの参照を失うだけで、タスクは abort されずに `sleep().await` が完了するまで生存する。

### `PendingRpcRequest` (`src/connection.rs:554-557`)

```rust
struct PendingRpcRequest {
    response_tx: oneshot::Sender<Result<Option<RpcResponse>>>,
    timeout_handle: JoinHandle<()>,
}
```

RPC タイムアウト処理で生成される `timeout_handle` も `Drop` を持たず、`pending_rpc_responses` から remove される (応答到着 / 明示的なキャンセル) 経路で初めて drop される。`SoraConnection` 全体が drop された場合、`HashMap` に残存している `PendingRpcRequest` の `timeout_handle` は `abort()` されない。

### `SoraConnection` (`src/connection.rs:530-552`)

`SoraConnection` も `Drop` を持たないため、上記 2 つの問題を集約的に防ぐためのフックも無い。

## 設計方針

### 方針 A: 各構造体に個別に `Drop` を実装する (推奨)

```rust
impl Drop for TimerManager {
    fn drop(&mut self) {
        for handle in [self.ping.take(), self.pong_timeout.take(), self.close_timeout.take()]
            .into_iter()
            .flatten()
        {
            handle.abort();
        }
    }
}

impl Drop for PendingRpcRequest {
    fn drop(&mut self) {
        self.timeout_handle.abort();
    }
}
```

- 責務がそれぞれの構造体に閉じる
- `SoraConnection` 側に Drop を追加する必要が無い (`HashMap::drop` が各 `PendingRpcRequest::drop` を連鎖して呼ぶ)
- 既存の `clear_timer()` / `pending_rpc_responses.remove()` 経由の `abort()` は引き続き働き、Drop と二重呼び出しになっても `JoinHandle::abort()` は冪等

### 方針 B: `SoraConnection` に集約的な `Drop` を実装する

`SoraConnection` の `drop()` で `TimerManager` と `pending_rpc_responses` の全 handle をまとめて `abort()` する。

- 集約位置が明確になる一方で、`TimerManager` が `SoraConnection` 外部にコピー / move された場合に対応できない (実際にはそのような経路は無いが、将来の拡張で破綻するリスク)

方針 A を採用する。

## 完了条件

- `TimerManager` と `PendingRpcRequest` に `impl Drop` が追加されている
- 二重 `abort()` が呼ばれても panic / 副作用が無いことが確認できる (`JoinHandle::abort()` は冪等。標準ライブラリの仕様で担保)
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- 単体テストで `TimerManager` を drop すると spawn したタスクが「すぐに」終了することを確認する (`tokio::test` + `JoinHandle::is_finished()` を `tokio::task::yield_now().await` を挟みつつ確認するなど、モック / スタブを使わずに検証する)

## 解決方法

1. `src/connection.rs:2087` 付近の `TimerManager` 定義の直下に `impl Drop for TimerManager` を追加し、`ping` / `pong_timeout` / `close_timeout` を `Option::take()` してから `abort()` する
2. `src/connection.rs:554` 付近の `PendingRpcRequest` に `impl Drop for PendingRpcRequest` を追加し、`timeout_handle.abort()` を呼ぶ
3. 既存の `clear_timer()` および `pending_rpc_responses` の remove 経路は変更しない (二重 `abort()` 許容)
4. `tokio::test` で `TimerManager` の Drop が timer task を停止することを単体テスト化する (AGENTS.md「テストはコメントを重視すること」「テストのログメッセージは全て日本語にすること」を遵守)
