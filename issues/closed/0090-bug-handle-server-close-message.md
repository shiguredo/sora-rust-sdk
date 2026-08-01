# DataChannel シグナリングの Close メッセージで接続を終了する

- Priority: High
- Created: 2026-07-29
- Completed: 2026-08-01
- Model: GPT-5
- Branch: feature/fix-server-close-message
- Polished: 2026-07-29

## 目的

Sora から DataChannel の `signaling` label で正式な `{"type":"close"}` メッセージを受信したときに、接続の終了処理へ移行し、`SoraConnection::run` を正常終了させる。

## 優先度根拠

High。Sora 側の `data_channel_signaling_close_message=true` とクライアント側の `data_channel_signaling=true`、`ignore_disconnect_websocket=true` を組み合わせた構成では、WebSocket の切断後も DataChannel シグナリングを継続する。
この状態で Close メッセージを無視すると、Sora が接続を終了させても `run` task が残留する。

Sora 側の `data_channel_signaling_close_message` は opt-in で、デフォルトは `false` である。

## 現状

`src/signaling_types.rs` は Close メッセージを `IncomingMessageData::Close {}` として parse し、仕様上必須の `code` と `reason` を検証せず破棄する。

`SoraConnection::handle_datachannel_message` は `Result<()>` しか返さず、DataChannel 経由の Close を unsupported message として扱う。
このため、メインの接続 loop へ終了要求を伝えられない。

WebSocket の text message として `IncomingMessageData::Close` を受信した場合も、現在の `break` は内側の WebSocket event poll loop だけを抜け、接続 loop は継続する。
ただし、これは公式の Close メッセージ経路ではない。
Sora が WebSocket シグナリング利用中に切断を通知する正式な経路は、JSON の Close メッセージではなく WebSocket protocol の Close フレームである。
非標準の WebSocket text Close への防御的対応は、本 issue の対象外とする。

次の 3 つは別の事象であり、コールバックや終了条件を混同しない。

- DataChannel の `signaling` label で受信する JSON の `{"type":"close"}` は、接続全体を終了させる server signaling message
- `ConnectionEvent::Close` は WebSocket transport の Close フレームであり、既存の `on_websocket_close` と `ignore_disconnect_websocket` の規則に従う
- `DataChannelStateChange` の Closed は label ごとの transport lifecycle event であり、既存の `on_data_channel_close` に通知する

## 設計方針

### Close メッセージの parse

- `IncomingMessageData::Close` に `code: u16` と `reason: String` を保持する
- `code` と `reason` は `required()` で取得し、欠落、型違い、負数、`u16` の範囲外を parse error にする
- Close の parse には、Sora ドキュメント「シグナリングの型定義」の `SignalingCloseMessage` を根拠とするコードコメントを付ける
- `1000` と `4490` の両方を有効な Close code として扱う
  - `1000` は正常切断、`4490` は Sora 側で異常が発生した切断を表す
  - どちらも Sora が接続終了を確定した通知であるため、Close メッセージを正常に処理した `run` の戻り値は `Ok(())` とする
- raw message は既存の `on_data_channel_message` と `on_signaling_message(SignalingType::DataChannel, SignalingDirection::Received, ...)` へ従来どおり 1 回ずつ通知する
- Close 専用の公開 callback は追加しない
  - `code` と `reason` は raw signaling message で観測できる
  - `on_websocket_close` を server signaling Close の通知に流用しない

### 接続 loop への終了伝播

- `handle_datachannel_message` の戻り値を、通常継続と server Close を区別できる private enum に変更する
  - `Continue`
  - `ServerClose { code, reason }`
- `ServerClose` を返すのは、DataChannel の `signaling` label で有効な Close メッセージを受信した場合だけとする
  - `stats`、`push`、`notify` label に同じ JSON が届いても接続全体を終了させず、既存どおり unsupported message として扱う
- DataChannel の Close だけを正式な terminal message とする判定には、Sora ドキュメント「Sora クライアント要求仕様」の「DataChannel シグナリングのみ利用時に Sora から切断が発生した際の `"type": "close"` メッセージの送信」を根拠とするコードコメントを付ける
- メインの接続 loop は `ServerClose` を受け取った同じ iteration で label 付き `break` により終了処理へ移行し、それ以降の redirect、WebSocket flush、command、signaling event を処理しない
- WebSocket text message で parse 済みの `IncomingMessageData::Close` を外側の接続 loop へ伝播させる変更は行わない
- WebSocket protocol の Close フレームと `ignore_disconnect_websocket` の既存挙動は変更しない

### 終了処理とコールバック

- 有効な server Close の受信後は、新しい signaling message や command を受け付けず、既存の共通終了処理を 1 回だけ実行する
- Sora は Close メッセージの送信後に DataChannel を閉じるため、SDK 側から `DataChannel::close()` は呼ばない
- 既存の `disconnect_wait_timeout` を共通 deadline として、Sora 側からの Closed event を待つ
- `on_data_channel_close` は `opened_datachannels.remove(label)` が成功したときだけ呼び、各 open 済み label について最大 1 回にする
  - timeout した label に対して、Closed を観測していない synthetic callback は呼ばない
- WebSocket がすでに閉じている場合は、WebSocket の終了処理や `on_websocket_close` を重ねて実行しない
- WebSocket が接続中の場合は、DataChannel の終了待機後に既存の WebSocket close handshake を実行する
  - server Close はすでに terminal event として確定しているため、`ws.close()` の state-machine error、close timeout、`flush_ws_output`、read、`feed_recv_buf` の error は warning として記録し、`run` の `Ok(())` を覆さない
  - ユーザーからの disconnect、WebSocket Close フレーム、TCP EOF など、server Close 以外の既存終了経路の error policy は変更しない
- pending RPC の error 契約は変更しない

## 変更対象

- `src/signaling_types.rs`
- `src/connection.rs`
- `e2e-tests/Cargo.toml`
- `e2e-tests/src/lib.rs`
- `e2e-tests/src/test_connection.rs`
- `e2e-tests/tests/server_close_message.rs`
- `Cargo.lock`
- `CHANGES.md`

## 完了条件

- `IncomingMessageData::Close` が必須の `code: u16` と `reason: String` を保持する
- Close メッセージの parse について、モックやスタブを使わず次を単体テストする
  - `code=1000` と `code=4490` を受理し、`reason` とともに保持する
  - `code` または `reason` の欠落を拒否する
  - `code` の文字列、小数、負数、`65536`、`reason` の非文字列を拒否する
- DataChannel message 処理について、`signaling` label の有効な Close だけが `ServerClose` を返し、`stats`、`push`、`notify` label では返さない
- WebSocket がすでに閉じた状態で DataChannel 経由の有効な Close を受信すると、ユーザーから `disconnect()` を呼ばなくても `run` が `disconnect_wait_timeout + 1 秒` 以内に `Ok(())` を返す
- WebSocket が接続中の状態で DataChannel 経由の有効な Close を受信しても、`run` が `disconnect_wait_timeout + websocket_close_timeout + 1 秒` 以内に `Ok(())` を返す
- WebSocket protocol の Close フレームだけを受信した場合は、既存の `on_websocket_close` と `ignore_disconnect_websocket` の挙動が変わらない
- `on_data_channel_close` を label ごとに最大 1 回だけ呼ぶ
- server Close を `on_websocket_close` として通知しない
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- Sora の実接続 E2E は次の前提を必須とし、環境不足や Close メッセージ未受信を skip または成功扱いにしない
  - Sora 側で `data_channel_signaling_close_message=true`
  - Sora 側で `signaling_normal_close_reason=true`
  - `TEST_SIGNALING_URLS` と `TEST_API_URL` が設定済み
  - クライアントは `data_channel_signaling=true`、`ignore_disconnect_websocket=true` で接続する
- 実接続 E2E はテストごとに一意な channel ID で接続し、`TEST_API_URL` へ実 HTTP 接続して DisconnectChannel API を実行する
  - WebSocket 接続中のケースでは、`switched` を確認した直後、SDK の `WS_DISCONNECT_DELAY` が経過する前に API を実行する
  - WebSocket 切断済みのケースでは、`switched` と WebSocket Close フレームを確認した後に API を実行する
  - `POST TEST_API_URL`
  - URL に明示された host、port、path、query を保持して request target と `Host` header を構築する
  - request header は `x-sora-target: Sora_20151104.DisconnectChannel`
  - request header は `Content-Type: application/json`
  - JSON request body は `{"channel_id":"<対象の channel ID>"}`
  - `TEST_API_URL` の `http://` と `https://` の両方を扱う
  - HTTP は Tokio の実 `TcpStream`、HTTPS は `rustls`、`rustls-platform-verifier`、`tokio-rustls` による実 TLS stream を使う
  - HTTPS は URL の host から `ServerName` を構築し、`ClientConfig::with_platform_verifier()` による証明書と hostname の検証を必須とする
  - 証明書または hostname の検証に失敗した場合は、接続を継続せずテストを失敗させる
  - `shiguredo_http11` の request encoder / response decoder を使い、外部の `curl` process、モック、スタブは使わない
  - `e2e-tests/Cargo.toml` に `rustls.workspace = true`、`rustls-platform-verifier.workspace = true`、`tokio-rustls.workspace = true` を直接依存として追加する
  - `e2e-tests/Cargo.toml` の `shiguredo_http11` の用途コメントに、DisconnectChannel API E2E でも利用することを反映する
  - connect、request write、response header / body read の全体へ 5 秒の共通 timeout を適用する
  - HTTP response が 2xx でない場合、response の decode に失敗した場合、または response body の `channel_id` が要求値と一致しない場合は、その場でテストを失敗させる
  - API response の成功を確認してから Close メッセージの受信待ちへ進む
  - クライアントから `disconnect()` は呼ばない
- 実接続 E2E で次を確認する
  - DataChannel の `signaling` label から `code=1000`、`reason="DISCONNECTED-API"` の Close メッセージを 1 回受信する
  - Close の raw message が `on_data_channel_message` と `on_signaling_message(DataChannel, Received, ...)` に各 1 回通知される
  - `run` task が timeout 内に panic や `Err` ではなく `Ok(())` で終了する
  - WebSocket 接続中と切断済みの両ケースで同じ Close message と正常終了を確認する
  - WebSocket 接続中のケースでは、API request 開始前と Close message の受信時点の両方で `on_websocket_close` が未通知であることを確認する
  - Close 受信前に open を観測した各 DataChannel label について、`on_data_channel_close` が重複しない
  - server Close によって `on_websocket_close` の回数が増えない
- production log は英語、コメントとテストの assertion message は日本語にする
- `cargo test --workspace` が成功する

## 解決方法

- `IncomingMessageData::Close` に必須の `code: u16` と `reason: String` を保持し、欠落・型違い・範囲外を parse error にする
- `handle_datachannel_message` の戻り値を `HandleDatachannelMessageResult` に変更し、`signaling` label の Close だけを `ServerClose` として扱う
- メインの接続 loop は `ServerClose` を受信した同一 iteration で label 付き break により終了処理へ移行する
- DataChannel の終了待機後に、WebSocket が接続中であれば close handshake を実行する。server Close は terminal event として確定しているため、後始末の error は warning として記録し `run` の `Ok(())` を覆さない
- `on_data_channel_close` は `opened_datachannels.remove(label)` が成功したときだけ呼び、server Close を `on_websocket_close` として通知しない
- 実接続 E2E は DisconnectChannel API を `TEST_API_URL` へ実 HTTP 接続で実行して、WebSocket 接続中と切断済みの両ケースで Close message 受信と `run` の正常終了を確認する
