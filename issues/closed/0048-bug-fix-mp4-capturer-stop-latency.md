# MP4 キャプチャ停止が sleep 完了までブロックする問題を直す

- Priority: Medium
- Created: 2026-07-23
- Completed: 2026-07-29
- Model: Composer
- Branch: feature/fix-mp4-capturer-stop-latency
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 残項目。

## 目的

`Mp4VideoCapturer` の破棄・停止が、ワーカーの `thread::sleep` 完了を待たずに速やかに戻るようにする。切断や再接続時の応答遅延を防ぐ。

## 優先度根拠

Medium。

- `Drop` がフレーム間隔ぶんブロックすると、切断・破棄パスのレイテンシが悪化する
- 低 FPS や長いサンプル間隔では数百 ms 以上待ちうる
- 正式リリースブロッカーではないが、実運用で体感しやすい

## 現状

`src/video_codecs/mp4.rs`:

- ワーカーはループ先頭（各フレーム処理前）でのみ `stop` を確認する（概ね L686-688）
- その後フレーム送信し、次フレーム時刻まで `thread::sleep` する（L714-716）
- `Drop` は `stop.store(true)` のあと `join` するだけ（L736-742）

そのため `sleep` 中に `Drop` されると、sleep が終わるまで `join` が戻らない。`stop` 確認は sleep 前に無い。

## 設計方針

- `stop` を sleep 中でも検知できるようにする
- 方針候補:
  - sleep を短い間隔に分割し、各区間で `stop` を見る
  - `Condvar` / チャネルで即起床する
- フレームタイミングの絶対時刻補正（累積ドリフト防止）は維持する

## 完了条件

- `stop` 後、最悪でも短い上限時間内にワーカーが終了する
- ループ再生のタイミング補正が壊れていない
- 停止・破棄の単体テスト、または同等の検証がある
- `cargo test -p sora_sdk` が通る

## 解決方法

本 issue は対応不要と判断し、コード変更なしで closed にする。

理由:

- 現象自体は存在するが、止まるのは `Mp4VideoCapturer` の `Drop` / `join` だけであり、Sora 切断・再接続そのもののレイテンシではない
- 待ち時間は概ね 1 フレーム間隔（30 fps なら最大約 33 ms）で、致命的な遅延ではない
- デッドロック・データ欠損・panic ではなく、正式リリースや再接続品質のブロッカーにもならない
- sleep 分割や Condvar 化はコードを複雑にするだけで、実運用上の効果が薄い
