# zlib 展開後サイズを制限する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-limit-zlib-decompression-size
- Polished: {YYYY-MM-DD}

## 目的

圧縮 DataChannel メッセージの展開後サイズを制限し、高圧縮率データによるメモリ枯渇とプロセス停止を防ぐ。

## 優先度根拠

High。リモートから受信したデータだけで無制限のメモリ確保が発生し、プロセスが OOM で停止し得る。

## 現状

`zlib` モジュールの `decompress_zlib` は、one-shot API が返す展開結果全体を `Vec<u8>` として受け取る。
`SoraConnection::handle_datachannel_message` は、圧縮が有効な DataChannel メッセージをこの関数で展開するが、展開後サイズを検査していない。

## 設計方針

- 展開処理を出力サイズ上限付きのストリーミング処理へ変更する
- 圧縮前サイズではなく展開後サイズで制限する
- 上限超過を専用エラーとして呼び出し側へ返す
- 正常な圧縮メッセージとの互換性を維持する

## 完了条件

- 展開後サイズが上限を超える入力で、上限を超えたメモリ確保が発生しない
- 上限以内の zlib データを従来どおり展開できる
- 高圧縮率入力と境界値を検証するテストがある
- モックやスタブを使わずにテストされている
