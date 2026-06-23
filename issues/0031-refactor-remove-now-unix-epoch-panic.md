# `now()` の `SystemTime::duration_since(UNIX_EPOCH).unwrap()` を panic 経路から除く

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/remove-now-unix-epoch-panic
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`now()` の panic 経路」分。

## 目的

`src/connection.rs:1872-1878` の `now()` は `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` で UNIX エポック以降の経過時間を計算しているが、システム時計が 1970 年以前を返した場合に `unwrap()` が panic する。組み込みデバイスや RTC が初期化されていない環境、コンテナで `--cap-drop` が誤って働いて時刻が取得できない場合などで発生し得る。

本 issue では `now()` から panic 経路を除く。

## 優先度根拠

Medium。

- サーバー / 一般のクライアント環境では `SystemTime::now()` が UNIX エポック以前を返すことは無く、`unwrap()` で panic することはまず無い
- 組み込み Linux (Raspberry Pi の RTC 未設定、Yocto ベースの初回起動など) や、コンテナの sandbox 時刻設定によっては発生する
- 当該 panic は WebSocket メッセージのタイムスタンプ生成で起こる場合、シグナリングスレッドが落ち SDK が機能停止する
- 修正は数行で、`unwrap_or_else(|_| Duration::ZERO)` または `tokio::time::Instant` への切り替えで完結する
- 正式リリース 2026.1.0 後でも修正可能だが、`unwrap()` で落ちる箇所を残したまま正式版を切るのは方針として避けたい

## 現状

`src/connection.rs:1872-1878`:

```rust
fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    Timestamp::from_millis(millis)
}
```

- `SystemTime::now()` がシステム時計を読み、`UNIX_EPOCH` より前を返したときに `duration_since()` は `Err(SystemTimeError)` を返す
- `unwrap()` でその場合は panic する
- `Timestamp` は `shiguredo_websocket` 側 (もしくは内部) の型と思われる。`Timestamp::from_millis(u64)` のシグネチャから、ミリ秒精度の UNIX タイムスタンプを保持する型と推測

`now()` の呼び出し元は本 issue では特定しないが、grep で確認したうえで挙動の方針 (失敗時の値) を決める。

## 設計方針

### 選択肢 A: `unwrap_or_else(|_| Duration::ZERO)` でゼロ埋め

```rust
fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Timestamp::from_millis(millis)
}
```

- panic しない
- タイムスタンプは 0 になるが、ホスト側でシステム時計が壊れている状況なので、いずれにせよまともなタイムスタンプは返らない
- 利用側 (`Timestamp` を使う処理) でゼロ値を不正値として扱う設計になっている必要がある

### 選択肢 B: `tokio::time::Instant` への切り替え

シグナリング内部の用途が「単調増加するタイムスタンプ」であれば `tokio::time::Instant` (`std::time::Instant`) のほうが本質的に安全:

- システム時計の巻き戻し / 未初期化に依存しない
- `Instant::now()` は panic しない
- ただし `Timestamp::from_millis(u64)` の意味論が UNIX エポックからのミリ秒なのか、起動からのミリ秒なのかで採否が変わる

呼び出し元と `Timestamp` の意味を確認した上で選択する。「シグナリング用の WebSocket Ping/Pong タイムスタンプ」のような単調増加で十分な用途なら選択肢 B、「UNIX タイムスタンプとしてサーバーや log の絶対時刻に揃える必要がある」なら選択肢 A。

### 選択肢 C: panic させる代わりに `Result` 経路に上げる

`now()` の呼び出し側がエラー伝播を扱える文脈なら、`Result<Timestamp, Error>` を返す。ただし `now()` は副作用無く呼ばれる小さなヘルパーであり、Error 経路を上に伝えると `?` 演算子の負担が増える。基本は選択肢 A か B を採る。

## 完了条件

- `now()` から `unwrap()` が消えている (もしくは `tokio::time::Instant` ベースに切り替え)
- `SystemTime::now()` が `UNIX_EPOCH` より前を返しても panic しない
- `now()` の呼び出し元すべてで返却値の意味が変わっていない (UNIX エポック基準を維持するか、`Instant` ベースに移行した結果として意味が変わるかを明示)
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `now()` の呼び出し元を grep で特定する (`src/connection.rs` 内のみか、他モジュールでも使われているか)
2. 呼び出し元の `Timestamp` 利用箇所の意味論 (UNIX 時刻 vs 単調時刻) を確認する
3. 意味論に応じて選択肢 A または B を採用する
4. 単体テストでは「`SystemTime::now()` を壊す」ことができないため、コードレビュー + Clippy lint (将来 `clippy::unwrap_used` を有効化する場合に備える) で担保する
5. 実装後、`grep -nE 'SystemTime|UNIX_EPOCH' src/` の結果を確認し、他に同様の `unwrap()` が無いかも併せて確認する (本 issue のスコープ外なら別 issue にエスカレートする)
