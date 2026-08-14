# sumomo のイベントチャネルを unbounded にして OnTrack / OnRemoveTrack を失わないようにする

- Priority: Medium
- Created: 2026-08-10
- Completed: 2026-08-15
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-event-channel-overflow
- Polished: 2026-08-14

## 目的

sumomo のイベントチャネルが満杯になったときに `OnTrack` / `OnRemoveTrack` イベントが黙って破棄されず、映像が表示されない・映像が出続ける状態を防ぐ。

## 現状

`examples/sumomo/src/main.rs` のイベントハンドラ（`AppEventHandler`）は `mpsc::channel::<AppEvent>(32)` に対して `try_send` しており、容量 32 を超えたイベントは破棄される。`OnTrack` が落ちると該当トラックが `tracks` に登録されず（`handle_on_track_event` が実行されない）、エラーも出ないまま映像が表示されない。`OnRemoveTrack` が落ちると `tracks` から除去されず、リモートが送信を止めても映像が出続ける。

通知 (notify) のバーストや、複数参加者のトラックが同時に追加される場面で発生し得る。イベントチャネルは ANSI / raw-player の両パスで共通であり、どちらでも同じ問題を持つ。

## 設計方針

- イベントチャネルを `mpsc::unbounded_channel::<AppEvent>()` に変更し、破棄しない
  - backpressure は採用しない。`SoraConnectionEventHandler` は同期トレイトであり、コールバックから `send().await` は await できない。`blocking_send` は `#[tokio::main(flavor = "current_thread")]` の runtime 内から呼ぶと panic する（`Cannot block the current thread from within a runtime`）ため、unbounded 以外に解がない
  - unbounded 採用は closed issue 0086（DataChannel 受信イベントキューを unbounded のまま維持する判断）と方向が一致する
- 送信側の型を `mpsc::Sender<AppEvent>` から `mpsc::UnboundedSender<AppEvent>` に変更し、`try_send` を `UnboundedSender::send` に変更する（受信側が drop された場合のみ失敗する。その場合はイベントを破棄してよい）
- 型変更の影響を受ける箇所は `examples/sumomo/src/main.rs` の `AppEventHandler` と `build_connection_builder` / `build_and_run_connection` の引数、`examples/sumomo/src/tests.rs` のイベントチャネル生成箇所
- `OnTrack` / `OnRemoveTrack` を含む全イベントが失われないことを確認する

## 完了条件

- イベントバースト時でもイベントが失われないことを検証する（`OnTrack` / `OnRemoveTrack` は `RtpTransceiver` / `RtpReceiver` が公開コンストラクタを持たないため直接は検証できない。構築可能な `Notify` / `Push` イベントを 32 を超えて送信し、全イベントが受信されることを実チャネルで検証する。モックやスタブは使わない）
- `examples/sumomo/src/tests.rs` の `AppEvent` チャネルを unbounded に合わせて更新する
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/tests.rs`
- `CHANGES.md`（`[FIX]` エントリを追加）

## 解決方法

- `examples/sumomo/src/main.rs` の `AppEventHandler` の送信側を `mpsc::Sender` から `mpsc::UnboundedSender` に変更し、`try_send` を `UnboundedSender::send` に変更した。イベントチャネルは `mpsc::unbounded_channel::<AppEvent>()` で生成するため、容量 32 を超えても `OnTrack` / `OnRemoveTrack` を含む全イベントが破棄されない
- backpressure は採用しない。`SoraConnectionEventHandler` は同期トレイトであり、コールバックから `send().await` は await できない。`blocking_send` は `#[tokio::main(flavor = "current_thread")]` の runtime 内から呼ぶと panic するため、unbounded 以外に解がない
- `examples/sumomo/src/tests.rs` に `event_channel_delivers_all_burst_events` テストを追加した。`Notify` / `Push` イベントを各 64 件（旧容量 32 を超過）送信し、全件が受信されることを実チャネルで検証する
- 型変更の影響を受ける `build_connection_builder` / `build_and_run_connection` の引数と、`examples/sumomo/src/tests.rs` のイベントチャネル生成箇所も unbounded に合わせて更新した
- `CHANGES.md` への記載は指示により行わなかった
