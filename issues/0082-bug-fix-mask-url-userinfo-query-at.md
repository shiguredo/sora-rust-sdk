# `mask_url_userinfo` がクエリ内 `@` を userinfo として誤マスクする挙動を修正する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-mask-url-userinfo-query-at
- Polished: {YYYY-MM-DD}

## 目的

`mask_url_userinfo` (types.rs:95-115) は「userinfo としての `@`」と「クエリ / フラグメント内の `@`」を `/` の位置だけで判定するため、`/` が存在しない URL でクエリ内に `@` を含むと userinfo と誤検出してオーバーマスクする。境界候補に `?` と `#` も含める。

## 優先度根拠

High。セキュリティ観点で「漏洩を増やす」バグではないが、Debug 出力の URL 情報が壊れる (`http://host?token=abc@def` が `http://<redacted>@def` に変換されるなど)。ユーザーがログから URL 構造を追えなくなり、障害調査が困難になる。issue 0037 の完了条件を実質的に満たすためにも修正が必要。

## 現状

`src/types.rs:95-115` あたり:

```rust
let Some(at_pos) = url_after_scheme.find('@') else { ... };
if let Some(slash_pos) = url_after_scheme.find('/')
    && at_pos > slash_pos
{
    return std::borrow::Cow::Borrowed(url);
}
// 続けて userinfo としてマスクする処理
```

- `http://host?token=abc@evil.com` → `url_after_scheme = "host?token=abc@evil.com"`, `slash_pos = None` のため早期 return されず、`http://<redacted>@evil.com` にオーバーマスクされる。
- `http://user:pass@host` → 正しくマスクされる。
- `http://host/path?q=x@y` → `@` が `/` より後ろで正しくスキップされる。

現状の単体テストは query / fragment 混在ケースが検証されていない。

## 設計方針

1. `?` と `#` も境界候補として `find` し、`at_pos` がそれらいずれかより後ろにあれば userinfo としてはマスクしない (早期 return する)。
2. あるいは `shiguredo_http11::Uri::parse` の結果から `authority` を切り出し、そこで `@` を探す方が堅牢。既存の実装スタイルに合わせて選択する。
3. 単体テストを追加する:
   - `http://user:pass@host` (userinfo あり)
   - `http://user:pass@host/path` (userinfo あり + path)
   - `http://host` (userinfo なし)
   - `http://host?q=x@y` (query 内 @、マスクされないこと)
   - `http://host#frag@evil` (fragment 内 @、マスクされないこと)
   - `http://host/path?q=x@y` (path + query 内 @)
   - `http://user@host` (user のみ)

## 完了条件

- `http://host?token=abc@evil.com` などのクエリ内 `@` が userinfo としてマスクされない。
- 上記の単体テスト全ケースが通る。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
