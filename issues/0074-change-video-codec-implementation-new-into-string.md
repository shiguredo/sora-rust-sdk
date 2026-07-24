# `VideoCodecImplementation::new` のシグネチャを `Into<String>` に変更する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/change-video-codec-implementation-new-into-string
- Polished: {YYYY-MM-DD}

## 目的

`VideoCodecImplementation::new(name: &'static str, description: &'static str)` は静的文字列を強制するため、ランタイム値 (GPU 名や実装バージョン等) を含む description を作れず `Box::leak` を強要する。内部は `String` で保持しているため `&'static` に制約する意味がない。シグネチャを `impl Into<String>` に緩和する。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。正式リリース後にシグネチャを緩められないと、外部から `Box::leak` している既存ユーザーへの罠が残り続ける。canary 期に緩めるのは非破壊 (ユーザーは今まで通り `&'static str` を渡せば動く) だが、実質的にシグネチャ変更なので破壊的変更として扱う。

## 現状

`src/video_codec_capability.rs:19-30` 付近:

```rust
pub fn new(name: &'static str, description: &'static str) -> Self {
    Self {
        name: name.to_string(),
        description: description.to_string(),
    }
}
```

同モジュール内に private の `new_internal(name: String, description: String)` が併存しており、JSON デシリアライズ経路 (162 行) では `String` で構築されている。

## 設計方針

1. `pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self` に変更する。
2. 内部の `new_internal` は削除するか、privately を残す。
3. 静的文字列を渡すユーザーは何もしなくても動く (`&'static str: Into<String>`)。
4. rustdoc に「name は unique であること」の記述を追加する (関連: レビュー指摘での「PartialEq が name 一致で判定される」件との整合)。

## 完了条件

- `VideoCodecImplementation::new` に `String` / `&str` / `&'static str` のいずれも渡せる。
- 既存の呼び出し箇所すべてが変更なしでビルドできる。
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace` が通る。
