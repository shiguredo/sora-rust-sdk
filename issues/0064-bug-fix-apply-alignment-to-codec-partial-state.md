# `apply_alignment_to_codec` の simulcast stream 部分 align 状態不整合を解消する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-apply-alignment-to-codec-partial-state
- Polished: 2026-07-27

## 目的

`apply_alignment_to_codec` は simulcast の一部 stream で align 失敗した場合に `continue` で「元のサイズのまま」放置する。トップレベルの `codec.width()` / `height()` は `set_width` / `set_height` の呼び出しが先に行われる（`video_codec.rs:177-178`）ため、stream ループ内で align 失敗しても codec 本体だけ align 済みという不整合状態が残る。全 stream align する / 失敗時は関数全体を無効化するのいずれかに揃える。

本 issue は #0063 （`align_down` の `Option<i32>` 化）の完了を前提とする。#0063 で `align_down` が `Option<i32>` になると、simulcast stream ループ内の既存チェック `aligned_stream_width <= 0` が `Option` 型に対して機能しなくなり、部分状態不整合のリスクが顕在化する。

## 優先度根拠

High。#0063 と複合すると、下流エンコーダに非アライン解像度が伝搬してエンコード失敗または破損の可能性がある。simulcast は sora-rust-sdk の主要機能で、実運用で頻繁に使われる。

## 現状

`src/video_codec.rs:161-194`:

```rust
fn apply_alignment_to_codec(
    codec: &mut VideoCodec,
    codec_type: VideoCodecType,
    horizontal_alignment: i32,
    vertical_alignment: i32,
) -> Option<(i32, i32)> {
    if codec.codec_type() != codec_type {
        return None;
    }

    let aligned_codec_width = align_down(codec.width(), horizontal_alignment);
    let aligned_codec_height = align_down(codec.height(), vertical_alignment);
    if aligned_codec_width <= 0 || aligned_codec_height <= 0 {
        return None;
    }

    codec.set_width(aligned_codec_width);       // ← 先にトップレベルを mutate
    codec.set_height(aligned_codec_height);     // ← 先にトップレベルを mutate

    for index in 0..codec.number_of_simulcast_streams() {
        let Some(mut stream) = codec.simulcast_stream(index) else {
            continue;
        };
        let aligned_stream_width = align_down(stream.width(), horizontal_alignment);
        let aligned_stream_height = align_down(stream.height(), vertical_alignment);
        if aligned_stream_width <= 0 || aligned_stream_height <= 0 {
            continue;   // ← この時点でトップレベルはすでに mutate 済み
        }
        stream.set_width(aligned_stream_width);
        stream.set_height(aligned_stream_height);
    }

    Some((aligned_codec_width, aligned_codec_height))
}
```

- トップレベル codec の `set_width` / `set_height` が stream ループの **前** にある。
- stream ループで `continue` してもトップレベルだけ align 済みのままになる。
- #0063 で `align_down` が `Option<i32>` に変わると、`aligned_stream_width <= 0` のチェックが型不一致になり、`Option` としての適切なハンドリングが必要になる。

## 設計方針

**案 A（推奨・採用）: 全要素を事前検証してから一括で適用する**

1. トップレベル codec と全 simulcast stream の align 結果を **先にすべて計算** する。
   - `align_down(codec.width(), ...)` → `Option<i32>`
   - `align_down(codec.height(), ...)` → `Option<i32>`
   - 各 stream についても同様に計算。
2. いずれか 1 つでも `None` があれば、**codec に対して何も変更せずに** `None` を返す。
3. すべてが `Some` の場合のみ、`set_width` / `set_height` を **一括で適用** する。
4. この設計により「トップレベルだけ align 済み、stream が非アライン」という部分状態が発生しない。
5. `init_encode`（`video_codec.rs:281`）は `codec.to_owned()` でコピーを作成してから `apply_alignment_to_codec` を呼ぶ。このため現在の partial mutation の影響はコピー内に留まり、元の codec は破壊されない。また `None` 返却時の `VideoCodecStatus::Error` 返却は #0063 で実装済みのため、本 issue では `apply_alignment_to_codec` の mutation 順序修正に集中する。

**案 B および案 C は採用しない。**

- 案 B（align 失敗 stream だけ無効化）: 「無効化」の意味（幅高さ 0 設定）が下流エンコーダに対して安全である保証がない。`None` 返却の方が呼び出し側が明示的にエラーを処理できる。
- 案 C（非アライン値をトップレベル値で埋める）: フォールバック値の選択基準があいまいで、単純な `None` 返却に比べてメリットがない。

## 完了条件

- `apply_alignment_to_codec` の mutation 順序が修正され、全要素の事前検証 → 一括適用の順になっている。
- simulcast stream の 1 本だけ align 失敗するケース（例: codec=320x180, stream=15x10, alignment=16）で `apply_alignment_to_codec` が `None` を返し、codec が変更されない。
- 全要素が align 成功するケースで従来通り `Some((w, h))` が返る。
- 上記の両ケースの単体テストが追加されている（`src/video_codec.rs` 内の `#[cfg(test)] mod tests`）。
- 既存テスト `alignment_updates_codec_and_simulcast_streams` の動作が維持されている（必要に応じて改修する）。
- #0063 の完了を前提とし、本 issue の実装は #0063 完了後（`align_down` が `Option<i32>` を返す状態）に行う。`AlignmentEncoderAdapter::init_encode` の `None` 返却処理は #0063 で実装済みのため、本 issue では `apply_alignment_to_codec` の mutation 順序修正に集中する。
- `cargo test --workspace --all-features` が通る。
- `cargo fmt --all -- --check` が通る。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
