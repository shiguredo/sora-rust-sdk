# feeder thread の deadline 計算を checked_add で安全化する

- Priority: High
- Created: 2026-08-10
- Completed: 2026-08-10
- Branch: feature/fix-mp4-deadline-overflow
- Polished: {YYYY-MM-DD}

## 目的

`Mp4VideoCapturer` の feeder thread がフレームの絶対送信時刻を計算する際に、オーバーフローで panic または wraparound しないようにする。

## 現状

`Mp4VideoCapturer` の feeder thread は、フレームの絶対送信時刻を `loop_start + Duration::from_micros(...)` で未検査に計算する。
`cumulative_us` は `u64` マイクロ秒（動画全体の長さに比例）で、`Instant` の表現範囲（プラットフォーム依存）を超える場合に overflow する。

## 設計方針

- `loop_start + Duration::from_micros(...)` を `checked_add` へ変更する
- オーバーフローした場合は panic や飽和を避け、英語の production log を残してフィーダースレッドを正常停止する
- 累積再生時間が `Instant` の表現範囲を超えるのは、再生時間が極めて長い破損入力に限られるため、overflow のテストは行わない（コードの `checked_add` 化と完了条件の検査で担保する）

## 完了条件

- deadline の計算が `checked_add` で行われ、オーバーフロー時に panic や飽和を避けて正常停止する
- オーバーフローが現実的には発生しない理由（再生時間が極めて長い破損入力のみ）と、テストを行わない旨がコメントに明記されている
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 解決方法

- `Mp4VideoCapturer` の feeder thread の deadline 計算を `loop_start + Duration::from_micros(...)` から `loop_start.checked_add(...)` に変更した
  - `let-else` 構文でオーバーフローを検出し、英語の production log を残してフィーダースレッドを正常停止する
  - オーバーフローが現実的には発生しない理由（再生時間が極めて長い破損入力のみ）と、テストを行わない旨をコメントに明記した
- `CHANGES.md` の develop セクションに `[FIX]` を追記した
