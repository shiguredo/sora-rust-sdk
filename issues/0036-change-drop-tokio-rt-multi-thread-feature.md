# `tokio` feature から `rt-multi-thread` を外し `rt` のみ要求する

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/drop-tokio-rt-multi-thread-feature
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`tokio` の `rt-multi-thread` 削除」分。

## 目的

`Cargo.toml:108-118` の `tokio` 依存は features に `rt-multi-thread` を含めているが、SDK 本体は `tokio::runtime::Runtime::new()` や `Builder::new_multi_thread()` を一切呼んでおらず、現状コード上は `rt` で十分。

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

`rt-multi-thread` を強制すると、SDK 利用者側のランタイム選択 (current_thread スケジューラを使いたい WASM ターゲットや、シングルスレッド組み込みターゲット) と衝突する可能性がある。本 issue では feature を `rt` に絞り、必要があれば利用者側が `tokio` 経由で `rt-multi-thread` を追加要求する形に整える。

## 優先度根拠

Medium。

- 正式リリース 2026.1.0 で `rt-multi-thread` を強制したまま公開すると、後から外すのは破壊変更ではないが、(1) 一度依存に入った feature は他クレートのビルドプランで活性化されるため、効果は限定的、(2) shrinking しても利用者の experience は変わらない、という観点で「正式版時点で正しい状態」に整えるのが正攻法
- SDK が `current_thread` スケジューラの利用者の前提を壊さない設計に揃えるのは、tokio ベース SDK としての標準的な配慮
- 修正は `Cargo.toml` 1 行と CI ビルド確認のみ

## 現状

`Cargo.toml:108-118`:

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

確認結果:

- `grep -rn 'Runtime::new\|Runtime::builder\|new_multi_thread\|new_current_thread' src/` で `tokio::runtime` の直接利用は src/ 配下にゼロ件 (要再確認)
- SDK は `#[tokio::main]` 経由でランタイムを起動するのではなく、利用者側で起動した tokio ランタイム上で動くことを想定 (= `tokio::spawn` / `tokio::time::sleep` を内部で使うのみ)
- `tokio::spawn` は `rt` feature で十分動く (`rt-multi-thread` は `Builder::new_multi_thread()` を使うときに必要)

## 設計方針

- `rt-multi-thread` を `rt` に置き換える
- `tokio::spawn` / `tokio::select!` / `tokio::time::sleep` 等は `rt` + `time` + `macros` の組み合わせで動く
- 利用者側がマルチスレッドランタイムを使いたい場合は、利用者の `Cargo.toml` で `tokio = { version = "...", features = ["rt-multi-thread", ...] }` を宣言してもらえばよい (Cargo の feature unification で SDK 側にも `rt-multi-thread` が活性化される)
- examples/sumomo はサンプル CLI として `#[tokio::main]` でマルチスレッドランタイムを起動している可能性がある。examples/sumomo の Cargo.toml は別判断で、`rt-multi-thread` を直接要求する

## 完了条件

- `Cargo.toml:108-118` の `tokio` features から `rt-multi-thread` が削除され `rt` に置き換わっている
- 全 feature 組み合わせで `cargo build` / `cargo test` が通る (default + 各個別 feature)
- `examples/sumomo/Cargo.toml` で `tokio` の features に `rt-multi-thread` が明示されている (sumomo が必要とする場合)
- CI で `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- 利用者向けの README で「本 SDK は tokio の current_thread / multi_thread のどちらでも動くこと」を明示する (オプション、親 issue M6 / S5 の README 整備と整合させる)

## 解決方法

1. `Cargo.toml:108-118` の `rt-multi-thread` を `rt` に置換する
2. `cargo build --no-default-features` から各個別 feature 組み合わせまで局所ビルドを通す
3. `examples/sumomo` が `rt-multi-thread` を要求しているかを確認し、必要なら `examples/sumomo/Cargo.toml` に `tokio = { workspace = true, features = ["rt-multi-thread", ...] }` を追記する
4. `grep -rn 'tokio::runtime\|Runtime::' src/ tests/` で SDK 本体に `tokio::runtime` の直接利用が無いことを再確認する
5. CI (parent issue S4 の matrix 強化と合流する場合は併せて) で current_thread ランタイムでも動くことを smoke テストで確認する (例: `tokio::runtime::Builder::new_current_thread().enable_all().build()` を使った最小サンプル)
6. 親 issue S1 のテスト戦略強化と連携できれば、current_thread ベースの単体テストを 1 件追加する
