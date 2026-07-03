# コールバック関数から Sync トレイト境界を除去する

- Priority: Medium
- Created: 2026-07-03
- Completed: {YYYY-MM-DD}
- Model: DeepSeek-V4 Pro
- Branch: feature/refactor-remove-sync-from-callbacks
- Polished: {YYYY-MM-DD}

## 目的

コールバック関数に付与されている `Sync` トレイト境界は実際には不要である。コールバックは非同期タスク間で共有されるため `Send` 境界は必須だが、同時に複数スレッドから呼び出されることはないため `Sync` は過剰な制約になっている。さらに API の `Sync` 要求は利用者側のコードに波及し、`AppEventSender` のようなトレイトにも不要な `Sync` 境界を強いている。すべての `Sync` 境界を除去し、利用者が `Sync` を実装しない型をキャプチャしたクロージャを自由に渡せるようにする。

## 優先度根拠

- API の制約緩和であり、現時点で具体的な不具合報告はない
- ただし `Sync` は利用者側の設計に波及するトレイト境界のため、早期の除去が望ましい
- `Box` 化により `Arc` の参照カウントオーバーヘッドも不要になる

## 現状

`src/connection.rs` の `SoraConnectionBuilder` 構造体が保持する 14 種のコールバックはすべて `Arc<dyn Fn(...) + Send + Sync>` で保持されている。また、対応するビルダーセッターメソッドの `where` 節もすべて `Fn(...) + Send + Sync + 'static` を要求している。

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
| `ice_server_url_configurer` | 118 | `Option<Arc<IceServerUrlConfigurer>>`（※ `IceServerUrlConfigurer` 経由で `Sync` を含む） |

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

`build()` 内（src/connection.rs:836-978）で各コールバックが `.clone()` されている。`on_track` と `on_remove_track` は `PcObserverHandler` と `SoraConnection` の両方で必要になるため、クローンが必要。

## 設計方針

### Arc は維持し Sync のみ外す

`build()` 内で `on_track` / `on_remove_track` が `PcObserverHandler` と `SoraConnection` の両方に渡されるため、`Box` ではクローンできず共有に問題が生じる。一方、`Arc` 自体は `Send + Sync` だが、その内部の `dyn Fn` が `Sync` でなくても `Arc` 全体は `Send` になる（`Arc<T>: Send` ⇔ `T: Send + Sync` だが `T = dyn Fn + Send` とすると `dyn Fn + Send` は `Sync` ではないので `Arc<dyn Fn + Send>` は `Send` にならない）。

実際には `Arc<dyn Fn + Send + Sync>` のまま `Sync` を外すと `Arc<dyn Fn + Send>` になり、`Send` が成立しなくなる（`Arc<T>: Send if T: Send + Sync`）。

そのため単に `Sync` 境界を外すだけでは不十分で、以下のいずれかの方針をとる必要がある:

### 方針 A: `Box` 化 + クローン不要へのリファクタリング

フィールド型を `Box<dyn Fn(...) + Send>` に変更し、`on_track` / `on_remove_track` が二重に必要とされている構造を見直す。

`PcObserverHandler` は `build()` 内で構築されるローカルな existence であり、`on_track` / `on_remove_track` は `PcObserverHandler` 内でのみ必要になる。`SoraConnection` 側で `on_track` / `on_remove_track` が使用されている箇所を確認し、不要なら `PcObserverHandler` 側だけに限定する。

もし `SoraConnection` 側でも必要であれば、`SoraConnection` に `on_track` / `on_remove_track` を持たせ、`PcObserverHandler` 側はチャネル経由で `SoraEvent` を送信する形に変更するなど、構造を整理する。

### 方針 B: `Arc` 維持 + `Sync` 削除 (`Arc<dyn Fn + Send>`)

厳密には `Arc<dyn Fn + Send>` は `Send` にならないが、実際の利用ではこれらの `Arc` は `tokio::spawn` の Future にムーブされるだけで `Send` 境界は別の形で満たせる可能性がある。この方針では、フィールドの所有権を `build()` で `Option::take()` により移譲し、構造体内で `Arc` を `.clone()` して使う形を維持しつつ `Sync` を外す。

### 推奨方針

方針 A が最もクリーン。実際に `src/connection.rs` の `SoraConnection::run()` 内のイベントループ（行 1080-1122）を見ると、`on_track` / `on_remove_track` は `SoraEvent::Track` / `SoraEvent::RemoveTrack` としては扱われておらず、`PcObserverHandler` からのみ呼び出されている可能性が高い。確認の上、不要な共有を排除する。

他のコールバックについても、`build()` 内でクローンした後に `SoraConnection::run()` のイベントループや internal method で使われており、`PcObserverHandler` との共有は発生していないため、`Box` 化 + `.clone()` 除去は可能。

## 完了条件

- `src/connection.rs` の全コールバック型定義・セッターメソッドの `where` 節から `Sync` 境界が除去されている
- フィールド型が `Arc<dyn Fn(...) + Send + Sync>` から `Box<dyn Fn(...) + Send>` に変更されている
- `build()` 内の `.clone()` 呼び出しが不要になり除去されている
- `PcObserverHandler` のフィールド、`handle_datachannel_state` / `handle_datachannel_message` / `configure_ice_server_urls` の引数型も追従している
- `e2e-tests/src/test_connection.rs:175` の where 節から `Sync` が除去されている
- `examples/sumomo/src/main.rs:76` の `AppEventSender` トレイトから `Sync` 境界が除去されている
- `cargo build --workspace` が全 feature 組合せで成功する
- `cargo test --workspace` が全 feature 組合せで成功する
- examples/sumomo がコンパイル可能である

## 解決方法

1. `IceServerUrlConfigurer` 型エイリアスから `Sync` を除去する（`src/connection.rs:66`）
2. `SoraConnectionBuilder` の全コールバックフィールドを `Arc<dyn Fn(...) + Send + Sync>` → `Box<dyn Fn(...) + Send>` に変更する（`src/connection.rs:78-118`）
3. 全セッターメソッドの where 節から `Sync` を除去し、`Arc::new(handler)` を `Box::new(handler)` に変更する（`src/connection.rs:189-499`）
4. `PcObserverHandler` の `on_track` / `on_remove_track` フィールドを `Box<dyn Fn(...) + Send>` に変更する（`src/connection.rs:841-842`）
5. `build()` 内の `config.on_*` からの値の取り出しを `Option` + `take()` に変更し、`.clone()` を除去する（`src/connection.rs:836-978`）
6. 構築後の各コールバック変数は `Box<dyn Fn(...) + Send>` として扱い、`SoraConnection` のフィールドやローカル変数へムーブする
7. `handle_datachannel_state` / `handle_datachannel_message` / `configure_ice_server_urls` の引数型を `&Box<dyn Fn(...) + Send>` に変更する（`src/connection.rs:1845-1846,1925-1929,1566`）
8. `e2e-tests/src/test_connection.rs:173-175` の `ice_server_url_configurer` ラッパーから `Sync` を除去する
9. `examples/sumomo/src/main.rs:76` の `AppEventSender` トレイトから `Sync` 境界を除去する
