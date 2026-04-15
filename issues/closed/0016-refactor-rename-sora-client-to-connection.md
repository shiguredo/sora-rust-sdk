# SoraClient を SoraConnection にリネームする

Created: 2026-04-15
Completed: 2026-04-15
Model: Opus 4.6

## 概要

`SoraClient` という名前は、WebRTC 接続 1 本を表すオブジェクトの名前として誤解を招きやすい。
「クライアント」という語は、通常はアプリケーション全体や複数接続を束ねるオブジェクトを想起させるため、
実際のセマンティクス（1 インスタンス = 1 接続）と一致しない。

名前を `SoraConnection` に変更し、ファイル名 `src/client.rs` / `src/client_context.rs` も
それぞれ `src/connection.rs` / `src/connection_context.rs` に揃える。

## 該当箇所

- `src/client.rs`
- `src/client_context.rs`
- `src/lib.rs`
- `src/error.rs`
- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/tests.rs`
- `e2e-tests/src/lib.rs`
- `e2e-tests/tests/` 配下の全テストファイル
- `README.md`
- `docs/SORA_CPP_SDK.md`
- `CHANGES.md`

## 優先度

中

## 変更方針

### リネーム対象

| 変更前 | 変更後 |
| --- | --- |
| `src/client.rs` | `src/connection.rs` |
| `src/client_context.rs` | `src/connection_context.rs` |
| `SoraClient` | `SoraConnection` |
| `SoraClientBuilder` | `SoraConnectionBuilder` |
| `SoraClientHandle` | `SoraConnectionHandle` |
| `SoraClientCommand` | `SoraConnectionCommand` |
| `SoraClientContext` | `SoraConnectionContext` |
| `SoraClientContextConfig` | `SoraConnectionContextConfig` |

### 据え置く（変更しない）

- Sora シグナリング仕様の `client_id` フィールドとビルダーメソッド
- TLS 用語の `TlsConfig::client_cert` / `client_key`、`SoraConnectionBuilder::client_cert()`、
  `Error::ClientCertParse` / `ClientKeyParse` / `ClientCertKeyIncomplete`
- Sora シグナリング送信値の `sora_client`（`version::get_sora_client_name()`）

## 破壊的変更

公開 API の型名が変わるため、このクレートを依存している外部コードは import 修正が必要。
`CHANGES.md` に `[CHANGE]` として明記する。

## 解決方法

以下をまとめてリネームした。

- `src/client.rs` → `src/connection.rs`（`git mv`）
- `src/client_context.rs` → `src/connection_context.rs`（`git mv`）
- 公開型 `SoraClient` / `SoraClientBuilder` / `SoraClientHandle` / `SoraClientCommand` /
  `SoraClientContext` / `SoraClientContextConfig` をそれぞれ `SoraConnection...` に改名
- `src/lib.rs` の `mod` と `pub use`、`src/error.rs` の `SoraClientCommand` 参照を追従
- `examples/sumomo` と `e2e-tests` の import・型参照・変数名・関数名（`build_client_builder`
  → `build_connection_builder`、変数名 `client` → `connection`、`client_task` →
  `connection_task` など）を追従
- `README.md`・`docs/SORA_CPP_SDK.md` のコード例と説明文を追従
- `CHANGES.md` に `[CHANGE]` エントリを追加し、既存の `SoraClientBuilder` / `SoraClientContext`
  の記述も新名に更新

Sora シグナリング仕様の `client_id`、TLS 用語の `client_cert` / `client_key`、
`Error::ClientCertParse` / `ClientKeyParse` / `ClientCertKeyIncomplete`、
`version::get_sora_client_name()` は据え置いた。

検証は `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` /
`cargo test --workspace --lib --tests` をパスすることを確認した。
