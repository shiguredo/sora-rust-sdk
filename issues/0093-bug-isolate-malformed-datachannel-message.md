# 不正に圧縮された DataChannel メッセージの影響を接続全体へ波及させない

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-malformed-datachannel-message
- Polished: 2026-07-29

## 目的

不正な圧縮データを含む DataChannel メッセージを受信しても、`SoraConnection::run` 全体を終了させない。

## 優先度根拠

High。`#` で始まる利用者定義 DataChannel は Sora が他の参加者の中継を行うため、任意の参加者が不正なデータを送信できる。
圧縮設定のある DataChannel では受信側が zlib 展開を行うため、不正な圧縮データを 1 件送るだけで、正常な PeerConnection と他の DataChannel を含む接続全体を切断できる。

## 現状

main event loop は `SoraConnection::handle_datachannel_message` のエラーを `?` で伝播する。
同関数は zlib 展開の失敗をそのまま返すため、zlib 展開失敗は callback 前に接続を終了する。

具体的には次の順序で処理している。

1. 設定に応じて zlib を展開する
2. 全 label について `on_data_channel_message` を呼ぶ
3. `signaling` / `stats` / `push` / `notify` / `rpc` は UTF-8 へ変換する
4. `signaling` は `on_signaling_message` を呼ぶ
5. `IncomingMessage::parse` または `RpcResponse::parse` で JSON と schema を解析する
6. parse 済み message に応じた処理を実行する

このため zlib 展開失敗では callback 前に接続を終了する。

## 設計方針

### 対象範囲

zlib 展開失敗だけを回復可能な異常として扱い、メッセージ 1 件単位で破棄する。

- warning を出して `handle_datachannel_message` は `Ok(())` を返す
- 対象 DataChannel を close しない
- `SoraConnection::run` と PeerConnection を終了しない
- 同じ DataChannel と他の DataChannel の後続メッセージを処理できる

UTF-8 変換と JSON / schema / RPC の parse 失敗は対象外とし、現行どおり `SoraConnection::run` の終了原因とする。

- `signaling` / `stats` / `push` / `notify` / `rpc` に届くデータは Sora サーバーが送信元であり、他参加者が直接注入する経路がない
- 制御チャンネル (`signaling`) の parse 失敗を黙って破棄すると re-offer / close を見逃して desync したまま継続するため、現行どおり接続全体のエラーとして扱う
- `handle_datachannel_message` 全体の `Err` を呼出側で一律に握り潰さない

### label と失敗段階ごとの処理

| label | 失敗段階 | 処理 |
| --- | --- | --- |
| 全 label | zlib 展開 | メッセージを破棄する。callback は呼ばず、DataChannel と接続を維持する |
| `signaling` / `stats` / `push` / `notify` / `rpc` | UTF-8 | 現行どおり run の終了原因にする (変更しない) |
| `signaling` / `stats` / `push` / `notify` / `rpc` | JSON syntax / schema / field type / message type | 現行どおり run の終了原因にする (変更しない) |
| `#` で始まる利用者定義 label | zlib 展開後 | 任意の binary data を正常入力として扱い、UTF-8 / JSON parse を行わない |
| 未対応 label | zlib 展開後 | 現行どおり raw callback を通知し、unsupported label warning を出す。UTF-8 / JSON parse を行わない |

- zlib 展開は label 非依存の共通経路のため、`#` ラベルの任意 binary data も展開失敗から保護される
- zlib 展開サイズの制限自体は issue 0085 で実装済み
  - 上限超過の `InvalidData` も、他の zlib 展開失敗と同じく本 issue でメッセージ単位に破棄する
- JSON-RPC response の `jsonrpc`、必須 `id`、`result` / `error` の意味検証強化と protocol error の完了方法は issue 0094 の責務とする
  - 本 issue では `rpc` ラベルの UTF-8 / JSON parse 失敗を対象外にしたため、0094 は rpc ラベルの UTF-8 / JSON syntax error と protocol violation を自ら非 fatal に処理する

### callback の互換性

- zlib 展開に失敗した場合
  - 展開後 bytes が存在しないため、どの callback も呼ばない
- zlib 展開に成功した場合
  - 全 label の `on_data_channel_message` は現行どおり 1 回呼ぶ
- `signaling` label で UTF-8 変換に成功した場合
  - `on_signaling_message` は JSON parse の成否にかかわらず現行どおり 1 回呼ぶ
- `#` label の任意 binary data は現行どおり `on_data_channel_message` の後に `on_message` へ渡す

raw callback が受信本文を利用者へ返す責任境界は変更しない。
SDK 自身の warning log へ受信本文を出力しない。

### ログ

- 新たに出す zlib 破棄 warning は英語の固定文とし、どの DataChannel で失敗したかを特定できるようラベル名と失敗段階 (zlib) を含める
  - ラベル名は Sora サーバーが offer で設定する値であり、通常ログにもすでに出力されている
  - `signaling` / `stats` / `push` / `notify` / `rpc` には Sora サーバーのデータしか届かないため zlib 展開失敗は稀で、warning は主に `#` ラベルの任意データで発生する
- warning には圧縮前後の本文、zlib の error message、metadata を含めない
- 同一入力による warning の頻度制限は本 issue の対象外とする

## 変更対象

- `src/connection.rs`
- `CHANGES.md`

## 完了条件

- zlib 展開失敗が `SoraConnection::run` を終了させず、メッセージ 1 件だけを破棄する
- 次の message を順に処理する、モックやスタブを使わないテストがある
  1. zlib 展開に失敗する message
  2. 同じ label の正常 message
  3. 別 label の正常 message
- 上記テストは実際の `SoraConnectionContext` と `SoraConnection` を構築し、private な message 処理経路を呼ぶ
  - WebRTC 型の fake、mock、stub を追加しない
  - zlib 展開失敗の処理が `Ok(())` を返し、その後の正常 message が semantic callback まで到達することを確認する
- zlib について次を確認する
  - 不正 header、truncated stream、Adler-32 不一致、展開サイズ上限超過を各 1 件だけ破棄する
  - どの callback も呼ばれない
  - 同じ DataChannel と接続は維持される
- `#` label の UTF-8 ではない正常な binary message が、`on_data_channel_message` と `on_message` に各 1 回渡される
- 未対応 label の UTF-8 ではない正常な binary message が、`on_data_channel_message` に 1 回渡され、接続を終了しない
- `signaling` label の不正 UTF-8 と JSON syntax error が、現行どおり `SoraConnection::run` の終了原因になることを確認する
- parse に成功した `re-offer` と不正 SDP の組み合わせで SDP 適用 error が返ることを確認する
- parse に成功した `ping` を signaling DataChannel が未登録の実際の `SoraConnection` で処理し、`DataChannelMissing` が返ることを確認する
- 新たな zlib 破棄 warning にラベル名と失敗段階 (zlib) が含まれ、圧縮前後の本文、zlib の error message、metadata が含まれないことをコードレビューで確認する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
- `cargo test --workspace` が成功する
