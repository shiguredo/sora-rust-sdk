# zlib 展開後サイズを制限する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-limit-zlib-decompression-size
- Polished: 2026-07-29

## 目的

圧縮 DataChannel メッセージの展開後サイズを制限し、高圧縮率データによるメモリ枯渇とプロセス停止を防ぐ。

## 優先度根拠

High。リモートから受信したデータだけで無制限のメモリ確保が発生し、プロセスが OOM で停止し得る。

## 現状

`zlib` モジュールの `decompress_zlib` は、one-shot API が返す展開結果全体を `Vec<u8>` として受け取る。
`SoraConnection::handle_datachannel_message` は、圧縮が有効な DataChannel メッセージをこの関数で展開するが、展開後サイズを検査していない。
`noflate::zlib::decompress` も内部では圧縮入力全体を 1 回の `Decoder::feed` に渡してから全出力を `Vec<u8>` へコピーするため、展開後の事後検査ではメモリ枯渇を防げない。

## 再現条件

1. `compress_zlib` で上限を超える長さの反復データを圧縮する
2. 生成した小さい圧縮データを現在の `decompress_zlib` に渡す
3. 展開完了までエラーにならず、展開結果全体が確保されることを確認する

## 設計方針

- DataChannel メッセージの展開後サイズ上限を、SDK 内部の定数 `MAX_DECOMPRESSED_DATA_CHANNEL_MESSAGE_SIZE` で 16 MiB と定義する
  - 同じ接続で使う WebSocket 実装の `DEFAULT_MAX_DECOMPRESSED_SIZE` と同じ値にそろえる
  - WebSocket 側の定数は SDK から参照できないため、独立した定数として定義する
  - 公開設定 API は追加せず、本 issue を受信データの防御的な上限追加に限定する
- `decompress_zlib` が展開後サイズ上限を引数で受け取るようにし、`SoraConnection::handle_datachannel_message` から上記定数を渡す
- `noflate::zlib::Decoder` へ圧縮入力全体を一度に渡さず、4 KiB の固定チャンクに分割して `feed` する
- 各 `feed` の直後に `output()` の長さを取得し、累積長との加算を `checked_add` で検査する
  - 累積長が上限を超える場合は、出力を結果の `Vec<u8>` へ追加せずにエラーを返す
  - 上限以内の場合だけ出力を追加し、追加した長さを `advance` して decoder から排出する
- 全入力の供給後に `is_finished()` を検査し、trailer まで完了していない入力は従来どおりエラーにする
- 上限超過は受信本文を含まない `io::ErrorKind::InvalidData` として返し、公開エラー型へ新しい variant を追加しない
- 本 issue は展開処理が上限超過をエラーとして返すところまでを対象とする
  - そのエラーをメッセージ単位で破棄するか接続全体へ伝播するかは、issue 0093 で扱う
- `decompress_zlib` の private な単体テストを `zlib` モジュール内に追加し、テストのために API を公開しない
  - 境界テストは小さい上限値を関数へ渡し、不要な大容量確保を避ける
  - 接続側が使う定数自体が 16 MiB であることは個別に検査する

`noflate` は 1 回の `feed` 中に出力上限で処理を中断する API を持たない。
そのため、返却する `Vec<u8>` は 16 MiB 以下に制限し、展開中の一時メモリは 16 MiB、4 KiB の圧縮入力 1 チャンクから生成される有界な出力、decoder の作業領域の合計に抑える。
総メモリが 16 MiB を 1 バイトも超えないことは要件にしない。

## 完了条件

- 上限を超える入力に対して、返却用 `Vec<u8>` が 16 MiB を超える前に `io::ErrorKind::InvalidData` を返す
- 圧縮入力は 4 KiB ごとに `feed` され、各回の出力が検査後に `advance` される
- 展開中の保持量が圧縮入力全体の展開後サイズに比例せず、設計方針に記載した範囲へ有界化される
- 上限未満と上限ちょうどの zlib データを従来どおり展開できる
- 上限 + 1 バイト、高圧縮率入力、上限 0 での空ペイロードを表す正常な zlib ストリーム、trailer 未完了、Adler-32 不一致を検証する単体テストがある
- モックやスタブを使わずにテストされている
