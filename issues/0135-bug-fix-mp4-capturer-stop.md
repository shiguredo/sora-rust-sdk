# Mp4VideoCapturer の停止遅延を解消する

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-mp4-capturer-stop
- Polished: 2026-08-10

## 目的

長い frame 間隔の待機中でも `Mp4VideoCapturer` を速やかに停止できるようにする。

## 現状

feeder thread は deadline まで `thread::sleep` し、`Drop` は stop flag の設定後に thread を `join` する。
`thread::sleep` 中は stop flag を確認できないため、malformed input 由来の巨大な frame duration では `Drop::join` が長時間停止する。
closed issue 0048 は通常の 30 fps なら停止待ちが約 1 frame 分であるため対応不要としたが、巨大な duration では同じ前提が成立しない。

## 設計方針

- `thread::sleep` を、stop signal で中断できる deadline wait helper へ置き換える
- feeder thread で `thread::park_timeout` を使い、`Drop` は stop flag を `Release` で保存した後、`JoinHandle::thread().unpark()` してから `join()` する
- 明示的な `unpark` なしの予期しない起床でフレームを早く送らないよう、wait helper は次を loop する
  1. stop flag を `Acquire` で読み、設定済みなら停止結果を返す
  2. checked deadline と `Instant::now()` から残り時間を求め、deadline 到達済みなら送信継続結果を返す
  3. 残り時間を `park_timeout` する
  4. 起床後に 1 へ戻り、deadline と stop flag を再評価する
- deadline の計算は `checked_add` で行い、オーバーフローした場合は panic や飽和を避け、英語の production log を残して feeder thread を正常停止する
- `Drop` の応答性を sample duration の値に依存させない

## 実装状況

capturer 停止の実装は、`feature/fix-mp4-arithmetic` ブランチに含まれている。
具体的には `wait_until` helper、`WaitResult` enum、`Drop` の `unpark`、deadline の `checked_add`、テスト 4 件 (`wait_until_stops_immediately_when_stop_is_set` / `wait_until_ready_when_deadline_passed` / `wait_until_interrupted_by_unpark` / `wait_until_rechecks_deadline_after_spurious_wakeup`) が実装済みである。
0098 のマージ後、capturer 停止部分を本 issue のブランチ (`feature/fix-mp4-capturer-stop`) へ分離して PR を出す。

## 完了条件

- feeder thread の待機を stop signal で中断でき、`Drop` が sample duration の残り時間だけ停止しない
- stop flag 設定済みなら park せずに即座に停止する
- unpark だけでは deadline 前に送信継続を返さない（loop で deadline と stop flag を再評価する）
- 実 thread と `std::sync::Barrier`、mpsc channel を使うテストで、park 中の wait が stop + unpark により終了し、終了通知を `recv_timeout` で受け取れることを確認する（`join` のブロック時間は計測しない）
- mock / stub、`thread::sleep` は使わず、`Drop` と同じ stop / unpark / join 順序でテストする
- 検証は 0098 マージ後に分離した本 issue のブランチ上で行う
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
