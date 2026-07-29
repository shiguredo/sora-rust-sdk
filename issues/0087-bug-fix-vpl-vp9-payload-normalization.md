# VPL VP9 payload の誤ったヘッダー除去を直す

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-vpl-vp9-payload-normalization
- Polished: {YYYY-MM-DD}

## 目的

VPL が返す raw VP9 payload の先頭データを誤って削除せず、正しい VP9 bitstream を WebRTC へ渡す。

## 優先度根拠

High。VPL の VP9 encoder を利用するだけで payload が破損し、映像を正常に送信できない。

## 現状

`vp9_payload_from_vpl` は、`DKIF` file header の有無だけを確認した後、常に 12 byte を IVF frame header とみなして削除する。
現在利用している VPL wrapper は IVF header 出力を有効化しておらず、通常は raw VP9 bitstream を返す。

## 設計方針

- VPL wrapper が返す出力形式を明確な API 契約として扱う
- raw VP9 を採用する場合は、無条件の 12 byte 除去を廃止する
- IVF を受け入れる場合は、file header と frame header の構造およびサイズを検証してから除去する

## 完了条件

- raw VP9 payload が 1 byte も欠落せず callback へ渡される
- 正常な IVF 入力を許容する場合は正しく payload を抽出できる
- raw、IVF、truncated input のテストがある
- VPL 実機で VP9 の送受信が成功する
