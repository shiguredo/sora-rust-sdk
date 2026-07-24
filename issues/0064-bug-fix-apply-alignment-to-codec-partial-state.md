# `apply_alignment_to_codec` の simulcast stream 部分 align 状態不整合を解消する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-apply-alignment-to-codec-partial-state
- Polished: {YYYY-MM-DD}

## 目的

`apply_alignment_to_codec` は simulcast の一部 stream で align 失敗した (`aligned_stream_width <= 0` 等) 場合に `continue` で「元のサイズのまま」放置する。トップレベルの `codec.width()` / `height()` は align 済み、simulcast stream だけ非アラインという不整合状態でエンコーダに渡る。全 stream align する / 失敗時は関数全体を無効化するのいずれかに揃える。

## 優先度根拠

High。指摘 0063 (`align_down` 契約破綻) と複合すると、下流エンコーダに非アライン解像度が伝搬してエンコード失敗または破損の可能性がある。simulcast は sora-rust-sdk の主要機能で、実運用で頻繁に使われる。

## 現状

`src/video_codec.rs:180-191` あたり:

```rust
for stream in codec.simulcast_streams_mut() {
    let aligned_stream_width = align_down(stream.width(), horizontal_alignment);
    let aligned_stream_height = align_down(stream.height(), vertical_alignment);
    if aligned_stream_width <= 0 || aligned_stream_height <= 0 {
        continue;   // ← 部分 align 状態が残る
    }
    stream.set_width(aligned_stream_width);
    stream.set_height(aligned_stream_height);
}
```

トップレベル codec 側は既に align 済みで stream だけ元のまま、という状態が発生する。

## 設計方針

以下のいずれかを選択する:

- **案 A (推奨)**: 1 本でも stream で align 失敗した場合は、関数全体で `None` を返して「この preference/frame では align 不能」を上位に伝える。呼び出し側 (`AlignmentEncoderAdapter` 等) がフォールバック処理をする。
- **案 B**: align 失敗した stream だけを無効化する (幅高さ 0 に設定 + `rtc_log_warning!`)。上位はこれを検出してエンコード対象から除外する。
- **案 C**: 「部分 align」を許容せず、失敗した場合は該当 stream の非アライン値を残さず「トップレベル codec と同じ align 済み値」で埋める (フォールバック)。

Issue 0063 の `align_down` を `Option` 返却に変えた上で、案 A を第一候補とし、実装しやすい方を選ぶ。「片方だけ align する」挙動は選ばない。

## 完了条件

- `apply_alignment_to_codec` から下流に「部分 align 状態」が渡らない。
- simulcast stream の 1 本だけ align 失敗するケースの単体テストが追加されている。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
