# コールバック関数から Sync トレイト境界を除去する

- Priority: Medium
- Created: 2026-07-03
- Completed: 2026-07-04
- Model: DeepSeek-V4 Pro
- Branch: feature/refactor-remove-sync-from-callbacks
- Polished: 2026-07-03

## 目的

コールバック関数に付与されている `Sync` トレイト境界は実際には不要である。コールバックは非同期タスク間で共有されるため `Send` 境界は必須だが、同時に複数スレッドから呼び出されることはないため `Sync` は過剰な制約になっている。さらに API の `Sync` 要求は利用者側のコードに波及し、`AppEventSender` のようなトレイトにも不要な `Sync` 境界を強いている。すべての `Sync` 境界を除去し、利用者が `Sync` を実装しない型をキャプチャしたクロージャを自由に渡せるようにする。

## 優先度根拠

- `Sync` 境界は利用者側の設計に波及するトレイト境界であり、不要な制約を早期に除去することで API の健全性を高められる
- 現時点で不具合報告はないが、放置すると利用者が不必要な `Sync` 実装を強いられ続ける
- `Arc` を `Box` に変更せざるを得ないのは `Arc<dyn Fn + Send>` が `Send` にならないという Rust の型システム上の制約のためであり、`Sync` 除去と `Box` 化は不可分な変更である

## 現状

`src/connection.rs` の `SoraConnectionBuilder` 構造体が保持する 12 種のコールバックと `ice_server_url_configurer`（計 13 種）はすべて `Arc<dyn Fn(...) + Send + Sync>` で保持されている。また、対応するビルダーセッターメソッドの `where` 節もすべて `Fn(...) + Send + Sync + 'static` を要求している。

### 変更対象の一覧

#### 型エイリアス（src/connection.rs:66）

- `IceServerUrlConfigurer`: `dyn Fn(&mut IceServer, &[String]) + Send + Sync`

#### SoraConnectionBuilder のフィールド（src/connection.rs:78-118）

| フィールド名 | 行 | 変更前 |
|---|---|---|
| `on_signaling_message` | 78 | `Arc<dyn Fn(SignalingType, SignalingDirection, &str) + Send + Sync>` |
| `on_notify` | 79 | `Arc<dyn Fn(&str) + Send + Sync>` |
| `on_push` | 80 | `Arc<dyn Fn(&str) + Send + Sync>` |
| `on_track` | 81 | `Arc<dyn Fn(RtpTransceiver) + Send + Sync>` |
| `on_remove_track` | 82 | `Arc<dyn Fn(RtpReceiver) + Send + Sync>` |
| `on_switched` | 83 | `Arc<dyn Fn() + Send + Sync>` |
| `on_websocket_close` | 85 | `Arc<dyn Fn(Option<u16>, &str) + Send + Sync>` |
| `on_message` | 89 | `Arc<dyn Fn(&str, &[u8]) + Send + Sync>` |
| `on_data_channel` | 92 | `Arc<dyn Fn(&str) + Send + Sync>` |
| `on_data_channel_open` | 93 | `Arc<dyn Fn(&str) + Send + Sync>` |
| `on_data_channel_message` | 95 | `Arc<dyn Fn(&str, &[u8]) + Send + Sync>` |
| `on_data_channel_close` | 96 | `Arc<dyn Fn(&str) + Send + Sync>` |
| `ice_server_url_configurer` | 118 | `Option<Arc<IceServerUrlConfigurer>>`（`IceServerUrlConfigurer` 経由で `Sync` を含む。他のコールバックと異なり `Option` でラップされている） |

#### ビルダーセッターメソッドの where 節（src/connection.rs:189-499）

全 13 メソッドが `F: Fn(...) + Send + Sync + 'static` を要求している。該当行:

- `on_signaling_message` (191), `on_notify` (203), `on_push` (215), `on_track` (227), `on_remove_track` (238), `on_switched` (248), `on_websocket_close` (260), `on_message` (274), `on_data_channel` (285), `on_data_channel_open` (296), `on_data_channel_message` (307), `on_data_channel_close` (318), `ice_server_url_configurer` (499)

#### PcObserverHandler のフィールド（src/connection.rs:841-842）

- `on_track`: `Arc<dyn Fn(RtpTransceiver) + Send + Sync>`
- `on_remove_track`: `Arc<dyn Fn(RtpReceiver) + Send + Sync>`

#### handle_datachannel_state の引数（src/connection.rs:1845-1846）

- `on_data_channel_open`: `&Arc<dyn Fn(&str) + Send + Sync>`
- `on_data_channel_close`: `&Arc<dyn Fn(&str) + Send + Sync>`

#### handle_datachannel_message の引数（src/connection.rs:1925-1929）

- `on_signaling_message`: `&Arc<dyn Fn(SignalingType, SignalingDirection, &str) + Send + Sync>`
- `on_notify`: `&Arc<dyn Fn(&str) + Send + Sync>`
- `on_push`: `&Arc<dyn Fn(&str) + Send + Sync>`
- `on_message`: `&Arc<dyn Fn(&str, &[u8]) + Send + Sync>`
- `on_data_channel_message`: `&Arc<dyn Fn(&str, &[u8]) + Send + Sync>`

#### e2e-tests/src/test_connection.rs:173-175

- `ice_server_url_configurer` の where 節: `F: Fn(&mut IceServer, &[String]) + Send + Sync + 'static`

#### configure_ice_server_urls の引数（src/connection.rs:1566）

- `configurer: Option<&Arc<IceServerUrlConfigurer>>`（`IceServerUrlConfigurer` 経由で間接的に `Sync` を含む）

#### examples/sumomo/src/main.rs:76

- `trait AppEventSender: Clone + Send + Sync + 'static` — コールバッククロージャにキャプチャされるイベント送信トレイト。`SoraConnectionBuilder` のセッターが `Sync` を要求するために間接的に `Sync` が必要になっている。コールバック側の `Sync` が外れれば `AppEventSender` の `Sync` も不要になる。

### 同一ファイル内でコールバックが clone されている箇所

- `SoraConnection::new()`（`src/connection.rs:836-837`）で `on_track` と `on_remove_track` が `.clone()` され、`PcObserverHandler` に渡されている。
- `run()` 内（`src/connection.rs:969-978`）で残りの 10 個のコールバック（`on_signaling_message`, `on_notify`, `on_push`, `on_switched`, `on_websocket_close`, `on_message`, `on_data_channel`, `on_data_channel_open`, `on_data_channel_message`, `on_data_channel_close`）が `.clone()` され、ローカル変数としてイベントループ内で使われている。

### コールバックの使用状況

- `on_track` / `on_remove_track` は `PcObserverHandler` からのみ呼び出される。`SoraConnection::run()` のイベントループでは使用されない（`SoraEvent` 列挙型に `Track` / `RemoveTrack` バリアントは存在せず、`run()` のローカル変数抽出にも含まれない）。
- 他の 10 個のコールバックは `run()` のイベントループ内および `handle_datachannel_state` / `handle_datachannel_message` から呼び出される。
- `ice_server_url_configurer` は `apply_pc_configuration` から参照として使われる。初回 offer と re-offer の両方で呼ばれるため、接続ライフサイクル中ずっと生存している必要がある。

## 設計方針

### Arc は維持できず Box 化が必須

`Arc<T>: Send` が成立するためには `T: Send + Sync` が必要である。したがって `dyn Fn(...) + Send` は `Sync` でないため、`Arc<dyn Fn(...) + Send>` は `Send` にならず、`tokio::spawn` にムーブできなくなる。このため、`Sync` を外すだけでは不十分で、`Arc` から `Box` への変更が必須である。

### フィールド型: `Option<Box<dyn Fn(...) + Send>>` への変更

`Box` は `Clone` を実装しないため、`.clone()` による共有が不可能になる。代わりに以下の対応を行う:

1. 全コールバックフィールドを `Option<Box<dyn Fn(...) + Send>>` に変更する（デフォルト値は `Some(Box::new(|...| {}))`、セッターは `self.field = Some(Box::new(handler))`）。
2. `SoraConnection::new()`（`src/connection.rs:836-837`）では `config.on_track.take()` / `config.on_remove_track.take()` で値を取り出し、`PcObserverHandler` へムーブする。`PcObserverHandler` の `on_track` / `on_remove_track` フィールドも `Box<dyn Fn(...) + Send>` に変更する。
3. `run()` 内（`src/connection.rs:969-978`）では `self.config.on_signaling_message.take().expect(...)` のように各コールバックを `take()` してムーブする。
4. `ice_server_url_configurer` は re-offer 時に複数回呼ばれるため take せず、`Option::as_ref()` で参照を使い続ける。`configure_ice_server_urls` の引数型を `Option<&Box<IceServerUrlConfigurer>>` に変更する。

### コールバックの格納先

take で取り出したコールバックの格納先は以下の方針とする:

- `on_track` / `on_remove_track`: `PcObserverHandler` のフィールドに格納（変更前と同様）
- 残りの 10 個のコールバック: `run()` のローカル変数として格納（`SoraConnection` 構造体に新たにフィールドを追加する必要はない）
- `ice_server_url_configurer`: `SoraConnectionBuilder` のフィールドに残したまま `as_ref()` で参照する

### Drop 順序の制約維持

`SoraConnection` の Drop 順序は `pc_observer` → `pc` → `config` であり、これは変更しない。コールバックを `config` から取り出した後も、`PcObserverHandler` にムーブされたコールバックは `pc_observer` が生存する限り有効である。`ice_server_url_configurer` を `config` に残したまま参照することで、Drop 順序の制約を維持する。

## 完了条件

- `src/connection.rs` の全コールバック型定義・セッターメソッドの `where` 節から `Sync` 境界が除去されている
- フィールド型が `Arc<dyn Fn(...) + Send + Sync>` から `Option<Box<dyn Fn(...) + Send>>` に変更されている（`ice_server_url_configurer` は `Option<Box<IceServerUrlConfigurer>>`）
- `build()` 内の `on_track` / `on_remove_track` の `.clone()` 呼び出しが `take()` に置き換えられ、`PcObserverHandler` のフィールド型が `Box<dyn Fn(...) + Send>` に変更されている
- `run()` 内の 10 個のコールバックの `.clone()` 呼び出しが `take().expect(...)` に置き換えられている
- `PcObserverHandler` のフィールド、`handle_datachannel_state` / `handle_datachannel_message` / `configure_ice_server_urls` の引数型も追従している。引数型は `&Box<dyn Fn>` ではなく `&dyn Fn` を基本とする
- `ice_server_url_configurer` は take せず `as_ref()` で参照を使い続け、re-offer 時にも動作する
- `SoraConnectionBuilder::new()` の全デフォルト値が `Some(Box::new(|...| {}))` に変更されている
- `e2e-tests/src/test_connection.rs:175` の where 節から `Sync` が除去され、`with_callbacks` 内のクロージャも追従している
- `examples/sumomo/src/main.rs:76` の `AppEventSender` トレイトから `Sync` 境界が除去されている（`Clone` 境界は据え置き。複数コールバックで `event_tx` を共有するために必要）
- `cargo build --workspace` が全 feature 組合せで成功する
- `cargo test --workspace` が全 feature 組合せで成功する（`connection.rs` 内の単体テスト `ice_server_url_configurer_can_add_only_turn_tcp_udp_urls` と `ice_server_url_configurer_skips_server_when_no_url_is_added` の 2 件の `Arc` → `Box` 変更を含む）
- examples/sumomo がコンパイル可能である
- `CHANGES.md` の `## develop` に [CHANGE] エントリが追記されている

## 解決方法

### A. 型エイリアスとフィールド定義の変更（`src/connection.rs`）

1. `IceServerUrlConfigurer` 型エイリアス（line 66）から `Sync` を除去する: `dyn Fn(&mut IceServer, &[String]) + Send`
2. `SoraConnectionBuilder` の全コールバックフィールド（lines 78-118）の型を以下のように変更する:
   - `Arc<dyn Fn(...) + Send + Sync>` → `Option<Box<dyn Fn(...) + Send>>`
   - `Option<Arc<IceServerUrlConfigurer>>` → `Option<Box<IceServerUrlConfigurer>>`
3. `PcObserverHandler` の `on_track` / `on_remove_track` フィールド（lines 841-842）を `Box<dyn Fn(...) + Send>` に変更する
4. `SoraConnectionBuilder::new()`（lines 140-151）のデフォルト値を `Arc::new(|...| {})` から `Some(Box::new(|...| {}))` に変更する。`ice_server_url_configurer` は `None` のまま

### B. セッターメソッドの変更（`src/connection.rs`）

5. 全 13 メソッドの where 節から `Sync` を除去する（`Fn(...) + Send + Sync + 'static` → `Fn(...) + Send + 'static`）
6. `self.field = Arc::new(handler)` → `self.field = Some(Box::new(handler))` に変更する

### C. `SoraConnection::new()` の変更（`src/connection.rs:836-884`）

7. `fn new(config: SoraConnectionBuilder)` を `fn new(mut config: SoraConnectionBuilder)` に変更する（`take()` に `&mut self` が必要なため）
8. `config.on_track.clone()` / `config.on_remove_track.clone()` を `config.on_track.take().expect("on_track は new() でデフォルト値が設定されている")` / `config.on_remove_track.take().expect("on_remove_track は new() でデフォルト値が設定されている")` に変更する
9. `PcObserverHandler` への値のムーブはそのまま

### D. `run()` の変更（`src/connection.rs:946-`）

10. lines 969-978 の 10 個の `.clone()` 呼び出しを `self.config.<field>.take().expect("<field> は new() でデフォルト値が設定されている")` に変更する（各フィールド名を expect メッセージに含める）
11. ローカル変数の型は `Box<dyn Fn(...) + Send>` となる
12. `ice_server_url_configurer` は take しない（re-offer 時に再利用されるため）
13. `handle_datachannel_state` の呼び出し（lines 1112, 1120）: `&on_data_channel_open` → `on_data_channel_open.as_ref()`、`&on_data_channel_close` → `on_data_channel_close.as_ref()` に変更する
14. `handle_datachannel_message` の呼び出し（line 1094）: `&on_signaling_message` → `on_signaling_message.as_ref()`、他 4 引数も同様に `.as_ref()` に変更する

### E. 内部メソッドの引数型変更（`src/connection.rs`）

15. `handle_datachannel_state`（line 1842）: 引数 `on_data_channel_open` と `on_data_channel_close`（2 引数）の型をそれぞれ `&Arc<dyn Fn(&str) + Send + Sync>` → `&dyn Fn(&str) + Send` に変更する
16. `handle_datachannel_message`（line 1921）: 引数 `on_signaling_message`, `on_notify`, `on_push`, `on_message`, `on_data_channel_message`（5 引数）の型をそれぞれ `&dyn Fn(...) + Send` に変更する
17. `configure_ice_server_urls`（line 1563）: 引数の型を `Option<&Arc<IceServerUrlConfigurer>>` → `Option<&Box<IceServerUrlConfigurer>>` に変更する
18. `apply_pc_configuration`（line 1577）: `self.config.ice_server_url_configurer.as_ref()` の呼び出しはそのまま（`Option<Box<...>>` の `as_ref()` が使えるため）

### F. テストコードの変更（`src/connection.rs:2750-`）

19. `ice_server_url_configurer_can_add_only_turn_tcp_udp_urls` テスト（line 2794）の `Arc::new(...)` を `Box::new(...)` に、`Arc<IceServerUrlConfigurer>` を `Box<IceServerUrlConfigurer>` に変更する
20. `ice_server_url_configurer_skips_server_when_no_url_is_added` テスト（line 2812）も同様に `Arc` → `Box` に変更する

### G. e2e テストとサンプルの変更

21. `e2e-tests/src/test_connection.rs`（line 175）の `ice_server_url_configurer` セッターの where 節から `Sync` を除去する
22. `examples/sumomo/src/main.rs`（line 76）の `AppEventSender` トレイトから `Sync` を除去する（`Clone + Send + Sync + 'static` → `Clone + Send + 'static`）。`Clone` は複数コールバックで `event_tx` を共有するために引き続き必要

### H. CHANGES.md への追記

23. `CHANGES.md` の `## develop` に以下を追記する:
    - `[CHANGE] コールバック関数から Sync トレイト境界を除去する`
      - `SoraConnectionBuilder` の全セッターメソッドの where 節から `Sync` を除去する
      - コールバックの内部保持型を `Arc<dyn Fn + Send + Sync>` から `Box<dyn Fn + Send>` に変更する
      - `sumomo` の `AppEventSender` トレイトから `Sync` 境界を除去する

### I. `#[expect(clippy::type_complexity)]` 属性の更新

24. `handle_datachannel_message`（line 1920）の `#[expect(clippy::too_many_arguments, clippy::type_complexity)]` のうち、`type_complexity` は引数型の簡略化により不要になる可能性があるため、変更後に警告が出ないか確認し、不要なら `too_many_arguments` のみを残す。フィールド定義（lines 77, 84, 88, 94）の 4 件は型の複雑さが同等のため据え置く。

### 補足: エッジケースと検証

25. `SoraConnectionHandle::disconnect()` 後も `run()` 内の `disconnect` コマンド処理で `on_data_channel_close` が呼ばれるが、take で取り出した後はローカル変数として生きているため問題ない
26. コールバッククロージャを複数の `SoraConnection` で共有するパターンは、`Box` 化後は不可能になるが、このユースケースは想定されていない
27. 本変更は feature-gated ではないため、全 feature 組合せでのビルド確認が可能である
28. `run()` ローカル変数に take されたコールバックは `run()` が `mut self` を取るため、`self`（とその内部の `config`）がドロップされるより前に take が実行され、`run()` のスコープ全体で生存する

## 解決方法

- `IceServerUrlConfigurer` 型エイリアスから `Sync` を除去する
- `SoraConnectionBuilder` の全コールバックフィールド (12 件) を `Arc<dyn Fn + Send + Sync>` から `Option<Box<dyn Fn + Send>>` に変更し、`ice_server_url_configurer` を `Option<Arc<...>>` から `Option<Box<...>>` に変更する
- フィールド型・`DataChannelMessageCallbacks` 構造体の clippy `type_complexity` 警告に対処するため、全コールバックに個別の型エイリアス (`OnSignalingMessageCallback` 等 12 種) を追加し、`#[expect(clippy::type_complexity)]` 4 件を削除する
- 全 13 セッターメソッドの `where` 節から `Sync` を除去し、`Arc::new` を `Some(Box::new)` に変更する
- `SoraConnection::new()` で `config` を `mut` に変更し、`on_track` / `on_remove_track` を `clone()` から `take().expect()` に変更する
- `PcObserverHandler` のフィールドを `Arc<dyn Fn + Send + Sync>` から `Box<dyn Fn + Send>` に変更する
- `run()` 内の 10 個のコールバックで `clone()` を `take().expect()` に置き換え、再代入が必要な 5 変数を `let mut` に変更する
- `handle_datachannel_state` の引数型を `&Arc<dyn Fn + Send + Sync>` から `&(dyn Fn + Send)` に変更する
- `handle_datachannel_message` は async 関数のため `.await` 越えに `&dyn Fn + Send` を保持できず、`DataChannelMessageCallbacks` 構造体で値渡しに変更する
- `configure_ice_server_urls` の引数型を `Option<&Arc<...>>` から `Option<&IceServerUrlConfigurer>` に変更し、呼び出し側を `.as_deref()` に変更する
- `e2e-tests/src/test_connection.rs` の `ice_server_url_configurer` セッターの where 節から `Sync` を除去する
- `examples/sumomo/src/main.rs` の `AppEventSender` トレイトから `Sync` を除去する
- `CHANGES.md` の `## develop` に `[CHANGE]` エントリを追記する
