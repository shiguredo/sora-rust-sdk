# `SecureRandom` を構造体に変更して `SystemRandom` を保持し、`expect()` メッセージを精緻化する

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-cache-secure-random-and-improve-expect-messages
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`SecureRandom` 毎フレーム初期化と panic 経路」分。

## 目的

`src/connection.rs:1852-1870` の `SecureRandom` は現在 ZST（単位構造体）であり、`RandomSource` トレイトの `masking_key()` / `nonce()` 実装のたびに内部で `aws_lc_rs::rand::SystemRandom::new()` を呼んでいる。`SystemRandom` 自体は ZST だが、毎フレームの関数呼び出しは不要である。

また `fill()` の失敗時に `expect("failed to generate masking key")` / `expect("failed to generate nonce")` で panic するが、エラーメッセージが貧弱で原因特定が困難である。

本 issue では (1) `SecureRandom` を `SystemRandom` をフィールドに持つ構造体に変更して `masking_key()` / `nonce()` 内での `SystemRandom::new()` 呼び出しをなくす、(2) `expect()` メッセージを精緻化して失敗原因の特定を容易にする。

## 優先度根拠

Medium。

- `SystemRandom::new()` は ZST の定数相当で実質ゼロコストだが、毎フレームの呼び出しは設計上の無駄である
- `fill()` 失敗は通常環境では発生しないが、組み込み Linux や jailed 環境では `EAGAIN` で失敗する可能性がある。URL シャッフル側の `fill()` 失敗は #0032 で対応不要と判断されたが、WebSocket フレーム送出はホットパスであり、パケット送出ごとにパニックする経路が残るのは望ましくない（`masking_key()` はフレーム送信のたびに、`nonce()` は接続確立時の 1 回だけ呼ばれる）
- `shiguredo_websocket::RandomSource` トレイトは `masking_key() -> [u8; 4]` / `nonce() -> [u8; 16]` と Result 非対応の API であるため、`fill()` 失敗時に panic 以外の復旧経路は取れない（上流改修は本 issue のスコープ外）
- 修正は `SecureRandom` の型変更 + `Clone` derive + 呼び出し側の変更 + `expect()` メッセージ変更の数行で、公開 API に影響しない

## 現状

### `SecureRandom` 定義 (`src/connection.rs:1852-1870`)

```rust
struct SecureRandom;

impl RandomSource for SecureRandom {
    fn masking_key(&mut self) -> [u8; 4] {
        let mut key = [0u8; 4];
        SystemRandom::new()
            .fill(&mut key)
            .expect("failed to generate masking key");
        key
    }

    fn nonce(&mut self) -> [u8; 16] {
        let mut nonce = [0u8; 16];
        SystemRandom::new()
            .fill(&mut nonce)
            .expect("failed to generate nonce");
        nonce
    }
}
```

`SecureRandom` は ZST。`SystemRandom` 自体も ZST かつ `#[derive(Clone)]` であり、`SystemRandom::new()` はコンパイル時定数を返す。`masking_key()` / `nonce()` 内で毎回 `SystemRandom::new()` を呼ぶのは無駄であり、フィールドとして保持すべき。

### SecureRandom の使用箇所

- `connection.rs:840`: `let mut ws = WebSocketClientConnection::new(options, SecureRandom);` （初期接続用 WebSocket）
- `connection.rs:1184`: `ws = WebSocketClientConnection::new(options, SecureRandom);` （リダイレクト先 WebSocket）

`WebSocketClientConnection::new(options, R)` は `R` を値で受け取るため、同一インスタンスの共有は不可能。リダイレクト時には新しい `SecureRandom` が必要だが、`SystemRandom` が ZST かつ `Clone` であるため、`SecureRandom` に `#[derive(Clone)]` を付与すればクローンは実質ゼロコストである。

### `RandomSource` トレイトの制約

`shiguredo_websocket::RandomSource` は `masking_key(&mut self) -> [u8; 4]` / `nonce(&mut self) -> [u8; 16]` と Result 非対応。`fill()` 失敗時に panic 以外の選択肢はなく、`expect()` メッセージの精緻化が現実的な対応となる。

## 設計方針

### `SecureRandom` の構造体化

```rust
#[derive(Clone)]
struct SecureRandom {
    rng: SystemRandom,
}

impl SecureRandom {
    fn new() -> Self {
        Self { rng: SystemRandom::new() }
    }
}

impl RandomSource for SecureRandom {
    fn masking_key(&mut self) -> [u8; 4] {
        let mut key = [0u8; 4];
        self.rng.fill(&mut key).expect("failed to generate masking key: aws-lc-rs SystemRandom::fill failed, OS RNG may be unavailable or exhausted");
        key
    }

    fn nonce(&mut self) -> [u8; 16] {
        let mut nonce = [0u8; 16];
        self.rng.fill(&mut nonce).expect("failed to generate nonce: aws-lc-rs SystemRandom::fill failed, OS RNG may be unavailable or exhausted");
        nonce
    }
}
```

- `SystemRandom` は ZST（`pub struct SystemRandom(())`）かつ `#[derive(Clone)]` であり、フィールド保持のコストはゼロ
- `#[derive(Clone)]` により `SecureRandom` のクローンが可能。`WebSocketClientConnection::new` が `R` を値で取るため、初期接続用とリダイレクト用の 2 回の WebSocket 生成でそれぞれクローンを渡す（クローンのコストは実質ゼロ）
- `masking_key()` / `nonce()` は `self.rng.fill()` を呼ぶだけになり、`SystemRandom::new()` の呼び出しがなくなる

### `expect()` メッセージの精緻化

`shiguredo_websocket::RandomSource` トレイトは Result 非対応であり、最新バージョンでもシグネチャに変更はないため、`fill()` 失敗時の選択肢は panic のみである。この前提で、デバッガビリティ向上のために以下を行う:

- 各 `expect()` メッセージを操作名 + 原因 + コンテキストを含む英語メッセージに変更する
- `masking_key` と `nonce` でメッセージを使い分け、panic 時にどちらの操作で失敗したかが分かるようにする

（ゼロ埋めフォールバックは WebSocket フレームのマスキングの実質的な無効化に繋がるため採用しない。上流 API 変更は別 issue として検討する。）

## 完了条件

- `SecureRandom` が `{ rng: SystemRandom }` 構造体に変更され、`#[derive(Clone)]` が付与されている
- `masking_key()` / `nonce()` 内で `SystemRandom::new()` が呼ばれず、代わりに `self.rng.fill()` が使われている
- `expect()` メッセージが操作名と `aws-lc-rs SystemRandom::fill` 失敗の旨を含む具体的内容に変更されている
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- 以下の単体テストが `src/connection.rs` の `#[cfg(test)] mod tests` 内に追加され、すべて通過すること（モック / スタブ禁止）:
  - `SecureRandom::new()` で生成したインスタンスの `masking_key()` を複数回呼んでも panic しないこと
  - `nonce()` を複数回呼んでも panic しないこと
  - 乱数性の簡易チェックとして、連続 2 回の呼び出しで異なる値が返ることの確認（衝突確率は無視できるほど低い。最初の 1 バイトが 0 でないことの確認は確率的に偽陰性が発生しうるため非推奨）

## 解決方法

1. `src/connection.rs:1852` の `struct SecureRandom;` を `#[derive(Clone)] struct SecureRandom { rng: SystemRandom }` に変更し、`impl SecureRandom { fn new() -> Self { Self { rng: SystemRandom::new() } } }` を追加する
2. `connection.rs:1854-1870` の `impl RandomSource for SecureRandom` の `masking_key()` / `nonce()` を、`SystemRandom::new().fill(...)` → `self.rng.fill(...)` に変更する
3. `masking_key()` の `expect()` メッセージを `"failed to generate masking key: aws-lc-rs SystemRandom::fill failed, OS RNG may be unavailable or exhausted"` に変更する（`nonce()` も同様に `"failed to generate nonce: ..."` に変更。操作名の区別を残す）
4. `src/connection.rs` の `use aws_lc_rs::rand::{SecureRandom as AwsSecureRandom, SystemRandom};` の import は現状維持で問題ない（`SystemRandom` を `SecureRandom` のフィールド型として使うため）
5. `connection.rs:838-839` 付近（`let (timer_tx, mut timer_rx) = ...` の後、`let mut ws = WebSocketClientConnection::new(options, SecureRandom);` の前）に `let secure_random = SecureRandom::new();` を追加する（`connection.rs:791` に URL シャッフル用の `let rng = SystemRandom::new();` が既に存在するため、変数名の衝突を避ける）
6. `connection.rs:840` の `WebSocketClientConnection::new(options, SecureRandom)` を `WebSocketClientConnection::new(options, secure_random.clone())` に変更する
7. `connection.rs:1184` の `WebSocketClientConnection::new(options, SecureRandom)` を `WebSocketClientConnection::new(options, secure_random.clone())` に変更する
8. `src/connection.rs` の `#[cfg(test)] mod tests` 内（`connection.rs:2490` 付近の既存テスト群の後に追加）に、完了条件に記載されたテストケースを実装する。AGENTS.md「テストはコメントを重視すること」「テストのログメッセージは全て日本語にすること」「モックやスタブは絶対に利用しないこと」を遵守する

### テスト戦略

- **単体テスト**: `masking_key()` / `nonce()` が panic せず非ゼロの乱数を返すことを検証。テストは `src/connection.rs` の `#[cfg(test)] mod tests` に追加する
- **PBT**: 適用しない。乱数生成の検証は確定的な property を持たない
- **Fuzzing**: 適用しない。任意入力を受け付ける経路ではない
