# DataChannel 受信イベントキューを有界化する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-bound-datachannel-event-queue
- Polished: 2026-07-29

## 目的

DataChannel 受信イベントの滞留量を制限し、受信速度が処理速度を上回った場合のメモリ枯渇を防ぐ。

## 優先度根拠

High。リモートがメッセージを連続送信すると、所有された受信データが無制限に蓄積し、プロセス停止につながる。

## 現状

`SoraConnection::new` はイベント配送に unbounded channel を使用する。
`DcObsHandler::on_message` は受信データを `Vec<u8>` に複製してからキューへ投入するため、キューの要素数と総バイト量に上限がない。
`SoraConnection::run` は同じキューからイベントを 1 件ずつ処理し、DataChannel メッセージの処理では非同期処理も行うため、同期 callback からの投入速度が処理速度を上回ると所有データが蓄積し続ける。

## 再現条件

1. DataChannel から、イベントハンドラーとシグナリング処理が消費できる速度を上回ってメッセージを連続送信する
2. `DcObsHandler::on_message` が各メッセージのラベルと本文を所有データへ複製し、`SoraEvent::DataChannelMessage` として投入することを確認する
3. 受信を継続する限り、未処理イベントの件数と本文の合計サイズが増え続けることを確認する

## 設計方針

- 既存の制御イベント用 unbounded channel は維持する
  - `SoraEvent` 全体を bounded channel に置き換えると、DataChannel メッセージの滞留により track、ICE candidate、DataChannel の登録・状態遷移、RPC timeout まで欠落し得る
  - DataChannel メッセージだけを enqueue 前の admission control で制限し、既存のイベント順序と redirect・shutdown 時の drain を維持する
- 接続全体で未処理の DataChannel メッセージを最大 64 件、本文の合計を最大 16 MiB に制限する
  - 件数上限は空または小さいメッセージによるイベント本体とラベルの蓄積を制限する
  - バイト上限は本文の `Vec<u8>` が保持する payload の合計を制限し、イベント本体、ラベル、allocator の固定オーバーヘッドは件数上限で拘束する
  - 16 MiB は issue 0085 の 1 メッセージあたりの展開後上限とそろえ、256 KiB のメッセージなら 64 件で同じ上限へ到達する値とする
  - 公開設定 API は追加せず、内部定数として定義する
- 件数予算とバイト予算には、それぞれ `tokio::sync::Semaphore` を使用する
  - `DcObsHandler::on_message` は `data.to_vec()` より前に `try_acquire_owned` で件数 permit を取得する
  - `data.len()` を `u32` へ検査付きで変換し、`try_acquire_many_owned` でバイト permit を取得する
  - 変換不能な長さと 16 MiB を超える単一メッセージは、本文を複製する前に上限超過として扱う
  - 両 permit を private な DataChannel メッセージイベントに保持し、メインループでの処理完了、redirect の drain、送信失敗、receiver の破棄のいずれでも RAII により返却する
- 上限超過ではメッセージを黙って drop せず、接続を fail closed にする
  - 接続全体で共有する `AtomicBool` を `compare_exchange` で更新し、最初の上限超過だけを `SoraEvent::DataChannelReceiveQueueOverflow` として制御イベントキューへ通知する
  - callback 側の更新には `AcqRel`、失敗時とメインループ側の読み取りには `Acquire` を使用する
  - callback の入口で上限超過フラグが立っていれば、本文を複製せず直ちに return する
  - フラグ設定と競合して既に permit を取得した callback は enqueue してよいが、件数とバイト数の上限内に収める
  - overflow event はメインループを起こす通知として使い、上限超過の判定では共有フラグを優先する
  - メインループは各周回の `tokio::select!` 前と、受信した `SoraEvent` を dispatch する前に共有フラグを検査し、overflow event より前に並ぶ DataChannel メッセージを欠落発生後に処理しない
  - メインループは本文やラベルを含まない英語の warning ログを 1 回だけ出力し、`Error::DataChannelReceiveQueueOverflow` を返して `run` を終了する
  - redirect では、事前検査、古い DataChannel と observer の破棄、イベントキューの drain、事後検査の順に処理する
  - 事後検査より後に発生した上限超過は、キューに残る overflow event とメインループ先頭の検査で処理する
- 公開 API の追加は `Error::DataChannelReceiveQueueOverflow` variant だけに限定し、新しい callback や設定項目は追加しない
  - 内部予算の超過は実際のメモリ割り当て失敗ではないため、`io::ErrorKind::OutOfMemory` には偽装しない
  - `Error` は `#[non_exhaustive]` ではないため variant 追加が外部の網羅的な `match` を壊すことを認識し、正式リリース前の段階で正確に判別できるエラーを追加する判断を優先する
  - `Display` には既存規約に合う日本語メッセージを追加し、`std::error::Error::source()` は `None` とする
- WebRTC の同期 callback では `send().await`、`blocking_send`、待機を伴う semaphore 取得を使用しない
- admission control を module scope の private な型へ分離し、外部接続やテスト専用 API なしで検証できるようにする
  - 長さから必要 permit 数を求める処理も private 関数へ分離し、巨大なバッファを確保せず `usize::MAX` の変換失敗を検証できるようにする
  - 共有フラグの検査と共通エラー生成、redirect の drain 前後検査も private 関数へ分離する

payload の保持量は 16 MiB 以下、未処理または処理中の DataChannel メッセージは 64 件以下に制限する。
ラベル、イベント本体、channel node、allocator のオーバーヘッドはこの 16 MiB には含めず、64 件の上限で有界化する。

## 完了条件

- 64 件の件数上限と 16 MiB の payload 合計上限が、DataChannel メッセージの複製前に適用される
- 上限ちょうどまでは受理し、件数超過、バイト超過、16 MiB を超える単一メッセージ、`u32` へ変換できない長さを拒否する
- 上限超過フラグを観測した callback は新しい本文を複製せず、上限超過通知と warning ログが接続ごとに 1 回だけ発生する
- `SoraConnection::run` が `Error::DataChannelReceiveQueueOverflow` を返し、メッセージ欠落後にキュー内の DataChannel メッセージを処理しない
- `Error` variant 追加の破壊的影響を、`CHANGES.md` の `develop` に `[CHANGE]` として記載する
- DataChannel メッセージの処理完了、redirect の drain、イベント送信失敗、receiver の破棄で permit が返却される
- private な admission-control 型を、実際の tokio channel、semaphore、`Vec<u8>` で単体テストする
  - 件数とバイト数の上限ちょうどと超過、空メッセージ、単一の過大メッセージ、`usize::MAX` の長さ、FIFO、permit の返却、通知の集約を検証する
  - redirect の drain 前後で競合する通知と、overflow event より前に並んだメッセージを dispatch しないことを検証する
  - モックやスタブ、外部の Sora 接続、テスト専用の公開 API を使用しない
- 通常の DataChannel メッセージ配送順と、DataChannel 以外の `SoraEvent` 配送に回帰がない
