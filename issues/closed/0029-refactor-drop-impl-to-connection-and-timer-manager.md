# TimerManager / PendingRpcRequest に `Drop` 実装を追加して `JoinHandle` を `abort()` する

- Priority: Medium
- Created: 2026-06-23
- Completed: 2026-06-25
- Model: Opus 4.7
- Branch: feature/refactor-drop-impl-to-connection-and-timer-manager
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`TimerManager` / `PendingRpcRequest` の `Drop` 実装」分。

## 目的

`src/connection.rs` の `TimerManager` および `PendingRpcRequest` は `tokio::task::JoinHandle<()>` を保持しているが、`impl Drop` が無いため、保持しているオブジェクトが drop されただけでは内部のタスクは `abort()` されない。

- `TimerManager` は `run()` のローカル変数 (`connection.rs:839`)。正常系では `clear_timer()` が呼ばれるが、`run()` 内の `?` 演算子による早期リターン (数十箇所存在する)、WebSocket 切断による `break` (`connection.rs:875`)、disconnect コマンドによる `break` (`connection.rs:938`) のいずれでも `timers` が drop され、設定中の ping / pong_timeout / close_timeout タイマーが abort されずに `sleep().await` 完了まで生存し続ける。またリダイレクト時 (`connection.rs:1182`) には `timers = TimerManager::new(...)` で古い `TimerManager` が上書き drop され、この経路でも同様にタイマーが残留する。
- `PendingRpcRequest` は `SoraConnection` の `pending_rpc_responses: HashMap<u64, PendingRpcRequest>` に保持される。`SoraConnection` 全体が drop された場合、`HashMap` に残存している `PendingRpcRequest` の `timeout_handle` は `abort()` されない。

SDK 利用者が `SoraConnection` を作り直して繰り返し利用する用途で残存タスクが累積し、tokio ランタイムの task 数とメモリが緩やかに増える。

本 issue では `TimerManager` / `PendingRpcRequest` に `impl Drop` を追加して、残存 `JoinHandle` を確実に `abort()` する。

## 優先度根拠

Medium。

- 通常の clean shutdown 経路では `clear_timer()` / RPC 応答処理で `abort()` されるため、即座に問題化はしない
- ただし長時間稼働や `connect()` / `disconnect()` の繰り返し利用、エラー経路での切断では残存タスクが累積する
- SDK 利用者から見ると `SoraConnection` を drop した時点で内部リソースは解放されると期待するのが自然 (RAII の前提に反する)
- 修正は `impl Drop` を 2 件追加し `PendingRpcRequest` の内部フィールド型を変更するだけで完結する。公開 API のシグネチャ（型・メソッド名・引数）に変更はなく、意味論的にも drop 時のリソース解放が追加される（暗黙的に破棄されていただけのタスクが明示的に abort される）だけであり、既存の利用コードを壊さない
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

`SoraConnection` は `pending_rpc_responses: HashMap<u64, PendingRpcRequest>` を保持しているが、`Drop` を持たない。`run()` の各種終了経路（早期リターン / break）において `pending_rpc_responses` がクリアされないまま `SoraConnection` が drop されると、残存する `PendingRpcRequest` の `timeout_handle` は `abort()` されずに放置される。なお `TimerManager` は `SoraConnection` のフィールドではなく `run()` 内のローカル変数であるため、`SoraConnection` の Drop で `TimerManager` の問題を統括的に対処することはできない。

## 設計方針

### 方針 A: 各構造体に個別に `Drop` を実装する (推奨)

```rust
impl Drop for TimerManager {
    fn drop(&mut self) {
        if let Some(h) = self.ping.take() {
            h.abort();
        }
        if let Some(h) = self.pong_timeout.take() {
            h.abort();
        }
        if let Some(h) = self.close_timeout.take() {
            h.abort();
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
- `TimerManager` は `run()` のローカル変数であるため、`SoraConnection` 側で集約対処するには `TimerManager` を `SoraConnection` のフィールドに引き上げるリファクタが必要となり、影響範囲が不必要に大きくなる
- `SoraConnection` 側に Drop を追加する必要が無い (`HashMap::drop` が各 `PendingRpcRequest::drop` を連鎖して呼ぶ)
- 既存の `clear_timer()` / `pending_rpc_responses.remove()` 経由の `abort()` は引き続き働き、Drop と二重呼び出しになっても `JoinHandle::abort()` は冪等（tokio ドキュメントで言明されている）

#### `PendingRpcRequest` の構造体設計変更

`PendingRpcRequest` に `Drop` を実装すると、既存コードで `response_tx: oneshot::Sender<...>` が部分ムーブされている箇所 (`connection.rs:918` の `pending.response_tx.send(...)`、`connection.rs:1821` の `pending.response_tx.send(...)`) がコンパイルエラーになる (Rust では `Drop` を実装する型のフィールドを部分ムーブできない)。

このため `response_tx` を `Option<oneshot::Sender<Result<Option<RpcResponse>>>>` に変更する。各送信箇所では `response_tx.take().unwrap().send(...)` に書き換える。`Drop` 実装は `timeout_handle.abort()` のみ行い、`response_tx` の `Option` が `None` の場合は単に drop される。

なお、`SoraConnection` 全体の drop により残存する `PendingRpcRequest` が一括 drop される際、`response_tx` が `Some(sender)` のまま破棄される。この場合、受信側の `send_rpc_command` は `RecvError` → `Error::CommandResponseMissing` を受け取る。これは改善前と同じ挙動であり、また `Drop` で明示的にエラーを送信しようとすると、`response_tx` を操作するために `Drop` が `&mut self` を取れない（`drop(&mut self)` のみ）という根本的な制約があるため、現状維持を採用する。

（方針 B: `SoraConnection` に集約的な `Drop` を実装することも検討したが、`TimerManager` が `run()` のローカル変数であるため `SoraConnection` の Drop で対処できない。よって方針 A を採用する。）

## 完了条件

- `TimerManager` と `PendingRpcRequest` に `impl Drop` が追加されている
- `PendingRpcRequest::response_tx` が `Option<oneshot::Sender<...>>` に変更され、既存の送信箇所が `take().unwrap().send(...)` に置き換わっている
- 二重 `abort()` が呼ばれても panic / 副作用が無いことが確認できる (`JoinHandle::abort()` は tokio ドキュメントで冪等と明記されている)
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- 以下の単体テストが `src/connection.rs` の `#[cfg(test)] mod tests` 内に追加され、すべて通過すること（モック / スタブ禁止）:
  - `TimerManager` に ping/pong_timeout/close_timeout を設定 → drop → 対応する `timer_rx` に短いタイムアウト内でメッセージが届かないことの確認（タイマーが abort された証拠）
  - `TimerManager` をタイマー未設定 (`Option` がすべて `None`) の状態で drop → panic しないことの確認
  - `TimerManager` にタイマーを設定 → `clear_timer()` → drop → 二重 abort でも panic しないことの確認
  - `PendingRpcRequest` を生成 → drop → `timeout_handle` が abort されたことを確認する。`JoinHandle` は `Clone` を実装していないため、`is_finished()` での直接検証はできない。代わりに、drop 後に `event_tx` に `RpcTimeout` が届かないことで間接的に検証する
  - タイマー期間 0 ms のケース — `tokio::task::yield_now().await` を複数回挿入して sleep 完了と abort の競合を意図的に発生させ、panic しないことの確認
- `JoinHandle` は `Clone` を実装しておらず、`drop()` 後に内部の `JoinHandle` にアクセスできない。このため `TimerManager` のテストでは `mpsc::channel` の受信側を用いた間接検証を行い、`PendingRpcRequest` のテストもチャネル経由で検証する

## 解決方法

1. `src/connection.rs:2087` 付近の `TimerManager` 定義の直下に `impl Drop for TimerManager` を追加し、`ping` / `pong_timeout` / `close_timeout` を `Option::take()` してから個別に `abort()` する
2. `src/connection.rs:554` 付近の `PendingRpcRequest` の `response_tx` フィールドを `Option<oneshot::Sender<Result<Option<RpcResponse>>>>` に変更する
3. `PendingRpcRequest` に `impl Drop for PendingRpcRequest` を追加し、`timeout_handle.abort()` を呼ぶ（`response_tx` が `Option` なので部分ムーブ問題は解消されている）
4. `connection.rs:917` の `if let Some(pending) =` を `if let Some(mut pending) =` に変更し、`pending.response_tx.send(...)` を `pending.response_tx.take().unwrap().send(...)` に変更する
5. `connection.rs:1818` の `let Some(pending) =` を `let Some(mut pending) =` に変更し、`pending.response_tx.send(...)` を `pending.response_tx.take().unwrap().send(...)` に変更する
6. `connection.rs:964` の `PendingRpcRequest` 構築時に `response_tx: Some(response_tx)` とする
7. 変更が必要なのは `PendingRpcRequest::response_tx` フィールドへのアクセスのみ。`connection.rs:956` の notification 分岐 (`let _ = response_tx.send(Ok(None))`) と `connection.rs:971` のエラー分岐 (`let _ = response_tx.send(Err(e))`) は `PendingRpcRequest` 構築前のローカル変数に対する操作のため、変更不要
8. 既存の `clear_timer()` は変更しない (二重 `abort()` 許容)
9. `src/connection.rs` の `#[cfg(test)] mod tests` 内（`connection.rs:2490` 付近の既存テスト群の後に追加）に、完了条件に記載された全テストケースを実装する。AGENTS.md「テストはコメントを重視すること」「テストのログメッセージは全て日本語にすること」「モックやスタブは絶対に利用しないこと」を遵守する

### テスト戦略

- **単体テスト**: `Drop` によるタスク停止と二重 abort の安全性を検証。テストは `src/connection.rs` の `#[cfg(test)] mod tests` に追加する（`TimerManager` はプライベート型のため外部テストファイルからアクセス不可）
- **PBT**: 適用しない。`Drop` の検証は副作用が本質であり、proptest のランダム入力による property 検証に馴染まない
- **Fuzzing**: 適用しない。任意入力を受け付ける経路ではない

## 解決方法

- `TimerManager` に `Drop` を実装し、全 `JoinHandle` を `abort()` する
- `PendingRpcRequest::response_tx` を `Option` 化し `Drop` で `timeout_handle.abort()` する
- 単体テスト追加

### 修正ファイル
- `src/connection.rs`
