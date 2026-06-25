# `now()` から `SystemTime::duration_since(UNIX_EPOCH)` の panic 経路を除去し、`Result` でエラーを伝播する

- Priority: Medium
- Created: 2026-06-23
- Completed: 2026-06-25
- Model: Opus 4.7
- Branch: feature/fix-remove-now-unix-epoch-panic
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`now()` の panic 経路」分。

## 目的

`src/connection.rs:1872-1878` の `now()` は `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` で UNIX エポック以降の経過時間を計算しているが、システム時計が 1970 年以前を返した場合に `unwrap()` が panic する。組み込みデバイスや RTC が初期化されていない環境、コンテナで `--cap-drop` が誤って働いて時刻が取得できない場合などで発生し得る。

本 issue では `now()` の戻り値型を `Timestamp` から `Result<Timestamp>` に変更し、`?` 演算子でエラーを伝播させる。これにより panic 経路を廃止する。L878 のメインループ側ではエラーが `run()` の戻り値 `Result<()>` として上位層に伝わり、利用者側でログ出力や再接続判断が可能になる。

## 優先度根拠

Medium。

- サーバー / 一般のクライアント環境では `SystemTime::now()` が UNIX エポック以前を返すことはなく、`unwrap()` で panic することはまずない
- 組み込み Linux (Raspberry Pi の RTC 未設定、Yocto ベースの初回起動など) や、コンテナの sandbox 時刻設定によっては発生する
- 当該 panic は WebSocket メッセージのタイムスタンプ生成で起こる場合、シグナリングスレッドが落ち SDK が機能停止する
- 修正は小さく、`Error` に 1 バリアント追加、`now()` のシグネチャ変更、呼び出し元 2 箇所の `now()?` 化で完結する
- 正式リリース 2026.1.0 後でも修正可能だが、`unwrap()` で落ちる箇所を残したまま正式版を切るのは方針として避けたい
- 同様の panic 経路除去 issue `0032` は「対応不要」で closed になったが、あちらは RNG 枯渇由来で Linux 通常環境では事実上発生しないのに対し、本件は RTC 未初期化という組み込み Linux では現実的なシナリオが存在する
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

- `now()` は非公開関数 (`fn`, `pub` なし) であるため、シグネチャ変更による公開 API への影響はない
- `Timestamp` は `shiguredo_websocket` から import されている (`use shiguredo_websocket::{ ..., Timestamp, ... };`) Sans I/O パターン用の型で、`from_millis(u64) -> Self` は引数の `u64` をそのまま内部値として保持するだけの単純なコンストラクタであり、任意の `u64` に対して panic しない
- `SystemTime::now()` がシステム時計を読み、`UNIX_EPOCH` より前を返したときに `duration_since()` は `Err(SystemTimeError)` を返す
- `unwrap()` によりその場合に panic する

`now()` の呼び出し元は以下の 2 箇所のみ:

1. `src/connection.rs:878` — `run()` 内のメインループ
   ```rust
   ws.feed_recv_buf(&buf[..n], now())?;
   ```
2. `src/connection.rs:1287` — `run()` 内のクローズ処理ループ
   ```rust
   ws.feed_recv_buf(&buf[..n], now())?;
   ```

いずれも `pub async fn run(mut self) -> Result<()>` (L749) 内にあり、`?` 演算子でエラー伝播が可能なコンテキストである。L1287 は `tokio::time::timeout` の `async` ブロック内で、`return Ok::<_, Error>(())` (L1280) により `?` のエラー型が `Error` と推論される。`timeout` 境界でエラーが消費され `run()` の呼び出し元までは伝播しないが、panic 除去という本 issue の目的は達成される。L878 側ではエラーが `run()` の戻り値として上位層に伝播する。

`feed_recv_buf(&mut self, buf: &[u8], now: Timestamp)` に渡された `Timestamp` は、Sans I/O パターンに従い、WebSocket ハンドシェイク (`Connecting` 状態) のタイムアウト管理に利用される。`Connected` / `Closing` 状態ではフレーム処理に渡されず、単調増加性や絶対時刻の正確性は要求されない。

## 設計方針

`now()` の戻り値型を `Timestamp` から `Result<Timestamp>` に変更し、システム時計異常時に panic ではなく `Err` を伝播させる。エラーは `run()` の戻り値として利用者側に通知される。

### Result アプローチ（採用）

```rust
fn now() -> Result<Timestamp> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis() as u64;
    Ok(Timestamp::from_millis(millis))
}
```

`duration_since(UNIX_EPOCH)` が `Err(SystemTimeError)` を返した場合、`From<std::time::SystemTimeError> for Error` により自動的に `Error::InvalidSystemTime` に変換され `?` で伝播する。

- 呼び出し元 2 箇所はいずれも `Result<()>` コンテキスト内で `ws.feed_recv_buf(&buf[..n], now()?)?` と書くだけでエラー伝播が成立する（L1287 のエラーは `tokio::time::timeout` 境界で消費され `run()` の呼び出し元までは伝播しないが panic はしない）
- L878 側のエラーは `run()` の戻り値 `Result<()>` として上位層に伝わり、利用者側でログ出力や再接続判断が可能になる
- スレッドを panic で落とさないため、SDK 全体が機能停止する事態を避けられる
- 選択肢 A （ゼロ埋め）と異なり、システム時計異常を隠蔽せず運用者が異常を検知できる
- `Error::InvalidSystemTime` の追加のみで実装でき、コードの複雑化は最小限

#### `Error::InvalidSystemTime` バリアントの追加

`src/error.rs` の `Error` 列挙型に以下を追加する:

```rust
InvalidSystemTime {
    source: std::time::SystemTimeError,
},
```

`Display` 実装:

```rust
Error::InvalidSystemTime { source } => write!(
    f,
    "システム時刻が UNIX エポック (1970-01-01) より前です: {source}"
),
```

`std::error::Error::source()` 実装:

```rust
Error::InvalidSystemTime { source } => Some(source),
```

`From<std::time::SystemTimeError> for Error` 実装:

```rust
impl From<std::time::SystemTimeError> for Error {
    fn from(err: std::time::SystemTimeError) -> Self {
        Error::InvalidSystemTime { source: err }
    }
}
```

`std::time::SystemTimeError` は Rust 1.42 以降で `std::error::Error` を実装しており、本プロジェクトの MSRV 1.88 では問題なく利用できる。

### 不採用案

#### 選択肢 A （ゼロ埋め）

`unwrap_or(0)` でエラー時は `Timestamp::from_millis(0)` を返す。一見単純だが、`Timestamp(0)` は 1970-01-01 00:00:00 UTC のタイムスタンプであり、WebSocket ハンドシェイクのタイムアウト判定に渡された場合、全タイマーが即座に期限切れと判定される可能性がある。また、システム時計異常という致命的な状況を完全に隠蔽してしまい、運用者が異常を検知できない。

#### 選択肢 B （`tokio::time::Instant` への切り替え）

`tokio::time::Instant` は不透明な起点からの経過時間であり、`Timestamp::from_millis(u64)` が期待する UNIX エポックからのミリ秒と本質的に異なる。`feed_recv_buf` に渡す `Timestamp` の意味論を SDK が勝手に切り替えることはできず、また `Instant` から `u64` への変換に起動時 `Instant` を基準とするラッパーが必要になるなど、単純な解決にならない。

## 完了条件

- `now()` の戻り値型が `Timestamp` から `Result<Timestamp>` に変更されている
- `src/error.rs` に `InvalidSystemTime { source: std::time::SystemTimeError }` バリアントが追加され、`Display`、`source()`、`From<std::time::SystemTimeError>` が実装されている
- 呼び出し元 2 箇所 (`src/connection.rs:878`, `src/connection.rs:1287`) が `now()?` に変更され、L878 側ではエラーが `run()` の戻り値 `Result<()>` として伝播する。L1287 側は `tokio::time::timeout` 境界でエラーが消費されるが panic はしない
- `SystemTime::now()` が `UNIX_EPOCH` より前を返しても `now()` が panic しない（コードレビューで確認）
- `src/connection.rs` 内の `now()` から `unwrap()` が消えている
- テスト: `src/connection.rs` の `#[cfg(test)] mod tests` 内に `now()` が `Ok(Timestamp)` を返すことを確認する正常系単体テストを追加する（テスト内では `super::now()` で呼び出す）。システム時計操作が必要な異常系テストは不可能なためコードレビューで担保する
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- `grep -rnE 'SystemTime|UNIX_EPOCH' src/ e2e-tests/ examples/` で同種の `unwrap()` / `expect()` が他にないか確認し、あれば本 issue のスコープ外であることを明記する
- 本 issue 対応により `0020` の S7 (`.unwrap()` 47 件の `.expect("MESSAGE")` 化) の作業対象から `now()` の `unwrap()` 1 件が外れる。実装時に `0020` issue に完了コメントを残し、S7 担当者にメンションすること

(End of file - total 128 lines)

## 解決方法

- `now()` の戻り値型を `Result<Timestamp>` に変更し `?` でエラー伝播
- `Error::InvalidSystemTime` バリアントを追加し Display/source/From 実装
- 呼び出し元 2 箇所を `now()?` に変更

### 修正ファイル
- `src/connection.rs`
- `src/error.rs`
