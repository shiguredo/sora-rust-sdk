# sumomo の capture format と変換処理を一致させる

- Priority: Medium
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-sumomo-capture-format
- Polished: {YYYY-MM-DD}

## 目的

video device が選択する pixel format を sumomo が変換可能な形式に限定し、接続成功後に全 frame が無言で破棄される状態を防ぐ。

## 優先度根拠

Medium。利用する device に依存するが、I420 や MJPEG が選択された場合に映像が 1 frame も送信されない。

## 現状

`VideoDeviceCapturer::new` は pixel format を指定せず device 側の選択に任せる。
capture callback は NV12 と YUY2 だけを変換し、それ以外の format を通知なしで破棄する。

## 設計方針

- device selection 時に変換可能な pixel format を要求する
- または device が返し得る全対応 format の変換を実装する
- 非対応 format は開始時に明示的な error とする
- callback 内で frame を無言破棄しない

## 完了条件

- 選択された capture format を必ず変換できる
- 非対応 format は capture 開始前に失敗する
- NV12、YUY2、および採用する追加 format を実 device で確認する
- frame 受信と video track 送信を検証するテストがある
