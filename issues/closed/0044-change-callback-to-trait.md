# SoraConnection のコールバックをトレイト化する

- Priority: Medium
- Created: 2026-07-06
- Completed: 2026-07-08
- Model: DeepSeek V4 Pro
- Branch: feature/change-callback-to-trait
- Polished: 2026-07-06

## 目的

現状 `SoraConnection` のコールバックは 12 個の独立した `Box<dyn Fn(...) + Send>` として定義されている。
実際の利用ではコールバック間で状態を共有する必要があるが、クロージャ間での共有には `Arc` や mpsc チャネルを経由する必要があり冗長。
コールバック群を単一のトレイト `SoraConnectionEventHandler` に統合することで、ユーザーが自身の struct に状態を持たせ、
`&mut self` による自然な状態共有を実現する。

## 優先度根拠

- 現状のコールバック設計は状態共有が必須の利用パターン（sumomo の `event_tx` 共有、e2e テストの `SoraTestEvent` 集約）に対して API が冗長
- 0043（コールバックから Sync 境界を除去）の完了を前提とした次の改善ステップであり、安定リリース (0020) に向けた API 整備の一環

## 現状

`src/connection.rs` で 12 個のコールバック型エイリアスが定義され（68-79 行目）、
`SoraConnectionBuilder` が 12 個の `Option<Box<dyn Fn(...) + Send>>` フィールド（90-105 行目）と対応する setter メソッド（191-331 行目）を持つ。

| コールバック | シグネチャ |
|---|---|
| `on_signaling_message` | `Fn(SignalingType, SignalingDirection, &str)` |
| `on_notify` | `Fn(&str)` |
| `on_push` | `Fn(&str)` |
| `on_track` | `Fn(RtpTransceiver)` |
| `on_remove_track` | `Fn(RtpReceiver)` |
| `on_switched` | `Fn()` |
| `on_websocket_close` | `Fn(Option<u16>, &str)` |
| `on_message` | `Fn(&str, &[u8])` |
| `on_data_channel` | `Fn(&str)` |
| `on_data_channel_open` | `Fn(&str)` |
| `on_data_channel_message` | `Fn(&str, &[u8])` |
| `on_data_channel_close` | `Fn(&str)` |

`SoraConnection::run()` 内では各コールバックを `config.on_xxx.take()` で取り出してローカル変数に保持し、
`DataChannelMessageCallbacks` 構造体（827-834 行目）で一部コールバックを束ねている。
`on_track` と `on_remove_track` は `PeerConnectionObserver` (`PcObserverHandler`) に格納される。

0043 で各コールバック型は `Arc<dyn Fn + Send + Sync>` から `Box<dyn Fn + Send>` に変更済みである。

## 設計方針

### トレイトについて

`shiguredo-rust` スキルは「トレイトを作らないこと。どうしても必要な場合は許可を得ること」と規定している。
本 issue では、12 個のイベントハンドラを自然な形で共有状態付きで束ねられる手段としてトレイトが最適であり、
かつ `enum` による代替（単一コールバック `on_event(SoraEvent)` 方式）では利用側での手動ディスパッチが必要となり API が冗長になる。
これらの理由によりトレイト導入を選択する。許可は @melpon から得ている。

### トレイト定義

新しいファイル `src/connection_event_handler.rs` にトレイト `SoraConnectionEventHandler` を定義する。
全てのトレイトメソッドとトレイト自体に doc コメントを付与する。
doc コメントの内容は、既存の各コールバック setter の doc コメントから転記する。
転記時に引数の `\` エスケープ（`[RtpTransceiver]` → `[\\[RtpTransceiver]\\]`）やリンク先解決を確認すること。

全 12 メソッドにデフォルトの空実装をトレイト定義内で提供する（マクロは使用しない）。
各メソッドは `&mut self` を受け取る。`FnMut` ではなく `&mut self` なのは、`FnMut` をトレイトの supertrait にするとクロージャ以外で実装できなくなるため。

`SoraConnectionEventHandler` は `Send` を要求する。
`Sync` は不要（各コールバックは単一タスクから直列に呼ばれ、並行呼び出しは発生しないため）。
`SoraConnectionEventHandler` は `#[non_exhaustive]` を付けない（将来のメソッド追加は破壊的変更として扱う）。

メソッドシグネチャ:

```rust
pub trait SoraConnectionEventHandler: Send {
    fn on_signaling_message(&mut self, signaling_type: SignalingType, direction: SignalingDirection, text: &str) {}
    fn on_notify(&mut self, text: &str) {}
    fn on_push(&mut self, text: &str) {}
    fn on_track(&mut self, transceiver: RtpTransceiver) {}
    fn on_remove_track(&mut self, receiver: RtpReceiver) {}
    fn on_switched(&mut self) {}
    fn on_websocket_close(&mut self, code: Option<u16>, reason: &str) {}
    fn on_message(&mut self, label: &str, data: &[u8]) {}
    fn on_data_channel(&mut self, label: &str) {}
    fn on_data_channel_open(&mut self, label: &str) {}
    fn on_data_channel_message(&mut self, label: &str, data: &[u8]) {}
    fn on_data_channel_close(&mut self, label: &str) {}
}
```

### Builder の変更

`SoraConnectionBuilder` から 12 個のコールバックフィールドと対応する setter メソッド（`on_signaling_message`, `on_notify`, `on_push`, `on_track`, `on_remove_track`, `on_switched`, `on_websocket_close`, `on_message`, `on_data_channel`, `on_data_channel_open`, `on_data_channel_message`, `on_data_channel_close`）を削除する。

代わりに `SoraConnection::builder()` の引数で `event_handler` を受け取るようにする。`.event_handler()` setter は不要:

```rust
pub fn builder(
    context: Arc<SoraConnectionContext>,
    signaling_urls: Vec<String>,
    channel_id: String,
    role: Role,
    event_handler: impl SoraConnectionEventHandler + Send + 'static,
) -> SoraConnectionBuilder {
    SoraConnectionBuilder::new(context, signaling_urls, channel_id, role, Box::new(event_handler))
}
```

`SoraConnectionBuilder` のフィールドには `event_handler: Option<Box<dyn SoraConnectionEventHandler + Send>>` を追加する。

`SoraConnection::new()` 内では `config` 全体を `SoraConnection` のフィールドに格納する都合上、部分ムーブを避けるため `Option` でラップする。
`SoraConnectionBuilder::new()` で常に値が設定されるため、`SoraConnection::new()` では `config.event_handler.take().expect("event_handler は new() でデフォルト値が設定されている")` で安全に取り出せる。

`ice_server_url_configurer`（`Box<IceServerUrlConfigurer>`、`IceServerUrlConfigurer` は `connection.rs:66` の型エイリアス）は設定用のコールバックであり、イベントハンドリングとは責務が異なるためトレイトに含めず、従来通りフィールドとして保持する。

### SoraConnection での保持方法

`SoraConnection` は `event_handler: Box<dyn SoraConnectionEventHandler + Send>` をフィールドとして保持する。
フィールド追加位置は `config` の直前とする（`config` が最後に破棄されるべき制約を維持するため）。
`SoraConnection::new()` 内では `config.event_handler` を `SoraConnection` に move する。

### `run()` の変更概要

`run()` 先頭の `config.on_xxx.take()` による 12 個のローカル変数取り出しをすべて削除する。
代わりに `self.event_handler` をローカル変数に move する:

```rust
pub async fn run(mut self) -> Result<()> {
    let mut handler = self.event_handler; // フィールドから取り出し
    // ...
}
```

全コールバック呼び出しを `handler.on_xxx(...)` に機械的に置き換える。
`handler` はローカル変数なので `expect()` 不要。

| 変更前 | 変更後 |
|---|---|
| `on_signaling_message(...)` | `handler.on_signaling_message(...)` |
| `on_notify(...)` | `handler.on_notify(...)` |
| `on_push(...)` | `handler.on_push(...)` |
| `on_switched()` | `handler.on_switched()` |
| `on_websocket_close(...)` | `handler.on_websocket_close(...)` |
| `on_data_channel(...)` | `handler.on_data_channel(...)` |

`on_data_channel_open` / `on_data_channel_close` は後述の `handle_datachannel_state` 改修で対応する。
`on_message` / `on_data_channel_message` は後述の `handle_datachannel_message` 改修で対応する。
`on_signaling_message` / `on_notify` / `on_push` は上記表の直接置き換えに加え、`handle_datachannel_message` 内でも `handler.on_xxx(...)` で呼ばれる。

### DataChannelMessageCallbacks の置き換え

現状の `DataChannelMessageCallbacks` は、`handle_datachannel_message` (async fn) 内で `.await` を跨いで借用を保持しないよう、
5 つの `Box<dyn Fn>` を値渡しで受け渡している。
トレイト化後も async 越えの問題があるため、`Box<dyn SoraConnectionEventHandler + Send>` を値渡しする:

```rust
// run() のイベントループ内
SoraEvent::DataChannelMessage { label, data } => {
    handler = self.handle_datachannel_message(&label, &data, handler).await?;
}
```

`handle_datachannel_message` の新しいシグネチャ:

```rust
async fn handle_datachannel_message(
    &mut self,
    label: &str,
    data: &[u8],
    handler: Box<dyn SoraConnectionEventHandler + Send>,
) -> Result<Box<dyn SoraConnectionEventHandler + Send>>
```

内部では `handler.on_xxx(...)` で呼び出す。

`handle_datachannel_message` 内では `.await`（`self.handle_offer`, `self.get_stats`）を挟んで
`handler.on_signaling_message()`, `handler.on_push()`, `handler.on_notify()`, `handler.on_message()`, `handler.on_data_channel_message()` が呼ばれる。
`handler` は `Box<dyn SoraConnectionEventHandler>` で値渡しされており、`.await` 地点では所有権が `handler` に残らない（`self` の一部ではない）ため、
Send 境界の問題は発生しない。また `handler` の各メソッドは `&mut self` で直列に呼ばれるため、
内部可変性（`RefCell` 等）を前提としない安全な設計である。

なお `register_data_channel` 内の `DataChannelObserverHandler::on_message`（`connection.rs:1868-1875`）は `SoraEvent::DataChannelMessage` を mpsc チャネル経由で送信し、
メインループで `handle_datachannel_message` にルーティングされる。この間接呼び出し経路はトレイト化で変更されない。

### handle_datachannel_state の改修

`handle_datachannel_state` は非 async のため、`&mut dyn SoraConnectionEventHandler` 参照で渡す:

```rust
fn handle_datachannel_state(
    &self,
    handler: &mut dyn SoraConnectionEventHandler,
    label: &str,
    opened_datachannels: &mut HashSet<String>,
    use_datachannel_signaling: &mut bool,
) {
    if self.is_datachannel_open(label) && !opened_datachannels.contains(label) {
        rtc_log_info!("DataChannel '{}' opened", label);
        opened_datachannels.insert(label.to_string());
        handler.on_data_channel_open(label);
        // ...
    } else if self.is_datachannel_closed(label) && opened_datachannels.contains(label) {
        // ...
        handler.on_data_channel_close(label);
    }
}
```

呼び出し元では `&mut *handler` で渡す。

`SoraEvent::DataChannelRegister` アーム内では `handler.on_data_channel(&label)` の直後に `self.handle_datachannel_state(&mut *handler, ...)` を呼ぶため、
`handler` への `&mut` 借用が連続する。NLL により別々の文（statement）として解決されるため借用チェッカー上の問題はない。

`SoraEvent::DataChannelStateChange` アーム（現: `connection.rs:1188-1189`）も同様に `handle_datachannel_state` を呼んでいるため、
`on_data_channel_open` / `on_data_channel_close` の参照を `&mut *handler` 経由に変更する必要がある:

```rust
SoraEvent::DataChannelStateChange(label) => {
    self.handle_datachannel_state(&mut *handler, &label, &mut opened_datachannels, &mut use_datachannel_signaling);
}
```

`run()` 内で `on_data_channel_close` を直接呼んでいる以下の 2 箇所も
`handler.on_data_channel_close(...)` に変更する:

1. 切断コマンドハンドラ (`connection.rs:1199-1202`)
2. `run()` 終了時の DataChannel クローズ待機ループ (`connection.rs:1528`)

### PcObserverHandler の改修

現在 `PcObserverHandler` は `Box<dyn Fn(RtpTransceiver) + Send>` と `Box<dyn Fn(RtpReceiver) + Send>` を保持しているが（`src/connection.rs:865-866`）、
トレイト化後は `event_handler` 全体を `PcObserverHandler` に move できない（他のメソッドも使う必要があるため）。
また、`&dyn SoraConnectionEventHandler` 参照を渡すことも `PcObserverHandler` が `Box<dyn PeerConnectionObserverHandler + 'static>` にキャストされるため不可能。

解決策: `PcObserverHandler` から `on_track` / `on_remove_track` を直接呼ぶのをやめ、
内部イベントチャネル (`SoraEvent`) 経由でメインループに通知し、メインループで `self.event_handler` のメソッドを呼び出す。

この変更は `on_ice_candidate` / `on_data_channel` で既に採用されているパターンと同一であり、設計の一貫性が向上する。

### 同期→非同期変更の影響

現状 `PcObserverHandler::on_track` / `on_remove_track` は WebRTC シグナリングスレッドから同期的に呼ばれており、
ユーザーのコールバックが即座に実行される。変更後は `SoraEvent` の mpsc チャネル経由で非同期的に tokio イベントループで処理される。

影響分析:

- **到着順序の変化**: `SoraEvent::Track` / `RemoveTrack` は mpsc チャネルの FIFO 順に処理されるため、WebRTC 層での発生順序は保たれる。
  `SoraEvent::DataChannelStateChange` や `SignalingMessage` 等の他イベントとの順序は、発生時刻に依存する。
  ただし、現状でも他イベント（DataChannel 登録や ICE candidate）が mpsc 経由で通知されているため、`on_track` を同期のまま特別扱いする一貫性のほうが問題である。
- **`RtpTransceiver` / `RtpReceiver` のライフタイム**: これらの型は `shiguredo_webrtc` 内で `unsafe impl Send` されており、mpsc チャネルでの送信が可能。
  mpsc 受信までの間に内部状態が変化する可能性はあるが、元の同期呼び出しでもコールバック実行時点の状態をスナップショットとして扱うことに変わりはない。
- **互換性**: 本変更は破壊的変更の一部であり、既存利用者がこのタイミング変化に依存しているケースは想定されない。
  ユーザー視点では、コールバックが同期的に呼ばれる保証は API 仕様として宣言されていない。

`SoraEvent` に以下を追加する（`SoraEvent` は `connection.rs:760` の非公開 enum であり、この変更は内部変更のみで公開 API に影響しない）。

```rust
enum SoraEvent {
    // ... 既存のバリアント ...
    Track(RtpTransceiver),
    RemoveTrack(RtpReceiver),
}
```

`RtpTransceiver` と `RtpReceiver` はいずれも `Send` を実装している（`shiguredo_webrtc` 内で `unsafe impl Send` 済み）ため、mpsc チャネルでの送信が可能。
実装前に `cargo check` で型が `Send` であることを確認すること（C++ バインディング経由の opaque type のため、shiguredo_webrtc のバージョン変更で `Send` 実装が変わる可能性がある）。

`PcObserverHandler` から `on_track` / `on_remove_track` フィールドを削除し、
代わりに 3 つ目の `event_tx` クローン `event_tx_for_track` を追加する。
既存の `event_tx_for_candidate` / `event_tx_for_channel` は ICE candidate / DataChannel 登録で使われているためそのまま維持する。

```rust
struct PcObserverHandler {
    event_tx_for_candidate: mpsc::UnboundedSender<SoraEvent>,
    event_tx_for_channel: mpsc::UnboundedSender<SoraEvent>,
    event_tx_for_track: mpsc::UnboundedSender<SoraEvent>,
}

impl PeerConnectionObserverHandler for PcObserverHandler {
    fn on_track(&mut self, transceiver: RtpTransceiver) {
        let _ = self.event_tx_for_track.send(SoraEvent::Track(transceiver));
    }

    fn on_remove_track(&mut self, receiver: RtpReceiver) {
        let _ = self.event_tx_for_track.send(SoraEvent::RemoveTrack(receiver));
    }
    // ... 他のメソッドは変更なし ...
}
```

`SoraConnection::new()` 内の変更:
- `config.on_track.take()` / `config.on_remove_track.take()` を削除
- `event_tx_for_track` クローンを追加
- `PcObserverHandler` のコンストラクタから `on_track` / `on_remove_track` 引数を削除し、代わりに `event_tx_for_track` を渡す
- `config.event_handler.take()` で `event_handler` を取り出し、`SoraConnection` に格納する

`run()` のメインループで `SoraEvent::Track` / `RemoveTrack` を処理するアームを追加:

```rust
SoraEvent::Track(transceiver) => {
    handler.on_track(transceiver);
}
SoraEvent::RemoveTrack(receiver) => {
    handler.on_remove_track(receiver);
}
```

### 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| `src/connection.rs` | 12 個の型エイリアス削除。`SoraConnectionBuilder` から 12 個のフィールドと setter 削除 + `event_handler` フィールド追加。`SoraConnectionBuilder::new()` に第 5 引数 `event_handler` 追加 + 12 個のデフォルトコールバック初期化削除。`SoraConnection` に `event_handler` フィールド追加。`SoraConnection::builder()` のシグネチャ変更（第 5 引数追加）。`SoraConnectionBuilder` の doc コメント更新（setter 一覧の内訳変更）。`run()` 全体書き換え。`DataChannelMessageCallbacks` 削除。`PcObserverHandler` 改修 (`on_track`/`on_remove_track` フィールド削除 + `event_tx_for_track` 追加)。`SoraEvent` 拡張 (`Track`/`RemoveTrack` 追加)。`handle_datachannel_message` シグネチャ変更。`handle_datachannel_state` シグネチャ変更。`new()` 内の `config.on_track.take()` / `config.on_remove_track.take()` 削除 + `config.event_handler.take()` 追加。
| `src/connection_event_handler.rs` | 新規作成。`SoraConnectionEventHandler` トレイト定義 |
| `src/lib.rs` | `mod connection_event_handler` 追加、`SoraConnectionEventHandler` を公開 API として re-export、利用例 doc コメントの更新 |
| `examples/sumomo/src/main.rs` | `build_connection_builder()` の書き換え（トレイト実装 struct の作成と `builder()` への引数渡し） |
| `e2e-tests/src/test_connection.rs` | `with_callbacks()` から `SoraConnectionEventHandler` 実装への書き換え |
| `README.md` | `sendrecv` 例（L110-124）・`recvonly` 例（L180-189）の `builder()` 呼び出しから `.on_notify()` / `.on_track()` 連鎖を削除し、トレイト実装 struct を渡す形に更新。`SoraConnection::builder() の設定` 節（L197-255）の 12 個のコールバック setter の説明を、`SoraConnectionEventHandler` トレイトの実装方法に置き換え。`メッセージ受信` 節（L279-287）の `on_message` 説明をトレイトメソッドベースに更新 |
| `CHANGES.md` | `[CHANGE]` エントリの追加 |

### テスト戦略

- 単体テスト: `tests/test_connection_event_handler.rs` を新規作成し、以下を検証する:
  - デフォルト実装が全 12 メソッドで空であり、呼び出してもパニックしないこと
  - 自作の struct で特定メソッドのみオーバーライドし、残りがデフォルト実装になること
  - `SoraConnectionBuilder::new()` が `event_handler` のデフォルト値として空実装を設定すること
- 単体テスト（既存）: `src/connection.rs` の `#[cfg(test)] mod tests`（2804 行目以降）で以下を確認する:
  - `SoraEvent::Track` / `RemoveTrack` バリアントの追加が既存の `SoraEvent` 構築テスト（`pending_rpc_request_drop_aborts_timeout_handle`: 3070 行目）を壊していないこと
- e2e テスト: `e2e-tests/src/test_connection.rs` の `SoraTestConnection` を `SoraConnectionEventHandler` 実装に移行し、既存の全 e2e テストが成功することを確認する
  - 全 callback イベントが `SoraTestEventHandler` → `SoraTestConnection::event_rx` の経路で正しく伝達されること
  - `on_track` / `on_remove_track` が `SoraEvent::Track` / `RemoveTrack` 経由で正しくメインループに届き、`handler.on_track()` / `handler.on_remove_track()` が呼ばれること
  - `handle_datachannel_message` が `handler` を値渡しで受け取り、`SoraEvent::DataChannelMessage` 処理後に所有権がループに戻ること
- PBT / Fuzzing: この変更では入力ドメインが変わらないため不要

### エッジケース

- トレイトメソッド内でパニックが発生した場合: `std::panic::catch_unwind` は使用禁止（`shiguredo-rust` スキル規約）。呼び出し側で握りつぶさず、パニックは上方に伝播させる（現状の `Box<dyn Fn>` と同じ挙動）
- `SoraConnectionEventHandler` 実装が `Send` を満たさない場合: コンパイル時に `builder()` の `where` 節で弾かれるため実行時の考慮は不要
- `builder()` がジェネリクス (`impl SoraConnectionEventHandler`) になったことで、利用者は `sora_sdk::SoraConnectionEventHandler` を import する必要がある。利用者への周知は CHANGES.md で行う
- `connection_event_handler.rs` に必要な use 文: `use shiguredo_webrtc::{RtpReceiver, RtpTransceiver};` および `use crate::types::{SignalingDirection, SignalingType};`

### e2e テストの移行設計

`e2e-tests/src/test_connection.rs` の `SoraTestConnectionBuilder` の移行方法:

1. イベントハンドラ用の内部 struct `SoraTestEventHandler` を追加する。
   `event_tx: UnboundedSender<SoraTestEvent>` を保持し、`SoraConnectionEventHandler` を実装する。

2. `SoraTestConnection::builder()` 内で `event_tx` / `event_rx` を生成し、
   生成した `SoraTestEventHandler { event_tx: event_tx.clone() }` を `SoraConnection::builder()` の第 5 引数に渡す。
   `SoraTestConnectionBuilder` に新たに `event_rx: UnboundedReceiver<SoraTestEvent>` フィールドを追加し、
   生成した `event_rx` を保持する。

3. `connect()` メソッドでは `self.event_rx` を取り出して `SoraTestConnection` に渡す。
   `with_callbacks()` メソッドを削除する。12 個の setter 連鎖は不要になり、`SoraTestEventHandler` 実装に一元化される。

```rust
// SoraTestConnectionBuilder の新しいフィールド
pub struct SoraTestConnectionBuilder {
    inner: SoraConnectionBuilder,
    event_rx: UnboundedReceiver<SoraTestEvent>,  // 新設
}

// builder() 内で event_tx/rx を生成し、SoraTestEventHandler を builder に渡す
pub fn builder(context: ..., urls: ..., channel_id: ..., role: ...) -> Self {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let event_handler = SoraTestEventHandler { event_tx };
    let inner = SoraConnection::builder(context, urls, channel_id, role, event_handler);
    SoraTestConnectionBuilder { inner, event_rx }
}

// connect() は self.event_rx を SoraTestConnection に渡すだけ
pub fn connect(self) -> Result<SoraTestConnection> {
    let (connection, handle) = self.inner.build()?;
    let run_task = tokio::spawn(async move { connection.run().await });
    Ok(SoraTestConnection {
        handle,
        event_rx: self.event_rx,
        event_log: Vec::new(),
        run_task,
    })
}
```

新設する内部 struct `SoraTestEventHandler`:

```rust
// 新設する内部 struct
struct SoraTestEventHandler {
    event_tx: UnboundedSender<SoraTestEvent>,
}

impl SoraConnectionEventHandler for SoraTestEventHandler {
    fn on_signaling_message(&mut self, signaling_type: SignalingType, direction: SignalingDirection, text: &str) {
        let _ = self.event_tx.send(SoraTestEvent::SignalingMessage { signaling_type, direction, text: text.to_string() });
    }
    fn on_notify(&mut self, message: &str) {
        let _ = self.event_tx.send(SoraTestEvent::Notify { message: message.to_string() });
    }
    fn on_push(&mut self, message: &str) {
        let _ = self.event_tx.send(SoraTestEvent::Push { message: message.to_string() });
    }
    fn on_track(&mut self, transceiver: RtpTransceiver) {
        let kind = transceiver.receiver().track().kind().ok();
        let _ = self.event_tx.send(SoraTestEvent::Track { kind });
    }
    fn on_remove_track(&mut self, receiver: RtpReceiver) {
        let kind = receiver.track().kind().ok();
        let _ = self.event_tx.send(SoraTestEvent::RemoveTrack { kind });
    }
    fn on_switched(&mut self) { let _ = self.event_tx.send(SoraTestEvent::Switched); }
    fn on_websocket_close(&mut self, code: Option<u16>, reason: &str) {
        let _ = self.event_tx.send(SoraTestEvent::WebsocketClose { code, reason: reason.to_string() });
    }
    fn on_message(&mut self, label: &str, data: &[u8]) {
        let _ = self.event_tx.send(SoraTestEvent::Message { label: label.to_string(), data: data.to_vec() });
    }
    fn on_data_channel(&mut self, label: &str) {
        let _ = self.event_tx.send(SoraTestEvent::DataChannel { label: label.to_string() });
    }
    fn on_data_channel_open(&mut self, label: &str) {
        let _ = self.event_tx.send(SoraTestEvent::DataChannelOpen { label: label.to_string() });
    }
    fn on_data_channel_message(&mut self, label: &str, data: &[u8]) {
        let _ = self.event_tx.send(SoraTestEvent::DataChannelMessage { label: label.to_string(), data: data.to_vec() });
    }
    fn on_data_channel_close(&mut self, label: &str) {
        let _ = self.event_tx.send(SoraTestEvent::DataChannelClose { label: label.to_string() });
    }
}
```

`SoraTestConnectionBuilder::connect()` 内で `event_handler` を構築して builder に渡すフローが必要だが、
`SoraConnection::builder()` が第 5 引数で `event_handler` を必須とするため、`builder()` 生成時に `event_tx`/`event_rx` を生成し `SoraTestEventHandler` を渡す設計とする。
`SoraTestConnection::builder()` の public シグネチャ（4 引数）は `SoraConnection::builder()` の 5 引数化により、
内部での呼び出しも引数変更が必要（第 5 引数に `SoraTestEventHandler` を渡す）。

### 後方互換

破壊的変更。12 個の個別コールバック setter が削除され、`SoraConnection::builder()` の第 5 引数で `event_handler` を渡す必要がある。
利用者はトレイトを実装した struct を作成し、`builder()` に渡す。

sumomo の移行イメージ:

`AppEventSender` トレイト（`examples/sumomo/src/main.rs:76`）は 0043 で `Clone + Send + 'static` となったが、
0044 のトレイト化後はクロージャ間の共有が不要になるため、トレイト実装 struct 内に `event_tx` を直接保持できる。
ただし `AppEventSender` には `mpsc::Sender<AppEvent>`（tokio）と `std::sync::mpsc::Sender<AppEvent>`（raw_player）
の 2 つの実装があるため、`AppEventHandler` をジェネリクスにする:

```rust
// 移行前
let builder = SoraConnection::builder(context, signaling_urls, channel_id, role);
builder
    .on_notify({ let event_tx = event_tx.clone(); move |text| { event_tx.send_event(AppEvent::Notify(text.to_string())); } })
    .on_track({ let event_tx = event_tx.clone(); move |transceiver| { event_tx.send_event(AppEvent::OnTrack(transceiver)); } });

// 移行後
struct AppEventHandler<T: AppEventSender> { event_tx: T }
impl<T: AppEventSender> SoraConnectionEventHandler for AppEventHandler<T> {
    fn on_notify(&mut self, text: &str) { let _ = self.event_tx.send_event(AppEvent::Notify(text.to_string())); }
    fn on_push(&mut self, text: &str) { let _ = self.event_tx.send_event(AppEvent::Push(text.to_string())); }
    fn on_track(&mut self, transceiver: RtpTransceiver) { let _ = self.event_tx.send_event(AppEvent::OnTrack(transceiver)); }
    fn on_remove_track(&mut self, receiver: RtpReceiver) { let _ = self.event_tx.send_event(AppEvent::OnRemoveTrack(receiver)); }
}
let builder = SoraConnection::builder(
    context, signaling_urls, channel_id, role,
    AppEventHandler { event_tx },
);
```

`build_connection_builder` 関数の新しいシグネチャは `fn build_connection_builder<T: AppEventSender>(context: Arc<SoraConnectionContext>, args: &Args, event_tx: T) -> Result<SoraConnectionBuilder>` となる。
`AppEventSender` トレイト自体は `sumomo` 内で引き続き使用する（`AppEventHandler` がジェネリクス境界として必要とするため）。

## 完了条件

- コールバック用の型エイリアス 12 個が削除されている
- `src/connection_event_handler.rs` に `SoraConnectionEventHandler` トレイトが定義され、全メソッドにデフォルトの空実装がある
- `SoraConnection::builder()` のシグネチャが `event_handler` を第 5 引数として受け取るように変更されている
- `SoraConnection` に `event_handler: Box<dyn SoraConnectionEventHandler + Send>` フィールドが追加されている
- `SoraConnectionBuilder` に `event_handler: Option<Box<dyn SoraConnectionEventHandler + Send>>` フィールドが追加されている
- `SoraConnectionBuilder::new()` のシグネチャに第 5 引数 `event_handler: Box<dyn SoraConnectionEventHandler + Send>` が追加され、12 個の旧コールバック初期化コードが削除されている
- `SoraConnectionBuilder` の 12 個のコールバックフィールドと対応 setter が削除されている
- `SoraConnectionBuilder` の公開 doc コメント（`src/connection.rs:81-84`）が更新され、callback setter の代わりに `builder()` 経由の `event_handler` 設定が説明されている
- `DataChannelMessageCallbacks` 構造体が削除されている
- `SoraEvent` に `Track` と `RemoveTrack` バリアントが追加され、既存の `SoraEvent` パターンマッチが壊れていない
- `PcObserverHandler` が `on_track` / `on_remove_track` を直接呼ばず、`SoraEvent` 経由でメインループに委譲している
- sumomo 例が `SoraConnectionEventHandler` 実装に移行している
- e2e テスト (`e2e-tests/src/test_connection.rs`) が `SoraConnectionEventHandler` 実装に移行し、全テストが成功している
- `ice_server_url_configurer` が従来通り `SoraConnectionBuilder` のフィールドとして維持され、トレイトに含まれていない
- `tests/test_connection_event_handler.rs` にトレイトのデフォルト実装の単体テストが追加されている
- `src/lib.rs` の `# 基本的な使い方` セクション (L18-33) の doc コメントが `builder()` に `event_handler` を渡すコード例に更新されている
- `README.md` の全コード例から 12 個のコールバック setter 連鎖が削除され、`SoraConnectionEventHandler` トレイト実装を渡す形に更新されている
- `README.md` の `SoraConnection::builder() の設定` 節が、12 個の setter 説明からトレイト実装方法の説明に置き換えられている
- `src/lib.rs` で `pub use crate::connection_event_handler::SoraConnectionEventHandler;` により再公開されている（同一クレート内のモジュール公開であり、`shiguredo-rust` スキルの「依存元の出自が分かりにくくなる」懸念には該当しない）
- `CHANGES.md` の develop セクションに `[CHANGE]` エントリとして以下が記載されている:
  - `SoraConnection のコールバックをトレイト化する`
  - `12 個の個別コールバック setter（on_signaling_message, on_notify, on_push, on_track, on_remove_track, on_switched, on_websocket_close, on_message, on_data_channel, on_data_channel_open, on_data_channel_message, on_data_channel_close）を削除する`
  - `SoraConnection::builder()` に第 5 引数 `event_handler: impl SoraConnectionEventHandler + Send + 'static` を追加する`
  - `src/connection_event_handler.rs` に `SoraConnectionEventHandler` トレイトを追加する`

## 解決方法

- `src/connection_event_handler.rs` を新規作成し、全 12 メソッドにデフォルト空実装を持つ `SoraConnectionEventHandler` トレイトを定義した
- `src/connection.rs` から 12 個の callback 型エイリアス・フィールド・setter を削除し、`SoraConnection::builder()` に第 5 引数 `event_handler` を追加した
- `PcObserverHandler` から `on_track` / `on_remove_track` の直接呼び出しを廃止し、`SoraEvent::Track` / `RemoveTrack` バリアント経由でメインループに委譲した
- `DataChannelMessageCallbacks` を削除し、`handle_datachannel_message` / `handle_datachannel_state` を `&mut dyn SoraConnectionEventHandler` 参照で統一した
- `event_handler` は `SoraConnection` の独立フィールドにせず `config` に保持し `run()` で直接 `take()` する形にした
- sumomo / e2e テスト / README / CHANGES を移行した
