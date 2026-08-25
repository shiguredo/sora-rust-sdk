# switched 受信前の DataChannel シグナリング切替を防ぐ

- Priority: High
- Created: 2026-07-29
- Completed: 2026-08-05
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

WebSocket の EOF、flush error、Close output / Closed state を無視して DataChannel シグナリングを継続する判定も、`switched_ignore_disconnect_websocket` だけを参照し、切替が成立しているか（`use_datachannel_signaling`）を確認しない。
切替成立前に WebSocket が切れると、送出シグナリングが WebSocket / DataChannel のどちらにも送られず黙って失われる。

## 前提（Sora 仕様）

- Offer の `data_channels` にない label が DataChannel として Open することはない
- `switched` を受信するということは DataChannel シグナリングが有効であり、Offer の `data_channels` に `signaling` が含まれる

この 2 つの前提から、切替条件に `signaling` の存在や Open を個別に確認する必要はない。
`switched` を受信済みで全設定チャンネルが Open 済みなら、`signaling` も必ず Open 済みである。

## 設計方針

### 切替 readiness

- 送信シグナリング経路の readiness を次の 2 条件の論理積として定義する
  - WebSocket 経由で正式な `switched` メッセージを受信済み
  - opened DataChannel 数が Offer の `data_channels` 数と一致している（全設定チャンネルが Open 済み）
- `data_channels` が空の構成では readiness を成立させない
- readiness 判定を `switched_received`、config 数、opened label 集合だけを読む private な pure helper に抽出する
- `use_datachannel_signaling` は false から true への切替状態として維持し、readiness が初めて成立したときだけ true にする
  - 本 issue は正式な初回切替前の競合を修正する
  - 切替完了後に `signaling` DataChannel が Closed になった場合の終了方針や WebSocket fallback は変更しない
- `#` で始まるユーザー定義 label は Offer の `data_channels` に含まれるため、未 Open の間は readiness を成立させない
  - 切替は全設定チャンネルの Open を待つ保守的な挙動として維持し、一部の label が未 Open の不完全な状態では DataChannel 経路へ切り替えない

### 再評価契機

- 次の両方の event 処理直後に、同じ pure helper で readiness を再評価する
  - `IncomingMessageData::Switched` を受信し、`switched_received` を true にした直後
  - `DataChannelRegister` / `DataChannelStateChange` で opened label 集合を更新した直後
- Open → switched と switched → Open のどちらの順序でも、後から成立した条件を処理した時点で DataChannel 経路へ 1 回だけ切り替える
- `on_switched` は Sora の `switched` 受信を表す既存 callback であり、切替条件の成立とは無関係に受信時に通知する
- redirect では `switched_received`、`switched_ignore_disconnect_websocket`、`use_datachannel_signaling`、opened label 集合、Offer の DataChannel config、WebSocket 切断 delay を既存どおり全て初期化する
  - redirect 前の readiness を持ち越さず、新しい Offer、Open、`switched` だけで再成立させる

### 送信経路と WebSocket 終了判定

- `SoraEvent::SignalingMessage` の送信先選択は既存の inline 分岐を変更しない
  - `use_datachannel_signaling=false` かつ WebSocket が Connected なら、`switched` の受信有無にかかわらず WebSocket へ送る
  - `use_datachannel_signaling=true` になった後だけ `signaling` DataChannel へ送る
- `switched_ignore_disconnect_websocket && use_datachannel_signaling` を、WebSocket 切断を無視できる単一の条件とする
  - `use_datachannel_signaling` は初回切替完了を保持する latch であり、切替後の DataChannel の live availability を表すものではない
- 次の既存 3 箇所へ同じ条件を適用する
  - read が EOF または `UnexpectedEof` になった場合の継続判定
  - `flush_ws_output` error の吸収判定
  - `ConnectionOutput::CloseConnection` または `ConnectionState::Closed` 後の継続判定
- SDK 側の WebSocket close delay の開始条件も、個数比較を pure helper の外に残さないため、同じ pure helper で再評価する
- readiness 成立前に WebSocket が利用不能になった場合は、DataChannel へ早期 fallback せず、既存の WebSocket 終了または error として処理する
- 通常の read error、`feed_recv_buf` error、timer error、`ConnectionEvent::Error` の既存 error policy は変更しない

## 変更対象

- `src/connection.rs`
- `e2e-tests/tests/messaging.rs`
- `CHANGES.md`

## 完了条件

- pure readiness helper について、モックやスタブを使わず次を単体テストする
  - `switched` 未受信かつ全 Open の時点は false
  - `switched` 受信済みでも一部未 Open なら false
  - `switched` 受信済みかつ全 Open で true
  - `data_channels` が空なら false
  - redirect 相当として switched、config、opened label を初期化すると false に戻る
- `use_datachannel_signaling` を true にする全箇所が pure helper を使用し、個数比較が helper の外に残っていないことをコードレビューで確認する
- `on_switched` は `switched` 受信時に従来どおり通知し、切替条件の成立まで遅延させない
- readiness 成立前の EOF / `UnexpectedEof`、WebSocket flush error、Close output / Closed state を、`switched_ignore_disconnect_websocket=true` だけを理由に吸収しない
- readiness 成立後は `switched_ignore_disconnect_websocket=true` の場合に限り、同じ 3 種の WebSocket 終了を吸収して DataChannel シグナリングを継続する
- SDK 側の WebSocket close delay は readiness 成立後にだけ開始し、readiness 成立前には WebSocket close を要求しない
- redirect 後は readiness が false であり、新しい Offer の全 DataChannel Open と新しい `switched` の両方が揃うまで再成立しない
- pure helper の単体テストを切替競合の主回帰テストとし、既存の `e2e-tests/tests/messaging.rs` は `#messaging` が Offer に共存する実 Sora 接続の補完テストとして拡張する
  - 2 クライアント目の参加で 1 クライアント目が受信する `re-offer` と、返信する `re-answer` が `on_signaling_message` で `SignalingType::DataChannel` として観測される
  - 双方向メッセージを送信する前に、両クライアントで `#messaging` の Open を明示的に待つ（全設定チャンネルの Open が切替条件であるため）
  - ユーザー定義 `#messaging` の双方向メッセージ送受信も従来どおり成功する
  - `TEST_SIGNALING_URLS` がない場合や `switched` / DataChannel signaling message を観測できない場合は、skip または成功扱いにしない
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
- `cargo test --workspace` が成功する

## 解決方法

`is_datachannel_signaling_ready` という private な pure helper を `src/connection.rs` に追加し、切替 readiness を「WebSocket 経由で `switched` を受信済み」「Offer の `data_channels` が空でない」「全設定 DataChannel が Open 済み」の 3 条件の論理積として判定した。

- `handle_datachannel_state` は `switched_received` を受け取り、個数比較をやめて helper で判定する
- `switched` 受信直後と DataChannel の Open / StateChange 直後に同じ helper で readiness を再評価する
- `use_datachannel_signaling` は初回切替成立を保持する latch とし、成立後は戻さない
- WebSocket の EOF / `UnexpectedEof`、flush error、Close / Closed state の継続判定を `switched_ignore_disconnect_websocket && use_datachannel_signaling` に変更し、readiness 成立前の切断を吸収しないようにした
- SDK 側の WebSocket close delay の開始条件も helper で再評価する
- pure helper の単体テスト 5 件を追加し、`e2e-tests/tests/messaging.rs` は `#messaging` の Open 明示待機と `re-offer` / `re-answer` の DataChannel 経路観測を追加した
