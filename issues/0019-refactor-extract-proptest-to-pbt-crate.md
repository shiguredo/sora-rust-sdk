# proptest のテストを pbt ワークスペースクレートに分離する

- Priority: Medium
- Created: 2026-06-12
- Model: Opus 4.7
- Branch: feature/refactor-extract-proptest-to-pbt-crate

## 目的

CLAUDE.md で PBT は `pbt/tests/prop_<module>.rs` に配置するルールになっているが、現状は `src/connection.rs` の `#[cfg(test)] mod tests` 内に `proptest!` ブロックがインラインで埋め込まれている。ルールに合わせて分離し、今後 PBT を増やしていく際の置き場所を明確にする。

## 優先度根拠

実利用には影響しないリファクタリングだが、CLAUDE.md のテスト配置ルール (`pbt/tests/prop_<module>.rs` / `tests/test_<module>.rs` の役割分担) から外れた状態が他モジュールにも波及しないうちに整理しておきたい。また、今後 PBT を増やしていく際に置き場所が確立されていないと判断コストが毎回発生するため、先延ばしする理由は薄い。一方で、欠陥や機能要求ではないため最優先ではない。よって Medium とする。

## 現状

- `src/connection.rs:2482` の `#[cfg(test)] mod tests` 内に `proptest!` ブロックが 1 つあり、以下 5 件のプロパティが入っている。
  - `url_getters_roundtrip_command_response` (2570)
  - `parse_proxy_url_accepts_http` (2614)
  - `parse_proxy_url_rejects_https` (2625)
  - `parse_proxy_url_rejects_socks` (2638)
  - `parse_proxy_url_rejects_userinfo` (2652)
- 同じ `mod tests` 内に、通常の `#[test]` (`ice_server_url_configurer_*`、`parse_proxy_info_*` など) が 10 件以上同居している。
- ワークスペースは `e2e-tests` と `examples/sumomo` の 2 メンバーで構成されており、PBT 用のメンバーは存在しない。
- ルート `Cargo.toml` の `[workspace.dependencies]` および `[dev-dependencies]` に `proptest = "1.11"` が宣言されている。
- `proptest` を実際に参照しているのは現状 `src/connection.rs` のみ。

## 設計方針

- ワークスペースに新規メンバー `pbt` を追加する (`e2e-tests` と同じパターン)。
  - `pbt/Cargo.toml` の `[dev-dependencies]` に `sora_sdk.workspace = true` と `proptest.workspace = true` を宣言する。
  - `pbt/src/lib.rs` は空のままでよい (テストの置き場所として crate を成立させるためだけのもの)。
  - PBT 本体は `pbt/tests/prop_connection.rs` に置く。
- `src/connection.rs` 側の `proptest!` ブロックは削除する。`use proptest::prelude::*;` も合わせて除去する。
- `url_getters_roundtrip_command_response` は mpsc で `Option<String>` を往復させているだけで PBT としての価値が薄いため、移動せず削除する。これにより `SoraConnectionCommand` / `SoraConnectionHandle` の内部構造を crate 外に露出させずに済む。
- 残る 4 件 (`parse_proxy_url_accepts_http` / `rejects_https` / `rejects_socks` / `rejects_userinfo`) を `pbt/tests/prop_connection.rs` に移動する。
- 移動に必要な可視性調整は最小限に留める。
  - `ParsedProxyInfo` 型と `ParsedProxyInfo::parse` 関数を `pub` にする。
  - `Error::ProxyUrlUnsupportedScheme` / `Error::ProxyUrlUserinfoNotSupported` バリアントが crate 外から `match` できる状態を確認する。
  - `proxy_info_with_url` ヘルパは PBT 側で再定義する (公開 API ではない)。
- ルート `Cargo.toml` の `[dev-dependencies]` から `proptest.workspace = true` を除去する (本体 crate からは使わなくなるため)。`[workspace.dependencies]` 側の `proptest = "1.11"` は新メンバーから参照するため維持する。

## 完了条件

- `pbt` ワークスペースメンバーが追加され、`pbt/tests/prop_connection.rs` で 4 件の PBT が実行される。
- `src/connection.rs` から `proptest!` ブロックと `use proptest::prelude::*;` が消えている。
- `src/connection.rs` 内の通常の単体テスト (`ice_server_url_configurer_*`、`parse_proxy_info_*` など) は影響を受けず引き続き動作する。
- `cargo test --workspace` がすべて通る。
- ルート `Cargo.toml` の `[dev-dependencies]` から `proptest.workspace = true` が外れている。
