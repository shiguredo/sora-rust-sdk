# シグナリング URL シャッフルの modulo bias を解消し、`SystemRandom::fill` 失敗時の panic 経路を廃止する

- Priority: Low
- Created: 2026-06-23
- Completed: 2026-06-24
- Model: Opus 4.7
- Branch: feature/fix-url-shuffle-modulo-bias-and-panic
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S3 (公開 API 設計の追加修正) のうち「URL シャッフルの modulo bias」分。

## 目的

`src/connection.rs:788-799` の Fisher-Yates シャッフルで、(1) 乱数のモジュロ演算 `(u64 % (i+1))` による modulo bias が残り、(2) `SystemRandom::fill()` 失敗時に `expect("failed to generate random bytes for URL shuffle")` が panic を引き起こす。

`u64` の値域は $2^{64}$ なので modulo bias の偏りは $\le 8$ 個程度の URL では実質ゼロに近く、振り分けの均一性に致命的な影響は無い。一方で「正しいシャッフル実装」のテンプレートからは外れているため、正式リリース 2026.1.0 のタイミングで rejection sampling に揃え、合わせて RNG 失敗時の panic 経路を廃止する。

## 優先度根拠

Low。

- modulo bias 自体は URL 数が 64bit に比べて桁違いに小さいため、実害ほぼゼロ
- `SystemRandom::fill()` 失敗は通常環境で発生しないが、もし発生すると signaling 接続 1 回ぶんの試行で panic する
- 修正は数行で済み、副作用も無い
- ただし負荷分散の観点で「均一性」を売りにしている挙動なので、実装としては rejection sampling に揃えたほうが望ましい

## 現状

`src/connection.rs:788-799`:

```rust
// URL リストをランダム化して負荷分散する (Fisher-Yates シャッフル)
let mut urls = signaling_urls.clone();
if urls.len() > 1 {
    let rng = SystemRandom::new();
    for i in (1..urls.len()).rev() {
        let mut buf = [0u8; 8];
        rng.fill(&mut buf)
            .expect("failed to generate random bytes for URL shuffle");
        let j = (u64::from_le_bytes(buf) % (i as u64 + 1)) as usize;
        urls.swap(i, j);
    }
}
```

問題点:

1. `u64::from_le_bytes(buf) % (i as u64 + 1)` は modulo bias を持つ
   - `i + 1` が $2^{64}$ を割り切らない限り、最小値付近にわずかな偏りが残る
   - URL 数 (`i + 1`) が $2^{64}$ に比べて桁違いに小さいため、実害は実質ゼロだが、教科書的には「rejection sampling」または「u128 を使う」のが正解
2. `expect()` で panic
   - `SystemRandom::fill()` が失敗するのは OS の `getrandom` が利用不能 / 枯渇しているような状況
   - `signaling_urls` のシャッフルは負荷分散の最適化なので、失敗時はシャッフルせずに元順序で続行できれば SDK 接続自体は維持できる

## 設計方針

### modulo bias の解消

選択肢 A: rejection sampling

```rust
fn random_below(rng: &SystemRandom, n: u64) -> Result<u64, ()> {
    // 2^64 を n の倍数で割り切れる最大値 (threshold) を計算
    // u64 -> n のマッピングで偏りが出る区間を捨てて再抽選する
    let threshold = (u64::MAX / n) * n;
    loop {
        let mut buf = [0u8; 8];
        rng.fill(&mut buf).map_err(|_| ())?;
        let r = u64::from_le_bytes(buf);
        if r < threshold {
            return Ok(r % n);
        }
    }
}
```

選択肢 B: `u128` を経由

```rust
let mut buf = [0u8; 16];
rng.fill(&mut buf).map_err(|_| ())?;
let r = u128::from_le_bytes(buf);
let j = (r % (i as u128 + 1)) as usize;
```

`u128` 経由でも数学的には bias は残るが、URL 数が 128bit に比べて桁違いに小さいため、観測可能な偏りはゼロ。シンプルさで選ぶなら選択肢 B。教科書的な厳密性で選ぶなら選択肢 A。実装フェーズで判断する。

### panic 経路の廃止

`SystemRandom::fill()` が失敗した場合は、シャッフルを諦めて元順序のまま続行する:

```rust
if urls.len() > 1 {
    let rng = SystemRandom::new();
    let shuffled = try_shuffle_urls(&mut urls, &rng);
    if !shuffled {
        rtc_log_warning!("URL shuffle skipped: SystemRandom::fill failed; falling back to original order");
    }
}
```

シグナリング接続自体は続行できる (順序が固定になるだけで負荷分散の効果が落ちるが、機能は維持される)。

## 完了条件

- modulo bias が rejection sampling もしくは `u128` 経由で解消されている
- `SystemRandom::fill()` 失敗時に panic せず、シャッフルをスキップして元順序で続行する
- 失敗時はログ警告 (英語、AGENTS.md 規約) を出す
- `urls.len() <= 1` のとき RNG を作らない (現状維持)
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- 単体テスト: 既知のシード (PBT で `SystemRandom` を直接置換できないので、シャッフル関数を `SystemRandom` 依存から `dyn FnMut(u64) -> Option<u64>` 注入型に分離する) でシャッフル結果が決定論的に検証できる構造に整える

## 解決方法

1. `src/connection.rs:788-799` の Fisher-Yates シャッフルを、内部関数 `try_shuffle_urls(&mut Vec<String>, &SystemRandom) -> bool` に切り出す
2. 内部関数内では rejection sampling (選択肢 A) もしくは `u128` 経由 (選択肢 B) を採用する
3. `rng.fill()` が `Err` を返した場合は早期 return `false` で抜け、呼び出し元はログ警告を出してシャッフルを諦める
4. テスト容易性のため、`try_shuffle_urls` の RNG 部分をトレイトか関数注入で抽象化する案も検討する (ただし「モックやスタブは絶対に利用しないこと」(AGENTS.md) のため、本物の `SystemRandom` を使うか、シャッフルロジックを純関数化して `Vec<u64>` を入力に取る単体テストにする)
5. テストは決定論的な乱数列を入力に「期待されるシャッフル結果が得られる」「`u64::MAX` 付近の値が `n` の倍数の手前で reject される」ことを確認する

## 解決方法

本 issue は対応不要と判断し、コード変更なしで closed にする。

理由:

- modulo bias は URL 数が高々 100 以下 (< $2^{64}$ に比べて桁違いに小さい) の状況では偏りが $10^{-18}$ オーダーであり、完全に無視できる
- `SystemRandom::fill()` 失敗は通常の Linux 環境では事実上発生しない。`getrandom()` が seccomp で塞がれるような極端な環境では、他の箇所でも同様に落ちるため panic でも実害は無い
- rejection sampling や u128 経由の対応を入れると、効果が無いにも関わらずコードが複雑化するだけである
