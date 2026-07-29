# WebSocket 接続完了まで URL failover を継続する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-websocket-url-failover
- Polished: 2026-07-29

## 目的

複数のシグナリング URL を指定した場合に、WebSocket Upgrade まで成功した URL を選択し、設定した接続 timeout 内で正常な代替 URL へ failover できるようにする。

## 優先度根拠

High。TCP または TLS だけ成功する URL が先着すると、正常な代替 URL を破棄した後で Upgrade に失敗し、接続不能になる。

## 現状

`connect_signaling_urls` は TCP、proxy、TLS が完了した時点で他の接続試行を中止する。
WebSocket Upgrade は URL 選択後に `SoraConnection::run` で実行される。
このため、transport 接続が最初に完了した URL の Upgrade が失敗すると、すでに中止した正常な候補へ failover できない。

現在の timeout は `connect_websocket` 内で URL ごとに生成され、TCP と TLS にだけ適用される。
DNS、proxy CONNECT の送受信、WebSocket Upgrade の送受信は同じ deadline の対象になっていない。

redirect も transport 接続後、Upgrade 成功前に `connected_signaling_url` を更新し、その後に新しい WebSocket state machine を構築している。

## 設計方針

### 接続成功条件

- URL ごとの接続成功条件を、`shiguredo_websocket::WebSocketClientConnection` を実 I/O で駆動し、`ConnectionState::Connected` へ到達したこととする
- HTTP status 101 だけを独自に判定しない
  - Upgrade / Connection header、`Sec-WebSocket-Accept`、subprotocol、extension を含む `shiguredo_websocket` の全 handshake 検証を通過する
  - status 101 でも handshake 検証に失敗した候補は URL failure として扱う
- `ConnectionState::Connected` 前の handshake error、EOF、`ConnectionOutput::CloseConnection`、deadline 到達を URL failure とする

### 確立済み接続の所有権

- 単一 URL を完全な WebSocket 接続まで進める private helper を作る
- helper は URL の parse、DNS、proxy CONNECT、TCP、TLS、WebSocket Upgrade を行い、次を一体化した private な確立済み接続を返す
  - original URL
  - `SignalingTarget`
  - `ClientStream`
  - handshake に利用した同一の `WebSocketClientConnection`
  - 同接続の `TimerManager` と timer receiver
- HTTP 101 と同じ read に含まれた最初の WebSocket frame、未消費 event、未処理 output を state machine 内に保持したまま返す
- `SoraConnection::run` は採用した state machine をそのまま使い、新しい state machine の生成や 2 回目の Upgrade を行わない
- helper が接続判定のために `ConnectionEvent::Connected` を消費する
  - 採用後に main loop が Sora の connect message を明示的に 1 回だけ送信する
  - connect message を送って WebSocket output を flush してから、handshake と同時に届いた後続 event を処理する
- 複数候補がほぼ同時に成功しても、JoinSet から採用した bundle の URL、target、stream、state machine だけを不可分な winner とする
- winner の `ConnectionState::Connected` を確認した後にだけ残りの候補を abort する

### Deadline と failover

- 初回 URL 群への接続処理を開始する直前に、`Instant::now() + websocket_connection_timeout` で absolute deadline を 1 回だけ生成する
- 全 URL 候補を現在と同じく並列に開始し、同じ absolute deadline を共有する
- 各候補の次の全 await に同じ deadline を適用し、段階ごとに timeout を作り直さない
  - direct 接続: target の DNS、TCP、必要な TLS、Upgrade request の送信、Upgrade response の受信
  - proxy 接続: proxy host の DNS、proxy TCP、CONNECT request の送信、CONNECT response header の受信、必要な TLS、Upgrade request の送信、Upgrade response の受信
- timeout error には接続段階を含め、TCP や TLS 以外の timeout を別段階の error として偽装しない
- 1 候補の parse、DNS、TCP、proxy、TLS、Upgrade、timeout error では接続処理全体を終了せず、deadline 内で他の候補を継続する
- 全候補が失敗した場合は、Upgrade rejection、EOF、timeout を含む各 URL の最終 error を `AllSignalingUrlsFailed` に URL と対応づけて集約する

### URL の記録と redirect

- 初回接続では、完全な WebSocket handshake に成功した winner を採用した後にだけ、`selected_signaling_url` と `connected_signaling_url` を同じ original URL で更新する
- redirect は単一の `location` に対して、redirect 受信時から新しい absolute deadline を生成し、初回候補と同じ single-URL helper で DNS から完全な handshake まで実行する
- redirect 先の handshake 成功後にだけ `connected_signaling_url` を更新する
- redirect 後も `selected_signaling_url` は初回 winner のまま維持する
- redirect 先への接続に失敗した場合は初回 URL 群へ fallback せず、接続全体をその error で終了する
- 初回接続と redirect で transport と WebSocket state machine を別々に構築する重複経路を残さない

## 変更対象

- `src/connection.rs`
- `src/error.rs`
- `CHANGES.md`

## 完了条件

- transport 接続が最初に完了した候補の handshake が失敗しても、完全な handshake に成功した別候補へ接続できる
- winner の URL、target、stream、WebSocket state machine、timer が同じ bundle に由来し、winner の state が `Connected` である
- winner 選択後に Upgrade request が再送されず、Sora connect message が 1 回だけ送信される
- HTTP 101 と最初の WebSocket frame が同じ read で届いても、その frame の event が失われない
- DNS、proxy CONNECT、TCP、TLS、WebSocket Upgrade の全段階が同じ absolute deadline を超えて待機しない
- 全候補失敗時の `AllSignalingUrlsFailed` が、入力した各 URL と Upgrade / timeout を含む最終 error を保持する
- `selected_signaling_url` は初回の full-handshake winner、`connected_signaling_url` は現在の full-handshake 済み接続を指す
- redirect は新しい deadline と同じ single-URL helper を使い、成功後にだけ `connected_signaling_url` を更新する
- `127.0.0.1:0` の実 `TcpListener` と `shiguredo_websocket::WebSocketServerConnection` を使い、モックやスタブなしで次をテストする
  - bad 側は TCP を先に確立して不正な `Sec-WebSocket-Accept` を含む 101 を返し、good 側は意図的に遅れて正しい handshake を完了するため、旧 transport-winner 実装では必ず失敗する
  - good 側の 101 と最初の WebSocket frame を同じ write で送り、返却された state machine に frame event が残る
  - direct Upgrade response が無応答でも、共通 deadline と小さい scheduling 許容差の範囲で終了する
  - proxy が TCP accept 後に CONNECT response を返さない場合も同じ deadline で終了する
  - TLS server が TCP accept 後に handshake を進めない場合も同じ deadline で終了する
  - proxy CONNECT 応答までに 3 秒を使い、その後 Upgrade 応答を停止する経路へ 4 秒の timeout を設定し、接続開始から 5 秒以内に失敗する
    - 段階ごとに 4 秒の timeout を作り直す誤実装では約 7 秒かかるため、共通 deadline の回帰を検出できる
  - 複数の失敗候補について、`AllSignalingUrlsFailed` に各 URL の error が 1 件ずつ残る
- 実 `TcpListener` と `WebSocketServerConnection` で初回接続から redirect までを実行し、`SoraConnectionHandle` 経由で次を確認する
  - 初回 handshake 後は `selected_signaling_url` と `connected_signaling_url` がともに初回 winner である
  - redirect handshake 後は `selected_signaling_url` が初回 winner のまま、`connected_signaling_url` だけが redirect location になる
  - redirect 先の handshake を意図的に遅延させるテスト構造とし、成功前に `connected_signaling_url` へ代入するコードがないこともコードレビューで確認する
- DNS lookup 自体が同じ `timeout_at` の内側にあることをコードレビューで確認する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- `cargo test --workspace` が成功する
