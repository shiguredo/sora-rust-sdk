# proptest のテストを pbt ワークスペースクレートに分離する

- Priority: Medium
- Created: 2026-06-12
- Polished: 2026-06-12
- Completed: 2026-06-12
- Model: Opus 4.7
- Branch: feature/refactor-extract-proptest-to-pbt-crate

## 目的

CLAUDE.md で PBT は `pbt/tests/prop_<module>.rs` に配置するルールになっているが、現状は `src/connection.rs` の `#[cfg(test)] mod tests` 内に `proptest!` ブロックがインラインで埋め込まれている。ルールに合わせて分離し、今後 PBT を増やしていく際の置き場所を明確にする。あわせて `Makefile:12-13` の `pbt-with-cover` ターゲットが対応する `pbt` クレート不在のまま存在している既存の不整合も解消する。

## 優先度根拠

実利用には影響しないリファクタリングだが、CLAUDE.md のテスト配置ルールから外れた状態が他モジュールにも波及しないうちに整理しておきたい。また、今後 PBT 化候補となるモジュール (例: ラウンドトリップ可能な変換ロジックを持つ `src/zlib.rs` や `src/signaling_types.rs`) が存在しうるため、置き場所の確立を先延ばしする理由は薄い。一方で欠陥や機能要求ではないため最優先ではない。よって Medium とする。

## 現状

- `src/connection.rs:2569` の `#[cfg(test)] mod tests` 内に `proptest!` ブロックがあり、5 件のプロパティが入っている。
  - 移動対象 4 件: `parse_proxy_url_accepts_http` / `parse_proxy_url_rejects_https` / `parse_proxy_url_rejects_socks` / `parse_proxy_url_rejects_userinfo`
  - 削除対象 1 件: `url_getters_roundtrip_command_response`
- `mod tests` には以下のヘルパが定義されている。
  - `proxy_info_with_url` (`src/connection.rs:2488-2493`): 移動対象 4 件と、移動しない通常 `#[test]` の `parse_proxy_info_*` 2 件 (`:2664`, `:2671`) で使用。
  - `block_on_test` (`src/connection.rs:2495-2501`): `url_getters_roundtrip_command_response` でのみ使用。あわせて `use std::future::Future;` (`src/connection.rs:2486`) もこのヘルパのために存在する。
  - `is_turn_tcp_or_udp_url` (`src/connection.rs:2503-2521`): `ice_server_url_configurer_*` で使用。
- `mod tests` には `ParsedProxyInfo` をフィールド名指定で直接構築する単体テスト `build_proxy_connect_request_omits_proxy_auth_when_credentials_absent` (`src/connection.rs:2682-2702`) と `build_proxy_connect_request_includes_proxy_auth_for_explicit_empty_credentials` (`src/connection.rs:2704-2725`) も存在する (移動対象外)。
- `ParsedProxyInfo` (`src/connection.rs:1929-1979`) は crate-private な構造体で、フィールドはすべて非公開、`ParsedProxyInfo::parse` も非公開。`src/lib.rs:2` の `mod connection;` も private mod のため、`pub` 化だけでは外部 crate から参照できない。
- `SoraConnectionHandle.command_tx` フィールドは crate-private のため、外部 crate から `SoraConnectionHandle { command_tx }` の形で再構築することはできない。なお `mod tests` 内の `url_getters_return_send_error_after_run_loop_stops` (`src/connection.rs:2727-2756`) も同じ構築パターンを使うが、こちらは同一 crate 内のため引き続き動作する。
- ワークスペースは `e2e-tests` と `examples/sumomo` の 2 メンバーで構成されている。`Makefile:12-13` には `pbt-with-cover: cargo llvm-cov -p pbt --tests` が定義されているが、対応する `pbt` クレートは存在しない。
- ルート `Cargo.toml` の `[workspace.dependencies]` に `proptest = "1.11"` が宣言され、`[dev-dependencies]` で `proptest.workspace = true` を参照している。実際の参照箇所は `src/connection.rs` の `mod tests` のみ。
- CHANGES.md の `## develop` 内には既に `### misc` サブセクション (`CHANGES.md:134` 以降) が存在する。
- crate 内の既存公開構造体 (`TlsConfig` / `ProxyInfo` 等) はいずれも `#[non_exhaustive]` を付与しておらず、フィールド追加時は `[CHANGE]` として破壊的変更を明示する方針で運用されている。
- 兄弟リポジトリの `moqt-rs/pbt` は `Cargo.toml` と `tests/` のみで構成され、`src/lib.rs` は持たない。

## 該当箇所

- `Cargo.toml` (ルート): `[workspace] members` と `[dev-dependencies]` を編集
- `Cargo.lock` (ルート): `pbt` パッケージのエントリ追加に伴って差分が必ず発生するためコミットに含める
- `src/connection.rs`: `mod tests` 内の `proptest!` ブロック、`use proptest::prelude::*;`、`use std::future::Future;`、`block_on_test` ヘルパを削除、`ParsedProxyInfo` 定義・実装の可視性変更
- `src/lib.rs`: `pub use crate::connection::ParsedProxyInfo;` を追加
- `pbt/Cargo.toml` (新規)
- `pbt/tests/prop_connection.rs` (新規)
- `CHANGES.md`: `## develop` 内の既存 `### misc` サブセクションに追記

## 設計方針

### ワークスペース構成

- ルート `Cargo.toml:13-14` の以下を変更する。
  - Before: `members = ["e2e-tests", "examples/sumomo"]`
  - After: `members = ["e2e-tests", "examples/sumomo", "pbt"]`
- `pbt/Cargo.toml` を新規作成する。`pbt` はテスト専用のため、兄弟リポジトリ `moqt-rs/pbt` と同様 `[package]` と `[dev-dependencies]` のみで構成し、`pbt/src/lib.rs` は作らない。Cargo は `src/lib.rs` / `src/main.rs` が無くても `tests/` 配下の integration test を実行できる。

  ```toml
  [package]
  name = "pbt"
  version = "0.0.0"
  edition = "2024"
  rust-version = "1.88"
  publish = false

  [dev-dependencies]
  sora_sdk.workspace = true
  proptest.workspace = true
  ```

  - `[package]` の `rust-version` と `publish = false` は `e2e-tests/Cargo.toml:1-6` の流儀に揃える。
  - 依存はすべて `[dev-dependencies]` に置く (PBT は integration test のみで利用するため)。
- `pbt/tests/prop_connection.rs` に PBT 本体を置く。

### 移動するテスト

- `parse_proxy_url_accepts_http` / `parse_proxy_url_rejects_https` / `parse_proxy_url_rejects_socks` / `parse_proxy_url_rejects_userinfo` の 4 件を `pbt/tests/prop_connection.rs` に移動する。
- 4 件は 1 つの `proptest!` ブロックにまとめる (現状の構成を踏襲)。
- 本 issue では PBT の strategy (`label`、`port`、`scheme` の値域) や検証内容は変更しない。
- 関数名は現状の英語識別子のまま移動する。
- 移動後の `Error` バリアント判定は、`Error` が `PartialEq` を派生していないため、フィールド無しバリアント (`ProxyUrlUserinfoNotSupported`) は `matches!`、フィールドありバリアント (`ProxyUrlUnsupportedScheme { .. }`) は `match { ... _ => prop_assert!(false) }` で書く (現状コードの選択をそのまま維持する)。
- `proxy_info_with_url` ヘルパは PBT 側にも独立して定義する (`src/connection.rs` 側の `mod tests` でも `parse_proxy_info_*` で引き続き使うため、両方に同名関数が並存する)。
- `pbt/tests/prop_connection.rs` の冒頭は以下の import で始める。`Result` 型は直接書かないため import 不要、`tokio` 系も削除対象テストでしか使っていなかったため不要。

  ```rust
  use proptest::prelude::*;
  use sora_sdk::{Error, ParsedProxyInfo, ProxyInfo};
  ```

### 削除するテストとヘルパ

- `url_getters_roundtrip_command_response` (`src/connection.rs:2570-2612`) を削除する。
- 削除理由: このテストは `SoraConnectionHandle { command_tx }` の形で `SoraConnectionHandle` を直接構築している。`SoraConnectionHandle.command_tx` フィールドは crate-private のため外部 crate に PBT を移すと再構築できない。テストのためだけにフィールドを `pub` 化することは公開 API を歪めるので採用しない。`selected_signaling_url` / `connected_signaling_url` は薄いラッパで、削除に伴う正常系のカバレッジ低下は許容する (エラーパス側は `url_getters_return_send_error_after_run_loop_stops` で別途カバー済み)。
- `block_on_test` ヘルパ (`src/connection.rs:2495-2501`) も `url_getters_roundtrip_command_response` 削除に伴い未使用となるため削除する。
- `use std::future::Future;` (`src/connection.rs:2486`) も `block_on_test` のためにのみ存在するため削除する。
- `is_turn_tcp_or_udp_url` ヘルパ (`src/connection.rs:2503-2521`) は `ice_server_url_configurer_*` で引き続き使われるため残す。

### 可視性調整

- `ParsedProxyInfo` を `pub struct` にする。
- `#[derive(Debug, Clone)]` は据え置く。`PartialEq` / `Eq` は PBT でフィールド単位の `prop_assert_eq!` を使うため不要。`Default` は全フィールドが意味を持ち、デフォルト値 (`host: ""`、`port: 0`) を仮定するのが安全でないため派生しない。
- `#[non_exhaustive]` は付けない。crate 内の他公開型 (`Error` / `TlsConfig` / `ProxyInfo` 等) のいずれも未付与で、フィールド追加時は CLAUDE.md「良い設計のためには破壊的変更を積極的に行うこと」に従い `[CHANGE]` として CHANGES.md に明示する方針を踏襲する。
- `ParsedProxyInfo::parse` を `pub fn` にする。シグネチャ (`fn parse(proxy: &ProxyInfo) -> Result<ParsedProxyInfo>`) は変更しない。`Result` は `crate::error::Result` (= `std::result::Result<T, Error>`) で、外部からは `sora_sdk::Result<sora_sdk::ParsedProxyInfo>` として見える。
- フィールドのうち PBT がアサーション対象にしている `host: String` と `port: u16` を `pub` にする。`username` / `password` / `user_agent` は PBT 対象ではないため非公開のまま残す。同一 crate 内の `build_proxy_connect_request_*` テストは引き続きフィールド名指定の構造体リテラル構築で全フィールドを書ける (非公開フィールドも crate 内なら見えるため)。
- `src/lib.rs:17-19` の既存 `pub use crate::connection::{ ... };` の同一ブレース内に `ParsedProxyInfo` を **アルファベット順で先頭** に追加する (`pub use crate::connection::{ParsedProxyInfo, SoraConnection, ...};`)。
- `ParsedProxyInfo` と `ParsedProxyInfo::parse` には日本語の doc コメントを付与する。以下を最低限の文例とする。

  ```rust
  /// `ProxyInfo` を解析し、HTTP プロキシ接続に必要な情報に正規化した結果。
  ///
  /// PBT 等の検証目的を主用途として公開している型のため、通常の利用者がこの型を
  /// 直接構築する必要はなく、`ParsedProxyInfo::parse` 経由で取得する。
  ```

  ```rust
  /// `ProxyInfo` を解析し、検証済みのプロキシ接続情報を返す。
  ///
  /// 受理するのは `http://host[:port]` 形式のみで、`https://` / `socks*://` や
  /// userinfo / fragment / query / 非空パスを含む URL は拒否する。
  ```

- `Error` / `ProxyInfo` は既に公開済みのため変更不要。

### ルート Cargo.toml の編集

- `[dev-dependencies]` セクション (`Cargo.toml:106-107` の 2 行) を削除する。`proptest.workspace = true` を除いた結果セクションが空になるため、見出しごと削除する。
- `[workspace.dependencies]` の `proptest = "1.11"` 行とコメントは現状のまま維持する。

### CHANGES.md への追記

- 既存の `### misc` サブセクション (`CHANGES.md:134` 以降) の末尾に、CLAUDE.md「エントリは種別の順番を守って記載すること (CHANGE → ADD → UPDATE → FIX の順)」に従い `[ADD]` → `[UPDATE]` の順で以下を追加する。
  - `[ADD] ParsedProxyInfo と ParsedProxyInfo::parse を公開する`
  - `[UPDATE] proptest のテストを pbt ワークスペースクレートに分離する`
- 各エントリは担当者行を続ける (`  - @<ユーザー名>`)。

### スコープ外

以下は本 issue の対象外として、必要があれば別 issue で扱う。

- `src/connection.rs` 内の通常 `#[test]` を `tests/test_connection.rs` に移す作業。
- `fuzz/` ディレクトリ新設と fuzzing ターゲット整備。
- 既存 PBT 内の `prop_assert!(false)` パターン改善や strategy の表現力強化。
- `Error` enum への `PartialEq` 派生。
- `make pbt-with-cover` (`cargo llvm-cov -p pbt --tests`) のカバレッジ計測対象の妥当性。本 issue では `pbt` クレートの test バイナリ実行が成功するところまでを担保し、`sora_sdk` 本体のカバレッジを意味のある形で取得する設計 (例: `-p sora_sdk` への切り替え) は別途検討する。

## 完了条件

### 成果物

- `pbt/Cargo.toml` と `pbt/tests/prop_connection.rs` が存在する (`pbt/src/lib.rs` は作らない)。
- `pbt/tests/prop_connection.rs` で 4 件の PBT (`parse_proxy_url_accepts_http` / `rejects_https` / `rejects_socks` / `rejects_userinfo`) が実行される。
- `src/connection.rs` から `proptest!` ブロック、`use proptest::prelude::*;`、`use std::future::Future;`、`url_getters_roundtrip_command_response`、`block_on_test` ヘルパが消えている。
- `ParsedProxyInfo` 型と `ParsedProxyInfo::parse` 関数が `pub` で、`src/lib.rs` の既存 `pub use crate::connection::{ ... };` 経由でアルファベット順に再エクスポートされている。`host` / `port` フィールドが `pub`、日本語の doc コメントが付与されている。`#[non_exhaustive]` は付与しない。
- ルート `Cargo.toml` の `[dev-dependencies]` セクションが見出しごと削除されている。`[workspace.dependencies]` の `proptest = "1.11"` は維持されている。
- CHANGES.md の既存 `### misc` サブセクション末尾に `[ADD] ParsedProxyInfo と ParsedProxyInfo::parse を公開する` と `[UPDATE] proptest のテストを pbt ワークスペースクレートに分離する` の 2 エントリ (各々担当者行付き) が、`[ADD]` → `[UPDATE]` の順で追加されている。

### 検証コマンド

- `cargo fmt --all -- --check` が通る。
- `cargo clippy --workspace --all-targets -- -D warnings` が通る。
- `cargo test --workspace` が通る (移動した 4 件の PBT および `src/connection.rs` に残る通常 `#[test]` が全て成功する)。
- `make pbt-with-cover` が exit 0 で終了する (`cargo-llvm-cov` 未導入の場合は事前に `cargo install cargo-llvm-cov` を行う)。

## 解決方法

- `pbt` ワークスペースクレート (`pbt/Cargo.toml` と `pbt/tests/prop_connection.rs`) を新規追加し、ルート `Cargo.toml` の `[workspace] members` に `pbt` を追加した。
- `parse_proxy_url_accepts_http` / `parse_proxy_url_rejects_https` / `parse_proxy_url_rejects_socks` / `parse_proxy_url_rejects_userinfo` の PBT 4 件を `pbt/tests/prop_connection.rs` に移動した。
- `url_getters_roundtrip_command_response` PBT、`block_on_test` ヘルパ、`use std::future::Future;` を `src/connection.rs` から削除した。
- `ParsedProxyInfo` 構造体と `ParsedProxyInfo::parse` 関数を `pub` 化し、`host` / `port` フィールドを `pub` にして日本語の doc コメントを付与した。
- `src/lib.rs` の `pub use crate::connection::{ ... };` ブレースに `ParsedProxyInfo` をアルファベット順で先頭に追加した。
- ルート `Cargo.toml` の `[dev-dependencies]` セクションを見出しごと削除した。`[workspace.dependencies]` の `proptest = "1.11"` は維持した。
- `Makefile:1` の `.PHONY` 行を整理し、実在しないターゲット `pbt` / `pbt-cover` / `fuzz` を削除して `pbt-with-cover` に置き換えた (既存の不整合解消)。
- `CHANGES.md` の既存 `### misc` サブセクション末尾に `[ADD] ParsedProxyInfo と ParsedProxyInfo::parse を公開する` と `[UPDATE] proptest のテストを pbt ワークスペースクレートに分離する` の 2 エントリを `[ADD]` → `[UPDATE]` の順で追加した。
- 検証コマンド (`cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` / `make pbt-with-cover`) がいずれも通過することを確認した。
