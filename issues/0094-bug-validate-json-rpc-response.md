# JSON-RPC 2.0 応答を厳密に検証する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-validate-json-rpc-response
- Polished: {YYYY-MM-DD}

## 目的

JSON-RPC 2.0 の要件を満たさない応答を成功または正規エラーとして受理せず、protocol error として扱う。

## 優先度根拠

High。不正応答を成功 `null` として利用者へ返せるため、RPC の結果判定と request / response 対応が壊れる。

## 現状

`RpcResponse::parse` は `jsonrpc` の値、必須 `id`、`result` と `error` の排他性を検査しない。
`error` がなければ、`result` が欠落していても `null` の成功として扱う。

## 設計方針

- `jsonrpc` が `"2.0"` であることを検証する
- response では `id` を必須にする
- `result` と `error` のいずれか一方だけが存在することを検証する
- protocol error と remote RPC error を区別する

## 完了条件

- 正常な success response と error response を parse できる
- version、id、result / error が不正な応答を拒否する
- response id と pending request の対応が維持される
- JSON-RPC の境界ケースを網羅する単体テストがある
