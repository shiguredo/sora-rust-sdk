# JSON-RPC 2.0 応答を厳密に検証する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-iroiro3
- Polished: 2026-07-29

## 目的

JSON-RPC 2.0 の要件を満たさない応答を成功または正規エラーとして受理せず、protocol error として扱う。

## 優先度根拠

High。不正応答を成功 `null` として利用者へ返せるため、RPC の結果判定と request / response 対応が壊れる。

## 現状

`RpcResponse::parse` は `jsonrpc` の値、必須 `id`、`result` と `error` の排他性を検査しない。
`error` がなければ、`result` が欠落していても `null` の成功として扱う。

また、同関数の戻り値 `Result<(Option<u64>, RpcResponse)>` は、response の `id` を解析した後に別 field の validation が失敗しても `id` を失う。
そのため、protocol violation を対応する pending request へ通知することと、message 単位で接続を維持することを両立できない。

## 仕様根拠

- JSON-RPC 2.0 Specification Section 5 `Response object`
  - https://www.jsonrpc.org/specification#response_object
- JSON-RPC 2.0 Specification Section 5.1 `Error object`
  - https://www.jsonrpc.org/specification#error_object
- RFC 8259 Section 4 `Objects`
  - https://www.rfc-editor.org/rfc/rfc8259.html#section-4

仕様では Response を単一の JSON Object とし、`jsonrpc`、`id`、`result` / `error`、Error Object の各 member に要件を定めている。
RFC 8259 Section 4 は object member 名を一意にすることを SHOULD とし、重複時の挙動が実装間で予測不能になることを説明している。

## 設計方針

### Response validation

次の条件を全て満たす response だけを受理する。

- top-level は単一の Object とする
  - SDK は batch request を送信しないため、Array の batch response は対応しない
- `jsonrpc` member は必須で、String の `"2.0"` と大文字・小文字を含め完全一致する
- `id` member は必須とする
- `result` と `error` は member の存在で判定し、必ず一方だけが存在する
  - `result: null` は member が存在する正常な success response とする
  - `error: null` は Error Object ではないため protocol violation とする
- success response の `result` は任意の JSON 値を保持する
- error response の `error` は Object とし、次を検証する
  - `code` は必須で、小数点と指数表記を含まない字句上の JSON 整数かつ既存の公開 field `i32` の範囲内とする
  - `message` は必須の String とする
  - `data` は省略可能で、存在する場合は `null` を含む任意の JSON 値を保持する
- JSON-RPC が定義しない追加 member と member の順序は受理結果へ影響させない
- response Object の `jsonrpc`、`id`、`result`、`error` と Error Object の `code`、`message`、`data` は、同じ名前が重複していたら protocol violation とする
  - member 名は JSON escape を decode した後で比較し、`"id"` と `"\u0069d"` も重複として扱う
  - unknown member の重複は JSON-RPC の解釈へ影響しないため本 issue では拒否しない

`RpcResponse::Error.code` は既存の公開 API が `i32` であり、JSON-RPC 2.0 仕様の「整数」より狭い。
本 issue では公開 field の型を変更せず、`i32` 範囲外の整数と、数学的な値が整数でも小数点または指数表記を含む Number を、SDK が表現または厳密に判定しない response として拒否する。
浮動小数点数への変換で丸めて整数とみなさない。
仕様上の全整数を表現するための公開 API 変更は本 issue の対象外とし、JSON-RPC 2.0 への完全準拠とは表現しない。

### response id と pending request の相関

JSON-RPC 2.0 仕様は Request ID に String、Number、Null を許すが、SDK が生成する Request ID は常に `u64` である。
そのため、次を全て満たす `id` だけを信頼できる response id として pending request の相関に使う。

- member が存在する
- Null ではない
- String から Number への coercion を行わない
- 非負で、小数点と指数表記を含まない字句上の JSON 整数である
- `u64` の範囲内である

String、Null、負数、小数、`u64` 範囲外、欠落は JSON-RPC 一般の ID 型を全て否定するものではなく、本 SDK が生成した Request ID と相関できない値として扱う。
数学的な値が整数でも小数点または指数表記を含む Number は、SDK が送信した canonical な `u64` ID と字句上の形式が異なり、浮動小数点変換による丸めを避けるため相関に使わない。
response 全体の validation より先に `id` を独立に検査し、後続の `jsonrpc`、`result` / `error`、Error Object が不正でも信頼できる `id` を保持する。
ただし `id` member 自体が重複している場合は値が同一でも相関が曖昧なため、`trusted_id: None` とする。

### parser outcome

`RpcResponse::parse` の戻り値は `Result<(Option<u64>, RpcResponse)>` を維持する。

- `Ok((Some(id), response))`
  - 全 validation に成功し、SDK の request と相関できる `id` を持つ response
- `Ok((None, response))`
  - 全 validation に成功したが、`id` が欠落 / Null / String / 負数 / 小数 / 範囲外で
    SDK の request と相関できない response (message 単位で破棄する)
- `Err(Error::RpcProtocolViolation { id })`
  - top-level、`jsonrpc`、`result` / `error`、Error Object の validation に失敗した response
  - `id` は validation より先に独立に検査した trusted id を保持し、
    相関できる `id` を得られない場合は `None`
- `Err(Error::JsonParse(_))`
  - JSON syntax error (message 単位で破棄する)

`rpc` ラベルの UTF-8 error と JSON syntax error は、response の `id` を相関できない入力として本 issue で message 単位に破棄する。
semantic validation error は公開 `Error::RpcProtocolViolation { id: Option<u64> }` を生成する。

### pending request の状態遷移

- 正常 response に信頼できる既知 `id` がある場合
  - 一致する pending request だけを remove する
  - timeout task を abort する
  - response channel を `Ok(Some(RpcResponse))` で 1 回完了する
- 正規の remote error response
  - `Ok(Some(RpcResponse::Error { code, message, data }))` として従来どおり利用者へ返す
  - remote の `message` と `data` を失わない
- protocol violation に信頼できる既知 `id` がある場合
  - 一致する pending request だけを remove する
  - timeout task を abort する
  - response channel を `Err(Error::RpcProtocolViolation { id: Some(id) })` で 1 回完了する
  - 他の pending request を変更しない
- UTF-8 変換または JSON syntax の解析に失敗した場合
  - response を message 単位で破棄する
  - 全 pending request、timeout task、response channel を変更しない
- 信頼できる `id` がない場合
  - response を message 単位で破棄する
  - 全 pending request、timeout task、response channel を変更しない
- 信頼できるが pending map に存在しない未知、timeout 済み、重複の `id` の場合
  - response を message 単位で破棄する
  - 他の pending request を変更しない

### 公開 API と 0093 との境界

- 公開 `Error` に variant `Error::RpcProtocolViolation { id: Option<u64> }` を追加する
  - `id` は SDK が request と相関できた trusted id
  - 公開 API の破壊的変更は許容する
- 公開 `RpcResponse` には variant を追加しない
- 正規の remote error と protocol violation は、次の公開 API で区別できる
  - remote error: `Ok(Some(RpcResponse::Error { .. }))`
  - 信頼できる既知 `id` 付き protocol violation: `Err(Error::RpcProtocolViolation { id: Some(id) })`
- `SoraConnectionHandle::send_rpc_request` の Rustdoc に上記の返却契約を明記する
- issue 0093 を先に実装する
- issue 0093 は `rpc` ラベルの UTF-8 / JSON syntax error を対象外とするため、その破棄は本 issue で行う
- protocol violation 1 件で `SoraConnection::run`、DataChannel、PeerConnection を終了しない
- trusted `id` の有無や pending request の有無にかかわらず、`handle_datachannel_message` から protocol violation を main event loop の error として伝播しない
- issue 0093 で定めた raw `on_data_channel_message` の通知契約を変更しない

### 秘密情報とログ

- `Error::RpcProtocolViolation` の `Display` / `Debug` / `source()`、SDK warning へ raw response を保持または連結しない
- 不正な `jsonrpc` / `id` / `code` / `message` の実値、remote `error.message` / `data`、metadata を protocol error へ含めない
- `Error::RpcProtocolViolation` が保持するのは SDK の Request ID と相関できた trusted id のみとする
- 正規の remote error を公開 `RpcResponse::Error` として利用者へ返す既存契約は維持する
- 新たに追加する production log は英語の固定文とし、validation stage の安全な分類だけを出す

## 変更対象

- `src/rpc.rs`
- `src/connection.rs`
- `src/error.rs`

## 完了条件

- `src/rpc.rs` の table-driven unit test で次を確認する
  - 正常 success: Object / Array / String / Number / Boolean / Null の各 `result`
  - 正常 remote error: `data` 欠落、Null、Primitive、Object、Array
  - `jsonrpc`: 欠落、Null、Number、String の `"2.0"` 以外、大文字・小文字違い
  - `id`: `0` と `u64::MAX`、欠落、Null、String、負数、小数、指数表記、`u64` 範囲外
  - `result` / `error`: 両方欠落、両方存在、`result: null`、`error: null`
  - Error Object: `code` の `i32::MIN` / `i32::MAX`、Object 以外、`code` 欠落、非 Number、小数、指数表記、`i32` 範囲外、`message` 欠落、非 String
  - top-level: Null、Primitive、Array、batch response
  - 追加 member と任意の member 順序を受理する
  - response の制御 member と Error Object の制御 member の重複を拒否する
  - raw 表記が異なる escaped member 名の重複も拒否する
- parser outcome について次を確認する
  - 有効な `u64` id と不正 version の組み合わせは、`Err(Error::RpcProtocolViolation { id: Some(id) })` になる
  - id 欠落、Null、String、負数、小数、範囲外は、それ以外の validation に成功していれば `Ok((None, response))` で返り、相関できないため破棄される
  - 重複した id は 2 つの値が同一でも trusted id を確立せず、それ以外の validation に成功していれば `Ok((None, response))` になる
  - validation error の `Display`、`Debug`、`source()` に各 field のダミー marker と raw response が含まれない
- `src/connection.rs` で実際の `SoraConnectionContext`、`SoraConnection`、Tokio の oneshot channel と timeout task を使い、モックやスタブなしで次を確認する
  - 既知 id の正常 success は対応する pending request だけを `Ok(Some(RpcResponse::Success))` で完了する
  - 既知 id の正常 remote error は対応する pending request だけを `Ok(Some(RpcResponse::Error))` で完了し、`message` / `data` を維持する
  - 既知 id 付き protocol violation は対応する pending request だけを `Err(Error::RpcProtocolViolation { id: Some(id) })` で完了する
  - 上記 3 case は pending を remove し、timeout task を abort し、response channel を 1 回だけ完了する
  - 信頼できる id がない response は全 pending と timeout task を維持する
  - 未知 id、timeout 済み id、同じ id の重複 response は他の pending request を変更しない
  - protocol violation の後も同じ DataChannel の正常 response と別 DataChannel の正常 message を処理できる
  - `handle_datachannel_message` は protocol violation について `Ok(())` を返し、main event loop の終了原因にしない
  - `rpc` ラベルの UTF-8 error と JSON syntax error は、全 pending と timeout task を維持し、`handle_datachannel_message` が `Ok(())` を返す
- `Error::RpcProtocolViolation` と新たな warning の `Display` / `Debug` / `source()` に raw response、実在する credential、metadata を含めない
- 仕様由来の validation コードコメントに JSON-RPC 2.0 Specification Section 5 / 5.1 を記載する
- 重複 member を検出するコードコメントには RFC 8259 Section 4 も記載する
- 上記の各仕様コメントに、仕様が将来変更される可能性を明記する
- テスト用の公開 API を追加しない
- production log は英語、コメントとテストの assertion message は日本語にする
- `cargo test --workspace` が成功する
