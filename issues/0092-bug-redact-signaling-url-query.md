# シグナリング URL の query をログとエラーから除去する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-redact-signaling-url-query
- Polished: 2026-07-29

## 目的

シグナリング URL の query に認証情報が含まれる構成でも、その実値がログやエラー表示へ残らないようにする。

## 優先度根拠

High。成功時と失敗時の両方に直接の出力経路があり、ログ集約先へ機密情報が残り得る。

## 現状

シグナリング URL の parse 結果は query を含む path 表現を保持する。
query は WebSocket handshake の request-target に必要であり、接続処理から削除できない。

SDK 自身が原値を出力または保存する経路は次のとおり。

- 初回 URL 候補の接続試行ログ
- 初回 URL winner の接続成功ログ
- `SoraConnection::run` の接続先ログ
- URL 候補ごとの接続失敗ログ
- `AllSignalingUrlsFailed.errors` に保存する URL
  - `Error` は `Debug` を derive するため、`Display` だけでなく `Debug` と公開 field の参照でも原値が露出する
- redirect message の受信ログ
- redirect 先の接続先ログ

`selected_signaling_url()` と `connected_signaling_url()` は入力 URL または redirect location の原値を返す。
`on_signaling_message` も raw signaling JSON を通知するため、redirect location の原値を含む。
これらは利用者へ protocol data を返す公開 API であり、SDK の自動ログやエラーとは責任境界が異なる。

## 設計方針

### 原値と安全表示の分離

- private な signaling URL 安全表示型を追加する
- 安全表示型は raw URL や query を内部 field に保持せず、次の成分だけを所有する
  - 検証済みの `ws` / `wss` を表す値
  - userinfo を除いた正規化済み host
  - port
  - path
- URI の parse と signaling URL 固有の検証は共通の private helper で 1 回だけ行い、接続用の `SignalingTarget` と安全表示型を同じ検証済み結果から生成する
  - 接続用と表示用で URL を独立に再 parse せず、許容条件や正規化結果のずれを防ぐ
- `Display` と `Debug` の両方を安全な成分だけから生成する
- 有効な絶対 `ws://` / `wss://` URL は、`scheme://host:port/path` の形式で表示する
  - scheme の判定は URI の規則どおり大文字と小文字を区別せず、表示は小文字の `ws` / `wss` に正規化する
  - IPv6 host は `[]` で囲む
  - port はデフォルト値を含めて常に表示する
  - path が空なら `/` として表示する
  - query は `?` を含めて全体を省略し、query の key 名も残さない
- 接続用の `SignalingTarget.path` は `Uri::origin_form()` のまま query の原値を保持し、WebSocket handshake に使用する
- raw URL は次の用途だけに保持する
  - URI parse と WebSocket 接続
  - 接続成功時の `selected_signaling_url` / `connected_signaling_url`
  - redirect 後の `connected_signaling_url`

### 不正 URL の fail-closed 表示

- `Uri::parse` に失敗した URL は、入力の一部を切り出さず固定文字列 `<invalid signaling URL>` とする
- URI parse 後でも次の signaling URL 検証に失敗した場合は、同じ固定文字列とする
  - scheme の欠落または `ws` / `wss` 以外
  - host の欠落または正規化失敗
  - userinfo の存在
  - fragment の存在
  - port の不正
- 既存の `ProxyInfo` 用 `mask_url_userinfo` は、query / fragment を保持し、parse 失敗時に原文を返すため再利用しない
- 不正 URL では診断情報の保持より秘匿を優先する

### ログとエラー

- 現状に列挙した初回接続と redirect の全 URL ログは、同じ安全表示型だけを使用する
- raw URL、`SignalingTarget.path`、redirect の raw `location` を log macro へ直接渡さない
- `AllSignalingUrlsFailed.errors` へは、接続失敗を収集する時点で安全表示文字列へ変換した URL だけを保存する
  - variant の型 `Vec<(String, String)>` は維持する
  - URL 側の `String` に raw URL を保存しない
  - signaling URL の parse または検証に失敗した場合、error message 側も入力由来の scheme などを含まない固定の安全な分類へ変換する
  - 接続処理で発生した error message 側にも raw URL を別途連結しない
- redirect location の parse または検証に失敗した error も、入力由来の scheme などを `Display`、`Debug`、`source()` に残さない安全な分類で返す
- warning log、`Error::Display`、derived `Error::Debug`、公開 `errors` field は、全て同じ保存済み安全文字列を使用する
- aggregate error の `source()` に raw URL や raw URL を保持する error を追加しない
- `AllSignalingUrlsFailed.errors` の Rustdoc と `CHANGES.md` に、URL 要素は接続用の原 URL ではなく query を除去した安全表示であることを明記する

### 公開 API の互換性

- `selected_signaling_url()` と `connected_signaling_url()` は、query を含む原 URL を従来どおり返す
- `on_signaling_message` は、redirect location を含む raw signaling JSON を従来どおり通知する
- getter と callback の値を安全表示文字列へ置換しない
- 両 getter の Rustdoc と README に、返り値が query の機密情報を含み得るため、そのままログへ出力しない注意を追加する
- `SoraConnectionEventHandler::on_signaling_message` の Rustdoc と README に、raw JSON は metadata や URL query などの機密情報を含み得るため、記録する場合は利用者側で redaction が必要なことを明記する

## 変更対象

- `src/connection.rs`
- `src/error.rs`
- `src/connection_event_handler.rs`
- `README.md`
- `CHANGES.md`

## 完了条件

- 安全表示型の pure unit test で、実在しないダミー marker を使って次を確認する
  - `wss://example.com/signaling?access_token=QUERY_SENTINEL` は `wss://example.com:443/signaling`
  - `WsS://example.com/signaling?access_token=MIXED_CASE_SENTINEL` は `wss://example.com:443/signaling`
  - 明示 port と path は保持する
  - IPv6 host は `[]` で囲み、port と path を保持する
  - 空 path は `/` として表示する
  - 複数 query、percent-encoded query、query 内の `@` を `?` 以降すべて省略する
  - userinfo、fragment、unsupported scheme、host 欠落、不正 port、不正 percent encoding を含む各 URL は `<invalid signaling URL>` になる
  - 各 case の `Display` と `Debug` に、query、userinfo、fragment ごとに異なるダミー marker が含まれない
- `parse_signaling_url` の単体テストで、query を含む有効な URL の `SignalingTarget.path` が query 原値を含む `origin_form` のまま維持される
- 初回接続の試行、成功、接続先、失敗の全 log macro と、redirect の受信、接続先の全 log macro が、安全表示型だけを参照することをコードレビューで確認する
- raw URL、`SignalingTarget.path`、redirect の raw `location` を URL の log macro へ渡す箇所が残っていないことをコードレビューで確認する
- 複数の URL が全て失敗するテストで、`AllSignalingUrlsFailed` について次を確認する
  - `errors` の各 URL は安全表示文字列または固定の invalid 表示であり、raw URL ではない
  - `errors` の URL と error message のどちらにも各ダミー marker がない
  - credential 風の marker を unsupported scheme に置いた不正 URL でも、URL と error message のどちらにも marker がない
  - `format!("{}", error)` と `format!("{:?}", error)` のどちらにも各ダミー marker がない
  - `std::error::Error::source()` の chain に各ダミー marker がない
- query 付きの有効な初回 URL で接続するモックやスタブを使わない loopback テストで、次を確認する
  - WebSocket handshake の request-target には query 原値が含まれる
  - SDK の接続試行、成功、接続先表示に使う文字列には query がない
  - `selected_signaling_url()` と `connected_signaling_url()` は入力した query 付き URL の原値を返す
- 実際の `TcpListener` と `WebSocketServerConnection` を使い、最初のローカル WebSocket server が query marker を含む redirect message を返し、2 台目のローカル WebSocket server が redirect 後の接続を受ける、モックやスタブを使わない loopback テストで次を確認する
  - redirect 受信と接続先表示に使う文字列には query がない
  - redirect 先 WebSocket handshake の request-target には query 原値が含まれる
  - `on_signaling_message` は query 付き redirect JSON の原文を通知する
  - redirect 後の `connected_signaling_url()` は query 付き location の原値を返す
  - 外部 Sora server 用の環境変数がなくても実行され、skip しない
- query、userinfo、fragment、不正 percent encoding に使用する marker は全て実在しないダミー値とし、実在する credential はテスト、ログ、issue に記載しない
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
- `cargo test --workspace` が成功する
