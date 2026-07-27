# HTTP CONNECT 経由 TLS 接続で、CONNECT 200 応答直後の余剰バイトを TLS 復号層に注入する経路を塞ぐ

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-http-proxy-tls-response-injection
- Polished: 2026-07-27

## 目的

`connect_websocket` (proxy 経由 + TLS ターゲット) が、HTTP CONNECT 応答直後にプロキシ側の平文 TCP から読み取った余剰バイト (`pending`) を、TLS ハンドシェイク完了後に `ClientStream::push_pending_read(pending)` で **アプリ層から「TLS 復号済みバイト」として読める位置に注入** している。この経路により、悪意ある / 侵害された HTTP CONNECT プロキシは 200 応答直後に任意バイトを付随させるだけで、TLS の完全性保護 (RFC 8446 §5) を迂回してアプリ層に任意データを挿入できる。TLS ターゲットでは余剰バイトを必ず拒否するよう修正する。

## 優先度根拠

High (セキュリティ致命)。TLS を通しても悪意あるプロキシから WebSocket 応答注入攻撃 (Sora シグナリング乗っ取り) が可能。認証プロキシで容易に成立する攻撃経路であり、CVE 相当の深刻度がある。

## 現状

`src/connection.rs:2332-2358` あたりで、以下のように pending バイトを TLS 側の read キューに挿入している:

```rust
if target.tls {
    let (tcp_stream, pending) = stream.into_plain_parts()
        .expect("BUG: proxy 接続後は plain stream のはずです");
    let tls_stream = connect_tls(&target.host, tcp_stream, tls_config, deadline).await?;
    let mut stream = ClientStream::new_tls(tls_stream);
    stream.push_pending_read(pending);   // ← ここで平文の余剰バイトが TLS 復号側 read に混入
    Ok(stream)
}
```

`ClientStream::read` (2134-2145) は `pending_read` を優先返却するため、平文の任意バイトが「TLS 復号済み」としてアプリ層 (WebSocket ハンドシェイクパーサ) に届く。

`pending` は shiguredo_http11 の `ResponseDecoder::take_remaining()` で得た、CONNECT 200 応答パースの残りバイト。TLS 前 (平文プロキシ TCP) から読み取ったバイトである。

## 設計方針

1. TLS ターゲット (`target.tls == true`) の場合、`connect_websocket` 内で `into_plain_parts()` から得た `pending` が空でなければ **即エラー (`Error::ProxyConnectUnexpectedTrailingData`) で接続を拒否** する。TLS 接続では ClientHello をクライアントが先に送るため、TLS 開始前にサーバーからバイトが届くことはありえない。余剰バイトは必ずプロキシによる挿入または応答注入であり、これを拒否する。
2. 新規エラー variant `Error::ProxyConnectUnexpectedTrailingData` を追加する。
3. 非 TLS (平文 ws://) の場合のみ、`pending` を `push_pending_read` で受け入れることを許容する。非 TLS では TLS レコード層による完全性保護が存在せず、プロキシが CONNECT 200 応答後に任意バイトを挿入しても暗号境界を侵害しない。また後方互換性の観点から、現状の挙動を維持する。
4. 単体テストで「TLS ターゲット + CONNECT 200 応答直後に任意バイトが追加された場合に接続が拒否される」ことを検証する。

## 完了条件

- TLS ターゲットで CONNECT 200 応答直後に余剰バイトがある場合、`connect_websocket` が Err を返してハンドシェイクが始まらない。
- 非 TLS ターゲットでは従来通り余剰バイトを `push_pending_read` で受け入れる。
- 単体テストで応答注入経路が拒否されることを検証している。
- `cargo test --workspace --all-features` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
