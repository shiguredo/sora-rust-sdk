# URL 文字列の手動パースを `shiguredo_http11::uri::Uri` に置き換える

- Priority: Medium
- Created: 2026-07-24
- Completed: 2026-07-28
- Model: Opus 4.7
- Branch: feature/refactor-use-uri-parse-for-url-parsing
- Polished: 2026-07-27

## 目的

`src/types.rs` の `mask_url_userinfo` と `src/connection.rs` の `is_turn_tcp_or_udp_url` は、URL 文字列に対して `find("://")`、`find('@')`、`split_once(':')`、`split('?')` などの手動の文字列走査を行っている。これらのアドホックなパースは RFC 3986 に準拠しておらず、特に `mask_url_userinfo` ではクエリやフラグメント内の `@` を userinfo の `@` として誤検出するバグがある。

すでにプロジェクトに依存として導入済みの `shiguredo_http11::uri::Uri` は RFC 3986 準拠の URL パーサーであり、`authority()` / `query()` / `fragment()` を正しく分離する。この `Uri::parse` を利用して手動パースを置き換える。

## 優先度根拠

Medium。`mask_url_userinfo` は `ProxyInfo` の Debug 出力用であり、誤マスクによって機密情報が漏洩することはないが、デバッグログの URL 構造が壊れて障害調査の妨げになる。`is_turn_tcp_or_udp_url` はテストヘルパーであり、機能的な動作変更はない。

## 現状

### `mask_url_userinfo` (src/types.rs:95-115)

```rust
fn mask_url_userinfo(url: &str) -> std::borrow::Cow<'_, str> {
    let Some(after_scheme) = url.find("://") else {
        return std::borrow::Cow::Borrowed(url);
    };
    let after_scheme = after_scheme + 3;
    let url_after_scheme = &url[after_scheme..];
    let Some(at_pos) = url_after_scheme.find('@') else {
        return std::borrow::Cow::Borrowed(url);
    };
    if let Some(slash_pos) = url_after_scheme.find('/')
        && at_pos > slash_pos
    {
        return std::borrow::Cow::Borrowed(url);
    }
    let masked = format!(
        "{}<redacted>@{}",
        &url[..after_scheme],
        &url[after_scheme + at_pos + 1..]
    );
    std::borrow::Cow::Owned(masked)
}
```

- `http://host?token=abc@evil.com` → `url_after_scheme = "host?token=abc@evil.com"`、`slash_pos = None` のため早期 return されず `http://<redacted>@evil.com` にオーバーマスクされる。
- `http://user:pass@host` → 正しくマスクされる。
- `http://host/path?q=x@y` → `@` が `/` より後ろで正しくスキップされるが、`/` を含まないクエリのみの URL で誤動作する。

### `is_turn_tcp_or_udp_url` (src/connection.rs:2583-2601)

```rust
fn is_turn_tcp_or_udp_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let Some((scheme, _)) = lower.split_once(':') else {
        return false;
    };
    if scheme != "turn" && scheme != "turns" {
        return false;
    }
    lower
        .split('?')
        .nth(1)
        .and_then(|query| {
            query
                .split('&')
                .find_map(|param| param.strip_prefix("transport="))
        })
        .is_some_and(|transport| transport == "tcp" || transport == "udp")
}
```

`#[cfg(test)]` 内のテストヘルパーであり、`ice_server_url_configurer` のテストからのみ呼び出される。

## 設計方針

1. **`src/types.rs` に `use shiguredo_http11::uri::Uri;` を追加する。** `shiguredo_http11` はすでに workspace 依存として導入済みのため `Cargo.toml` の変更は不要。

2. **`mask_url_userinfo`:**
   - `Uri::parse(url)` で URL をパースする。失敗時は元の URL をそのまま返す（Debug 出力用であり、パースエラーを伝播させる必要はない。ただしパース失敗時は旧実装と同様の誤マスクが発生しうることを認識する）。
   - `uri.authority()` から authority 文字列を取得する。`None` の場合は userinfo なしとして元の URL をそのまま返す。
   - authority 文字列に対して `rfind('@')` で userinfo の有無を判定する。`rfind` を使うのは、userinfo 内に `:` が含まれるケース（`user:pass@host`）に対応するためであり、RFC 3986 の authority = `[userinfo "@"] host [":" port]` に準拠する。authority 内の `@` のみを検索対象とするため、クエリやフラグメントに含まれる `@` は誤検出されない。
   - userinfo がある場合、`uri.scheme()` + `"://<redacted>@"` + `&authority[at_pos+1..]` (host:port 部) + `uri.path()` + `format!("?{}", query)` + `format!("#{}", fragment)` でマスク済み URL を再構築する。`uri.query()` / `uri.fragment()` の戻り値には区切り文字（`?` / `#`）が含まれないため、`Option` が `Some` の場合のみ手動で付与する。

3. **`is_turn_tcp_or_udp_url`:**
   - `Uri::parse(url)` で URL をパースする。失敗時は `false` を返す。
   - `uri.scheme()` が `"turn"` または `"turns"` (case-insensitive) かをチェックする。
   - `uri.query()` でクエリ文字列を取得し、`split('&')` と `strip_prefix("transport=")` で `transport` パラメータを探す。クエリは `Uri::parse` によって正しく切り出されるが、個別パラメータの抽出は引き続き手動で行う。
   - `transport` 値の比較は **case-insensitive** (`eq_ignore_ascii_case`) で行う。現行の実装は `url.to_ascii_lowercase()` で URL 全体を小文字化してから比較しているが、`Uri::parse` は生のクエリ文字列を返す。`#[cfg(test)]` 内のヘルパーであり、既存のテストコードの URL はすべて小文字のため既存テストは壊れないが、大文字混じりの `?Transport=TCP` にも正しくマッチさせる。

4. **TURN URI の注意点:** TURN URI 形式 (`turn:host:port?transport=tcp`) は `://` を含まないため、`Uri::parse` の結果として `uri.authority()` / `uri.host()` は `None` になる。`is_turn_tcp_or_udp_url` は `scheme()` と `query()` のみを参照するため問題ない。

5. 単体テストを `src/types.rs` の `#[cfg(test)] mod tests` 内に追加する:
   - `http://user:pass@host` (userinfo あり)
   - `http://user:pass@host/path` (userinfo あり + path)
   - `http://host` (userinfo なし)
   - `http://host?q=x@y` (クエリ内 `@`、マスクされないこと)
   - `http://host#frag@evil` (フラグメント内 `@`、マスクされないこと)
   - `http://host/path?q=x@y` (path + クエリ内 `@`)
   - `http://user@host` (user のみ)
   - `http://host?token=abc@evil.com` (バグ再現ケース)
   - `http://user:pass@host:8080/path` (ポート番号あり)
   - `rtsp://admin:12345@camera.example.com:554/stream` (http 以外のスキーム)
   - スキームなしの URL はそのまま返すこと
   - 不正な URL はそのまま返すこと

## 解決方法

手動の URL 文字列パースを `shiguredo_http11::uri::Uri::parse` に置き換えた。
- `mask_url_userinfo`: `Uri::parse` で authority を正しく分離し、authority 内の `@` のみをマスク対象とする。クエリやフラグメント内の `@` は誤検出されない。
- `is_turn_tcp_or_udp_url`: `Uri::parse` でスキームとクエリを取得し、`transport` パラメータの抽出は case-insensitive で行う。
- 各種 URL パターンに対する単体テストを追加した。

## 完了条件

- `mask_url_userinfo` が `shiguredo_http11::uri::Uri::parse` を使用して実装されている。
- クエリ内やフラグメント内の `@` が userinfo としてマスクされない。
- `is_turn_tcp_or_udp_url` が `Uri::parse` を使用して実装されている。
- `mask_url_userinfo` の単体テストが全ケース通る。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
