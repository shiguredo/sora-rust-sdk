# `tokio` feature から `rt-multi-thread` を外し `rt` のみ要求する

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-drop-tokio-rt-multi-thread-feature
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`tokio` の `rt-multi-thread` 削除」分。

## 目的

SDK 本体は `tokio::runtime::Runtime::new()` や `Builder::new_multi_thread()` を一切呼んでいないにもかかわらず、`Cargo.toml` の `tokio` features に `rt-multi-thread` を含めており、利用者側のランタイム選択を過剰に拘束している。`rt` のみの要求に縮小し、利用者が必要に応じて `rt-multi-thread` を追加する形に改める。

## 優先度根拠

Medium。

- 正式リリース 2026.1.0 前に正しい feature 指定に整える。後から外しても Cargo の feature unification により利用者の依存グラフ次第で `rt-multi-thread` が再活性化されるため、効果には限界がある。それでも、SDK 自体が `rt-multi-thread` を要求しないことを明示するのは tokio ベース SDK の標準的な配慮として意味がある
- SDK が利用者のランタイム選択 (`current_thread` / `multi_thread`) を前提としない設計に揃える
- 修正は `Cargo.toml` 1 行の置換とテスト 1 件の flavor 変更で完結する

## 現状

### `Cargo.toml:108-118`

```toml
tokio = {
  workspace = true,
  features = [
    "io-util",
    "macros",
    "net",
    "rt-multi-thread",
    "sync",
    "time",
  ]
}
```

### SDK 本体の tokio 利用状況

- `src/` 配下に `tokio::runtime::Runtime::new()` / `Builder::new_multi_thread()` の直接呼び出しはゼロ
- SDK は利用者側で起動した tokio ランタイム上で動作する前提 (`tokio::spawn` は `rt` feature で動作)
- `tokio::task::JoinSet` (`src/connection.rs:2400`) は `rt` feature で利用可能 (tokio ドキュメント確認済み)

### テストの注意点

- `src/connection.rs:2631` に `#[tokio::test]` が 1 件存在する。デフォルト flavor は `multi_thread` のため `rt-multi-thread` を要求する。本変更では `#[tokio::test(flavor = "current_thread")]` に変更する (当該テストはシングルスレッドで十分動作する内容)

### 利用クレートの状況

- `examples/sumomo/Cargo.toml`: 既に `rt-multi-thread` を独自に宣言しているため本変更の影響なし
- `e2e-tests/Cargo.toml`: `features = ["full"]` で影響なし
- `pbt/Cargo.toml`: tokio 非依存で影響なし
- `README.md:83`: 利用者向けサンプルコードに `rt-multi-thread` が例示されているため本 issue で修正する

## 設計方針

- `rt-multi-thread` を `rt` に置き換える
- `tokio::spawn` / `tokio::select!` / `tokio::time::sleep` / `tokio::task::JoinSet` は `rt` + `time` + `macros` の組み合わせで動作する
- 利用者側がマルチスレッドランタイムを使いたい場合は、利用者の `Cargo.toml` で `tokio = { features = ["rt-multi-thread", ...] }` を宣言すれば、Cargo の feature unification で SDK 側にも活性化される
- `#[tokio::test]` の flavor を `current_thread` に変更し、テストが `rt` のみで動作するようにする

## 完了条件

- `Cargo.toml:108-118` の `tokio` features から `rt-multi-thread` が削除され `rt` に置き換わっている
- `src/connection.rs:2631` の `#[tokio::test]` が `#[tokio::test(flavor = "current_thread")]` に変更されている
- `cargo test -p sora_sdk --lib` が `rt` feature のみで通過する
- `cargo build --no-default-features` が通過する
- `README.md` の tokio 依存のサンプルコードから `rt-multi-thread` が削除され、利用者が自前で宣言する旨の説明に更新されている (`examples/sumomo/Cargo.toml` の例を参照させる)
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `Cargo.toml:114`: `"rt-multi-thread"` を `"rt"` に置換する
2. `src/connection.rs:2631`: `#[tokio::test]` を `#[tokio::test(flavor = "current_thread")]` に変更する
3. `cargo test -p sora_sdk --lib` でテストが通過することを確認する
4. `cargo build --no-default-features` でビルドが通過することを確認する
5. `README.md` の tokio 依存のサンプルコードを `features = ["rt", "macros", "time"]` に更新し、`rt-multi-thread` が必要な場合は利用者側で追加する旨の説明を追記する
