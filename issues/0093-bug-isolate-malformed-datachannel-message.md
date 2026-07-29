# 不正 DataChannel メッセージの影響を接続全体へ波及させない

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-malformed-datachannel-message
- Polished: 2026-07-29

## 目的

不正な圧縮データ、UTF-8、JSON を含む DataChannel メッセージを受信しても、`SoraConnection::run` 全体を終了させない。

## 優先度根拠

High。リモートから不正メッセージを 1 件送るだけで、正常な PeerConnection と他の DataChannel を含む接続全体を切断できる。

## 現状

main event loop は `SoraConnection::handle_datachannel_message` のエラーを `?` で伝播する。
同関数は zlib、UTF-8、JSON の parse error をそのまま返す。

具体的には次の順序で処理している。

1. 設定に応じて zlib を展開する
2. 全 label について `on_data_channel_message` を呼ぶ
3. `signaling` / `stats` / `push` / `notify` / `rpc` は UTF-8 へ変換する
4. `signaling` は `on_signaling_message` を呼ぶ
5. `IncomingMessage::parse` または `RpcResponse::parse` で JSON と schema を解析する
6. parse 済み message に応じた処理を実行する

このため zlib 展開失敗では callback 前に接続を終了する。
UTF-8 または JSON の失敗では raw callback の一部を呼んだ後に接続を終了する。

## 設計方針

### 異常入力と運用エラーの分離

- リモート入力の decode / parse error だけを回復可能な `DataChannelMessageDecodeError` 相当の private な分類へ変換する
- 回復可能な error は warning を出して受信メッセージ 1 件だけを破棄し、`handle_datachannel_message` は `Ok(())` を返す
  - 対象 DataChannel を close しない
  - `SoraConnection::run` と PeerConnection を終了しない
  - 同じ DataChannel と他の DataChannel の後続メッセージを処理できる
- parse 成功後の処理で発生した運用エラーは回復可能扱いにしない
  - `handle_offer` による SDP 適用
  - stats の取得後に行う送信
  - `send_signaling_message` / `send_stats_message`
  - その他の PeerConnection 操作または送信
- `handle_datachannel_message` 全体の `Err` を呼出側で一律に握り潰さない
- 回復可能な decode / parse と、副作用を伴う parse 後の処理を別の private helper または内部 outcome で分離し、後者の error は従来どおり main event loop へ伝播する

### label と失敗段階ごとの処理

| label | 失敗段階 | 処理 |
| --- | --- | --- |
| 全 label | zlib 展開 | メッセージを破棄する。callback は呼ばず、DataChannel と接続を維持する |
| `signaling` / `stats` / `push` / `notify` | UTF-8 | 展開後 bytes の raw callback だけを通知した後、メッセージを破棄する |
| `signaling` / `stats` / `push` / `notify` | JSON syntax / schema / field type / message type | 現行の raw callback を通知した後、semantic 処理をせずメッセージを破棄する |
| `rpc` | UTF-8 または `RpcResponse::parse` | 展開後 bytes の raw callback だけを通知した後、pending RPC を変更せずメッセージを破棄する |
| `#` で始まる利用者定義 label | zlib 展開後 | 任意の binary data を正常入力として扱い、UTF-8 / JSON parse を行わない |
| 未対応 label | zlib 展開後 | 現行どおり raw callback を通知し、unsupported label warning を出す。UTF-8 / JSON parse を行わない |

- `IncomingMessage::parse` が返す JSON syntax、必須 field、field type、`CandidateNotSupported`、`UnsupportedMessageType` を全て message 単位の parse error として扱う
- zlib 展開サイズの制限自体は issue 0085 の責務とする
  - issue 0085 で追加する上限超過の `InvalidData` も、他の zlib 展開失敗と同じく本 issue でメッセージ単位に破棄する
  - issue 0085 を先に実装する
- JSON-RPC response の `jsonrpc`、必須 `id`、`result` / `error` の意味検証強化と protocol error の完了方法は issue 0094 の責務とする
  - 本 issue で message 単位に破棄する RPC error は、UTF-8 / JSON の decode または現行 `RpcResponse::parse` に失敗し、信頼できる response `id` が解析結果として返っていない場合に限定する
  - issue 0094 は、信頼できる `id` を特定できる protocol violation について、該当 pending request を `Err` で完了する設計を選べる
  - issue 0094 で別の structured outcome を追加する場合も、protocol violation 1 件を理由に `SoraConnection::run` を終了させない

### callback の互換性

malformed message を semantic message として処理しない一方、受信データを観測する公開 raw callback の現行契約と順序は維持する。

- zlib 展開に失敗した場合
  - 展開後 bytes が存在しないため、どの callback も呼ばない
- zlib 展開に成功した場合
  - 全 label の `on_data_channel_message` は、UTF-8 / JSON の成否にかかわらず現行どおり 1 回呼ぶ
- `signaling` label で UTF-8 変換に成功した場合
  - `on_signaling_message` は JSON parse の成否にかかわらず現行どおり 1 回呼ぶ
- UTF-8 または JSON / schema / message type の parse に失敗した場合
  - `on_notify`、`on_push`、offer / ping / stats 処理などの semantic callback / 処理は呼ばない
- `#` label の任意 binary data は現行どおり `on_data_channel_message` の後に `on_message` へ渡す

raw callback が受信本文を利用者へ返す責任境界は変更しない。
SDK 自身の warning log へ受信本文を出力しない。

### RPC pending 状態

- `RpcResponse::parse` が信頼できる response `id` を解析結果として返す前に、`pending_rpc_responses` から request を remove しない現行順序を維持する
- decode / parse が失敗して信頼できる `id` が返らない RPC response は、本文上に `id` らしき値があっても pending request と timeout handle を変更しない
- malformed response の後に同じ `id` の正常 response を受信した場合は、その response で request を正常に完了できる
- 正常 response が来なければ既存 timeout が request を完了する
- 信頼できる `id` が返らない malformed response では、response channel へ成功・protocol error のどちらも送らない
- 信頼できる `id` を伴う JSON-RPC protocol violation の response channel semantics は issue 0094 で定める

### ログ

- 回復可能な異常について新たに出す warning は、固定の英語文で label の分類と失敗段階だけを記録する
- 新たな回復 warning に raw label、圧縮前後の本文、JSON / RPC の error message、metadata を含めない
- 同一入力による warning の頻度制限は本 issue の対象外とする

## 変更対象

- `src/connection.rs`
- `CHANGES.md`

## 完了条件

- 回復可能な decode / parse error と parse 後の運用エラーが、型または private helper の境界で分離されている
- main event loop が `handle_datachannel_message` の運用エラーを従来どおり伝播する
- 次の message を順に処理する、モックやスタブを使わないテストがある
  1. malformed message
  2. 同じ label の正常 message
  3. 別 label の正常 message
- 上記テストは実際の `SoraConnectionContext` と `SoraConnection` を構築し、private な message 処理経路を呼ぶ
  - WebRTC 型の fake、mock、stub を追加しない
  - malformed message の処理が `Ok(())` を返し、その後の正常 message が semantic callback まで到達することを確認する
- zlib について次を確認する
  - 不正 header、truncated stream、Adler-32 不一致、issue 0085 の展開サイズ上限超過を各 1 件だけ破棄する
  - どの callback も呼ばれない
  - 同じ DataChannel と接続は維持される
- `signaling` / `stats` / `push` / `notify` について次を確認する
  - 不正 UTF-8、JSON syntax error、必須 field 欠落、field type 不一致、未対応 message type を各 1 件だけ破棄する
  - `on_data_channel_message` は zlib 展開成功時に 1 回呼ばれる
  - `signaling` の `on_signaling_message` は UTF-8 成功時に 1 回呼ばれる
  - `on_notify`、`on_push` と parse 後の処理は呼ばれない
- `#` label の UTF-8 ではない正常な binary message が、`on_data_channel_message` と `on_message` に各 1 回渡される
- 未対応 label の UTF-8 ではない正常な binary message が、`on_data_channel_message` に 1 回渡され、接続を終了しない
- malformed RPC response について次を確認する
  - UTF-8 error、JSON syntax error、信頼できる `id` を返せない field type error を各 1 件だけ破棄する
  - `on_data_channel_message` は zlib 展開成功時に 1 回呼ばれる
  - pending request、timeout handle、response channel が変更されない
  - 続けて受信した同じ `id` の正常 response だけが pending request を remove して response channel を 1 回完了する
- parse に成功した `re-offer` と不正 SDP の組み合わせで SDP 適用 error が返ることを確認する
- parse に成功した `ping` を signaling DataChannel が未登録の実際の `SoraConnection` で処理し、`DataChannelMissing` が返ることを確認する
- 新たな回復 warning に raw label、圧縮前後の本文、JSON / RPC の error message、metadata を渡す箇所がないことをコードレビューで確認する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
- `cargo test --workspace` が成功する
