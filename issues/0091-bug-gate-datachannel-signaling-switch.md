# switched 受信前の DataChannel シグナリング切替を防ぐ

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-datachannel-signaling-switch
- Polished: 2026-07-29

## 目的

正式な `switched` メッセージを受信する前に、送信シグナリング経路が DataChannel へ切り替わる競合を防ぐ。

## 優先度根拠

High。正常な DataChannel シグナリング構成でも、Open event と `switched` の到着順によって誤った経路へメッセージを送信し得る。
たとえば、`signaling` を含む DataChannel の Open event が先行すると、現行実装は `switched` 前でも `use_datachannel_signaling=true` にする。
この直後に WebSocket の Ping を受信すると、Pong が正式な切替前の DataChannel へ誤送信される。

## 現状

`SoraConnection::handle_datachannel_state` は、opened DataChannel 数が設定数と一致しただけで DataChannel シグナリングを有効にする。
`switched_received` は別に管理されており、切替判定へ渡されない。

個数比較のため、設定外の label が Open すると、`signaling` が未 Open でも偶然同数になり得る。
反対に、リアルタイムメッセージング用の `#` label が未 Open だと、送信シグナリングに必要な `signaling` が Open 済みでも切替を妨げる。

WebSocket の EOF、flush error、Close output / Closed state を無視して DataChannel シグナリングを継続する判定も、`switched_ignore_disconnect_websocket` だけを参照し、ローカルの `signaling` DataChannel が利用可能かを確認しない。

## 設計方針

### 切替 readiness

- 送信シグナリング経路の readiness を次の 3 条件の論理積として定義する
  - WebSocket 経由で正式な `switched` メッセージを受信済み
  - Offer の `data_channels` に `signaling` label が存在する
  - ローカルの opened label 集合に `signaling` が存在する
- `signaling` は `re-answer` と `disconnect` を送る DataChannel であり、送信シグナリング経路の成立に直接必要な唯一の label とする
- `stats`、`notify`、`push`、`rpc`、将来追加される別の内部 label は readiness の条件に含めない
- `#` で始まるユーザー定義 label と、Offer にない予期しない label は、Open / 未 Open のどちらでも readiness を変えない
- `data_channel_configs.len()` と `opened_datachannels.len()` の個数比較は削除する
- readiness 判定を `switched_received`、Offer の config、opened label 集合だけを読む private な pure helper に抽出する
- `use_datachannel_signaling` は false から true への切替状態として維持し、readiness が初めて成立したときだけ true にする
  - 本 issue は正式な初回切替前の競合を修正する
  - 切替完了後に `signaling` DataChannel が Closed になった場合の終了方針や WebSocket fallback は変更しない

### 再評価契機

- 次の両方の event 処理直後に、同じ pure helper で readiness を再評価する
  - `IncomingMessageData::Switched` を受信し、`switched_received` を true にした直後
  - `DataChannelRegister` / `DataChannelStateChange` で opened label 集合を更新した直後
- Open → switched と switched → Open のどちらの順序でも、後から成立した条件を処理した時点で DataChannel 経路へ 1 回だけ切り替える
- `on_switched` は Sora の `switched` 受信を表す既存 callback であり、`signaling` の Open 待ちで遅延させない
- redirect では `switched_received`、`switched_ignore_disconnect_websocket`、`use_datachannel_signaling`、opened label 集合、Offer の DataChannel config、WebSocket 切断 delay を既存どおり全て初期化する
  - redirect 前の readiness を持ち越さず、新しい Offer、Open、`switched` だけで再成立させる

### 送信経路と WebSocket 終了判定

- `SoraEvent::SignalingMessage` は、`use_datachannel_signaling=false` かつ WebSocket が Connected なら、`switched` の受信有無にかかわらず WebSocket へ送る
- `use_datachannel_signaling=true` になった後だけ `signaling` DataChannel へ送る
- `use_datachannel_signaling` と WebSocket の Connected 状態から `Some(SignalingType::WebSocket)`、`Some(SignalingType::DataChannel)`、`None` を返す private な pure route helper を作る
  - `SoraEvent::SignalingMessage` の送信先選択と `on_signaling_message` の `SignalingType` は、この helper の結果を 1 箇所で使用する
  - `None` の場合に送信しない既存挙動は変更しない
- readiness 成立前に WebSocket が利用不能になった場合は、DataChannel へ早期 fallback せず、既存の WebSocket 終了または error として処理する
- `switched_ignore_disconnect_websocket && use_datachannel_signaling` を、WebSocket 切断を無視できる単一の条件とする
  - `use_datachannel_signaling` は初回切替完了を保持する latch であり、切替後の DataChannel の live availability を表すものではない
- 次の既存 4 箇所へ同じ条件を適用する
  - read が EOF または `UnexpectedEof` になった場合の継続判定
  - `flush_ws_output` error の吸収判定
  - `ConnectionOutput::CloseConnection` または `ConnectionState::Closed` 後の継続判定
  - SDK 側の WebSocket close delay の開始と close 実行
- 通常の read error、`feed_recv_buf` error、timer error、`ConnectionEvent::Error` の既存 error policy は変更しない

## 変更対象

- `src/connection.rs`
- `e2e-tests/tests/messaging.rs`
- `CHANGES.md`

## 完了条件

- pure readiness helper について、モックやスタブを使わず次を単体テストする
  - `signaling` Open → switched の順では、Open だけの時点は false、switched 後は true
  - switched → `signaling` Open の順では、switched だけの時点は false、Open 後は true
  - `signaling` が Offer の config にない場合は、同名の予期しない opened label があっても false
  - `signaling` が config にあっても未 Open なら false
  - `#` label が未 Open でも true になり、Open 済みでも結果が変わらない
  - `stats`、`notify`、`push`、`rpc`、Offer にない label の Open / 未 Openで結果が変わらない
  - redirect 相当として switched、config、opened label を初期化すると false に戻る
- `use_datachannel_signaling` を更新する全箇所が pure helper を使用し、個数比較が残っていないことをコードレビューで確認する
- pure route helper について、モックやスタブを使わず次を単体テストする
  - `use_datachannel_signaling=false` かつ WebSocket Connected なら WebSocket
  - `use_datachannel_signaling=true` なら WebSocket の状態にかかわらず DataChannel
  - `use_datachannel_signaling=false` かつ WebSocket が Connected でなければ `None`
- readiness helper と route helper を同じ状態遷移テストで組み合わせ、Open → switched と switched → Open の両順序について次を確認する
  - readiness 成立前の同じ signaling message は WebSocket を選ぶ
  - readiness 成立直後の同じ signaling message は DataChannel を選ぶ
- `SoraEvent::SignalingMessage` の production 分岐が pure route helper を使用し、inline の経路選択条件が残っていないことをコードレビューで確認する
- `on_switched` は `switched` 受信時に従来どおり通知し、`signaling` の Open まで遅延させない
- readiness 成立前の EOF / `UnexpectedEof`、WebSocket flush error、Close output / Closed state を、`switched_ignore_disconnect_websocket=true` だけを理由に吸収しない
- readiness 成立後は `switched_ignore_disconnect_websocket=true` の場合に限り、同じ 3 種の WebSocket 終了を吸収して DataChannel シグナリングを継続する
- SDK 側の WebSocket close delay は readiness 成立後にだけ開始し、readiness 成立前には WebSocket close を要求しない
- redirect 後は readiness が false であり、新しい Offer の `signaling` Open と新しい `switched` の両方が揃うまで再成立しない
- pure helper の単体テストを経路選択競合の主回帰テストとし、既存の `e2e-tests/tests/messaging.rs` は `#messaging` が Offer に共存する実 Sora 接続の補完テストとして拡張する
  - 2 クライアント目の参加で 1 クライアント目が受信する `re-offer` と、返信する `re-answer` が `on_signaling_message` で `SignalingType::DataChannel` として観測される
  - 双方向メッセージを送信する前に、両クライアントで `#messaging` の Open を明示的に待つ
  - ユーザー定義 `#messaging` の双方向メッセージ送受信も従来どおり成功する
  - `TEST_SIGNALING_URLS` がない場合や `switched` / DataChannel signaling message を観測できない場合は、skip または成功扱いにしない
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
- `cargo test --workspace` が成功する
