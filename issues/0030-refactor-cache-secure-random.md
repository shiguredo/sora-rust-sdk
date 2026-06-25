# `SecureRandom` を毎フレーム初期化せずに保持し、masking_key / nonce の `expect()` panic 経路を廃止する

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/cache-secure-random-and-fix-panic-paths
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「`SecureRandom` 毎フレーム初期化と panic 経路」分。

## 目的

`src/connection.rs:1852-1870` の `SecureRandom` は `RandomSource` トレイトの `masking_key()` / `nonce()` 実装で WebSocket フレームを送出するたびに `aws_lc_rs::rand::SystemRandom::new()` を生成・破棄している。さらに `fill()` の失敗を `expect("failed to generate masking key")` / `expect("failed to generate nonce")` で panic 化しているため、シグナリングメッセージ送出 1 回ぶんの RNG 取得失敗で WebSocket スレッドごと落ちる。

本 issue では (1) `SystemRandom` を `SoraConnection` (もしくは `WebSocketStream` を所有する箇所) で 1 回だけ生成して保持する、(2) `fill()` 失敗時の `expect()` を `panic` させない経路に置き換える、の 2 点を行う。

## 優先度根拠

Medium。

- `SystemRandom::new()` は `aws-lc-rs` 上は実質ゼロコストに近いが、毎フレーム呼ぶ意味は無く、設計上の汚れ
- `fill()` 失敗は `aws-lc-rs` の `getrandom` 呼び出しが失敗するケースで、Linux/macOS の通常環境では起こらないが、組み込み Linux や jailed 環境では `EAGAIN` などで失敗する可能性がある
- WebSocket フレーム送出側 (`shiguredo_websocket` の `RandomSource`) で panic すると SDK のシグナリングスレッドが死に、利用側からは復旧不能になる
- 修正は数行で、API 互換性も維持される
- 致命度は低い (実環境ではほぼ発火しない) が、正式リリース 2026.1.0 の段階で「`expect()` で落ちる経路がパケットごとに残っている」状態は望ましくない

## 現状

`src/connection.rs:1852-1870`:

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

- `SecureRandom` は ZST (zero-sized type) で、毎メソッド呼び出し時に `SystemRandom::new()` を作っている
- WebSocket フレームを送るたびに RNG オブジェクトを生成・破棄しているため、ホットパスでメモリアロケーションやスレッドローカル状態の再初期化が起こる可能性 (`aws-lc-rs` の実装依存) がある
- `fill()` 失敗時の挙動は「panic」: 復旧の余地が無い

`SecureRandom` は `shiguredo_websocket` (もしくは `noflate`) の `RandomSource` トレイトを実装した型として `WebSocket` ハンドシェイクに渡されている。シグネチャ上 `&mut self` を取れるため、状態を保持して良い。

## 設計方針

### `SystemRandom` を保持する

`SecureRandom` を以下のように変更する:

```rust
struct SecureRandom {
    rng: SystemRandom,
}

impl SecureRandom {
    fn new() -> Self {
        Self { rng: SystemRandom::new() }
    }
}
```

`SoraConnection` (もしくは WebSocket を生成する箇所) で `SecureRandom::new()` を 1 回だけ呼んで保持する。

### `fill()` 失敗時の挙動

`RandomSource` トレイトは `masking_key() -> [u8; 4]` / `nonce() -> [u8; 16]` のように Result を返さない API になっている (`shiguredo_websocket` の定義に従う)。そのため失敗時は何らかの値を返すか panic するかの二択しかない。

選択肢:

1. **`expect()` を残すが、メッセージで `aws-lc-rs` の挙動を明示する**
   - 現状とほぼ同じだが、メッセージで「`aws-lc-rs` の `SystemRandom::fill` が失敗した。通常起きないが OS の `getrandom` が利用不能の可能性」と明記する
2. **失敗時はゼロ埋めバッファを返してログ警告を出す**
   - WebSocket masking key がゼロの場合、サーバー側で復号できないため接続自体が壊れる。事実上は接続が落ちるが、SDK 全体は生存できる
3. **`shiguredo_websocket` の `RandomSource` トレイトを Result 返却に変更する (上流改修)**
   - 上流クレートの API 変更が必要で、本 issue のスコープを超える

選択肢 1 で「`expect()` の理由を明記する」が最も現実的。選択肢 2 は WebSocket プロトコル上は仕様違反 (RFC 6455 では XOR mask の値に制約はないが、ゼロマスクは事実上「マスクしていない」扱い) であり、復旧経路としてもまともに機能しないため避ける。選択肢 3 は別 issue として上流に提案する。

実装フェーズで `shiguredo_websocket::RandomSource` のシグネチャを再確認し、Result を返せる版があれば移行する。無ければ選択肢 1 で `expect()` メッセージを精緻化する。

## 完了条件

- `SecureRandom` が `SystemRandom` を 1 回だけ保持し、`SoraConnection` 側でこのインスタンスを使い回している
- `aws_lc_rs::rand::SystemRandom::new()` が WebSocket フレーム送出ごとに呼ばれなくなっている
- `fill()` 失敗時の `expect()` メッセージが「`aws-lc-rs` の `SystemRandom::fill` 失敗」「OS RNG が枯渇 / 不在の可能性」を明示している (`shiguredo_websocket` API が Result を返せない前提の場合)
- 上流 (`shiguredo_websocket`) で `RandomSource` を Result 返却版に変更できる場合は、そちらに移行する
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/connection.rs:1852-1870` の `SecureRandom` を `{ rng: SystemRandom }` 構造に変更し、`new()` を追加する
2. `SoraConnection` を構築する箇所で `SecureRandom::new()` を 1 回だけ呼び、保持する (現状 `SecureRandom` を渡している箇所を grep して特定する)
3. `masking_key()` / `nonce()` は `self.rng.fill(&mut buf)` を呼ぶように変更する
4. `expect()` メッセージを「`aws-lc-rs SystemRandom::fill failed; OS RNG unavailable or exhausted`」のような英語メッセージに統一する (AGENTS.md「ログメッセージは全て英語にすること」)
5. `shiguredo_websocket::RandomSource` の最新シグネチャを確認し、Result 返却版があれば API を切り替える
6. テストでは `SecureRandom` を直接 `RandomSource` として使い、`masking_key()` / `nonce()` が呼ばれるたびに `SystemRandom::new()` を作らないことをコードリーディングで確認する (副作用の不在を直接テストするのは困難なので、コードレビューで担保)
