# HTTP プロキシの CONNECT 応答待ちにタイムアウトを設ける

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-proxy-connect-response-timeout
- Polished: 2026-08-13

## 目的

CONNECT 応答を返さない HTTP プロキシで `SoraConnection::run` が無限にハングしないようにする。

## 現状

`connect_http_proxy_tunnel` (`src/connection.rs`) の CONNECT 応答待ちループは `stream.read` にタイムアウトがなく、`websocket_connection_timeout` の期限を一切参照しない。TCP 接続は成立するが CONNECT 応答を返さないプロキシに接続した場合、応答待ちで永久にブロックする。

さらに `connect_signaling_urls` は全 URL の失敗時に `join_set.join_next()` で全タスクの完了を待つため、1 つのタスクが応答待ちでハングすると run() 全体がハングする (URL 数に関わらない)。`SoraConnectionBuilder::websocket_connection_timeout` のドキュメント (「WebSocket 接続が確立するまでの待機時間の上限」) に反する。

## 設計方針

- `connect_websocket` が計算済みの絶対時刻 `deadline` を `connect_http_proxy_tunnel` にも渡し、CONNECT 応答待ちの read を `tokio::time::timeout_at` で囲む
  - `deadline` は `connect_tcp` / `connect_tls` と共有し、1 接続の全体 (TCP 接続・CONNECT 応答待ち・TLS 接続) の上限として扱う
  - deadline 超過時はループを抜けてエラーを返す
- タイムアウト時は既存のタイムアウト系エラーに倣い、新規エラーバリアント `Error::ProxyConnectTimeout { host, port }` を返す。`host` / `port` は接続先プロキシの値を格納する
  - `Display` メッセージは既存のタイムアウト系バリアント (`TcpConnectTimeout` 等) と同じ日本語にする (production log は英語だが、`Error` の `Display` はタイムアウト系バリアントが日本語のため)。文言は「プロキシ CONNECT 応答待ちがタイムアウトしました: {host}:{port}」等、CONNECT 応答待ちのタイムアウトであることが分かる内容にする
  - 既存の `TcpConnectTimeout` / `TlsConnectTimeout` は接続フェーズ固有の意味を持つため流用しない (CONNECT 応答待ちのタイムアウトを流用すると「TCP 接続がタイムアウトしました」等の実態と反するメッセージになる)
  - `ProxyConnectResponseMissing` は「応答を受信する前に接続が閉じられた」場合のエラーであり、タイムアウトとは別事象として区別する
- CONNECT リクエストの送信 (`stream.write_all`)、proxy host の DNS 解決、CONNECT 応答後の WebSocket Upgrade 応答待ちは本 issue のスコープ外とする
  - リクエストは小サイズで TCP 送信バッファに収まるため実用上ブロックしない。DNS 解決と Upgrade 応答待ちは別途対処が必要な問題であり、本 issue では扱わない
  - closed issue 0089 は CONNECT の送受信・WebSocket Upgrade まで含むフルハンドシェイクレース方式を検討したが不採用で closed になっており、本 issue はそのうち CONNECT 応答待ちのみを対象とする
- `connect_signaling_urls` 自体は変更しない。対象シナリオ (DNS 解決済み・TCP 接続成立) ではタイムアウト導入により各タスクが deadline 内に必ず終了するため、全タスクの完了待ちが無限に続かなくなる

## 完了条件

- CONNECT 応答を返さないプロキシに対し、`websocket_connection_timeout` 以内にエラーが返る
- タイムアウト時にプロキシの host:port を含む具体的なエラー内容がエラーメッセージに含まれる
- 正常なプロキシ経由接続の挙動が変わらない (既存の e2e テスト (proxy 経由の sendrecv) で確認する)
- CONNECT 応答を返さないプロキシでタイムアウトすることを検証するテストがある (実 TCP リスナーで accept 後に応答を返さないプロキシを再現する。モックやスタブは使わない)。テストでは `Error::ProxyConnectTimeout { host, port }` が返り、メッセージに host:port が含まれることを検証する
  - `connect_http_proxy_tunnel` は private 関数であり、公開 API 経由では `AllSignalingUrlsFailed` に包まれて返るため、`src/connection.rs` 内の `#[cfg(test)]` モジュールから `connect_http_proxy_tunnel` を実 TCP リスナーに対して直接呼ぶ単体テストとする
- `cargo test --workspace` が成功する
- CHANGES.md の develop セクションに [FIX] を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `src/error.rs`
- `CHANGES.md`
