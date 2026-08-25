# テスト戦略を強化する

- Priority: Medium
- Created: 2026-07-23
- Completed: {YYYY-MM-DD}
- Model: Composer
- Branch: feature/test-strengthen-test-strategy
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S1。

## 目的

シグナリング・RPC・stats・主要オプション経路の回帰を、単体テストと e2e で機械的に検知できるようにする。現状はパース層の単体テストが薄く、失敗時にテストが緑のまま終わる経路もある。

## 優先度根拠

Medium。

- 正式リリース後の破壊変更検知コストが高い領域（メッセージパース・stats・接続オプション）
- 既に e2e / PBT の土台はあるが、穴が残っている
- 正式リリースのブロッカーではないが、リリース直後の品質担保に直結する

## 現状

確認済みの穴:

- `IncomingMessage::parse`（`src/signaling_types.rs:79`）と `RpcResponse::parse`（`src/rpc.rs:51`）に専用の単体テストが無い
- `e2e-tests/src/lib.rs:191` の `parse_stats_lossy` は `WebRtcStatsReport::parse` 失敗時に要素単位へフォールバックし、パース失敗を握りつぶす。stats アサーションが誤って合格しうる
- `e2e-tests/tests/redirect.rs:16-24` は環境変数や URL 数不足時に `return` するだけなので、テストは成功扱いになる（スキップではなく誤合格）
- TURN-TLS / `client_cert` / `spotlight` / `forwarding_filters` の e2e が不足
- `pbt/` は `prop_connection.rs` 程度で、パース層の PBT が手薄

## 設計方針

- まず誤合格を潰す（`redirect.rs` の早期 return、`parse_stats_lossy` の失敗扱い）
- 次にパース単体テストと必要最小限の PBT を追加する
- e2e は環境依存が大きいため、既存 e2e の流儀（環境変数・`#[ignore]` 等）に合わせて追加する
- モックやスタブは使わない（AGENTS.md）

## 完了条件

- `IncomingMessage::parse` と `RpcResponse::parse` の代表ケース単体テストがある
- `parse_stats_lossy` が壊れた JSON で黙って空成功しない（または呼び出し側が失敗を検知する）
- `redirect.rs` が環境不足時に成功扱いにならない
- TURN-TLS / `client_cert` / `spotlight` / `forwarding_filters` の e2e が追加されているか、対象外理由が issue / テストコメントに明記されている
- `cargo test --workspace` が通る

## 解決方法

1. `redirect.rs` の早期 `return` を `#[ignore]` 条件付き実行、または不足時 panic / 明示 skip に変える
2. `parse_stats_lossy` のフォールバック方針を見直し、テスト用途では厳密パースを使うか失敗を伝播する
3. `signaling_types` / `rpc` にパース単体テストを追加する
4. 必要なら `pbt/` にパース向けプロパティを追加する
5. 主要オプションの e2e を既存テストの隣に追加する
