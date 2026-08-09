# sumomo のイベントチャネル満杯で OnTrack を失わないようにする

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-sumomo-event-channel-overflow
- Polished: {YYYY-MM-DD}

## 目的

sumomo のイベントチャネルが満杯になったときに `OnTrack` イベントが黙って破棄されず、映像が表示されない状態を防ぐ。

## 現状

`examples/sumomo/src/main.rs` のイベントハンドラは `mpsc::channel::<AppEvent>(32)` に対して `send_event` で `try_send` しており、容量 32 を超えたイベントは破棄される。`OnTrack` が落ちると該当トラックが `tracks` に登録されず、エラーも出ないまま映像が表示されない。

通知 (notify) のバーストや、複数参加者のトラックが同時に追加される場面で発生し得る。raw-player パスは unbounded チャネルでこの問題がない。

## 設計方針

- イベントチャネルを unbounded にするか、`send` (backpressure) に変更して破棄しない
- `OnTrack` / `OnRemoveTrack` などトラック管理に影響するイベントを確実に処理する
- 満杯時の挙動 (ブロック vs ドロップ) を明確にし、ドロップする場合は警告ログを出す

## 完了条件

- イベントバースト時でも `OnTrack` が失われない
- 通常時のメインループの遅延が許容範囲である
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `examples/sumomo/src/main.rs`
- `CHANGES.md`
