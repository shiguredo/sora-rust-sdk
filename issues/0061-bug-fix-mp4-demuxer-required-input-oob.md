# MP4 デマルチプレクサの `required_input` が示す位置がファイルサイズを超えた場合の panic を解消する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-mp4-demuxer-required-input-oob
- Polished: {YYYY-MM-DD}

## 目的

`Mp4SampleReader::new` が壊れた MP4 (truncated / moov が壊れている / co64 や stco に不正 offset がある等) を受け取ったとき、`demuxer.required_input()` が返す `RequiredInput.position` が `file_data.len()` を超え、`file_data[start..end]` のスライスで panic する経路がある。`Result` を返す関数内で panic するのは仕様として不正なので Err を返すよう修正する。

## 優先度根拠

High。`Mp4SampleReader::new` は失敗を `Result` で返す公開 API 経路であり、panic は API 契約違反。ユーザーは正常にエラーハンドリングできず、キャプチャースレッド上で発生した場合はプロセスごとクラッシュする。

## 現状

`src/video_codecs/mp4.rs:173-184` あたりで、`demuxer.required_input()` の結果を無防備にスライスしている:

```rust
let start = required.position as usize;
let end = match required.size {
    Some(size) => (start + size).min(file_data.len()),
    None => file_data.len(),
};
let data = &file_data[start..end];
```

- `start > file_data.len()` の場合、スライスの左端がすでに範囲外で panic。
- `start + size` の加算自体が `usize` オーバーフローの可能性 (現実的にはほぼ無いが理論上あり得る)。
- `end` は `.min(file_data.len())` で頭打ちだが、`start` 側の検証がない。

## 設計方針

1. `file_data.get(start..end)` を使い、`Option` で失敗を検出する。None なら以下のいずれかで対応:
   - `demuxer.handle_input` に空スライスを渡してデマルチプレクサをエラー状態に遷移させ、次のループで `required_input()` が `None` を返すのを待つ。
   - あるいは即座に `Err(Mp4Error::...)` を返して呼び出し側にエラーを伝える。
2. `start + size` は `checked_add` で加算オーバーフローを検出。
3. どちらの経路でも panic には至らないことを、単体テストで担保する。

## 完了条件

- `demuxer.required_input()` の `position` がファイルサイズを超える MP4 を渡しても panic せず、`Mp4Error` として呼び出し側に返る。
- `cargo test --workspace` に、truncated MP4 フィクスチャを使った単体テストが追加されている。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
