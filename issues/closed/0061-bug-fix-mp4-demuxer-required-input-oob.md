# MP4 デマルチプレクサの `required_input` が示す位置がファイルサイズを超えた場合の panic を解消する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-28
- Model: Opus 4.7
- Branch: feature/fix-mp4-demuxer-required-input-oob
- Polished: 2026-07-27

## 目的

`Mp4SampleReader::new` が壊れた MP4（truncated / ボックスヘッダ破損により `box_size` が異常値）を受け取ったとき、`demuxer.required_input()` が返す `RequiredInput.position` が `file_data.len()` を超え、`file_data[start..end]` のスライスで panic する経路がある。`Result` を返す関数内で panic するのは API 契約違反のため、即座に Err を返すよう修正する。

なお、`stco` / `co64` の不正オフセットによるサンプルデータ範囲外アクセスは `get_sample` 側の panic であり、本 issue の対象ではない（#0062 で対応する）。

## 優先度根拠

High。`Mp4SampleReader::new` は失敗を `Result` で返す公開 API 経路であり、panic は API 契約違反。ユーザーは正常にエラーハンドリングできず、キャプチャースレッド上で発生した場合はプロセスごとクラッシュする。

## 現状

`src/video_codecs/mp4.rs:185-196` の `new_inner` 内で、`demuxer.required_input()` の結果を無防備にスライスしている:

```rust
let start = required.position as usize;
let end = match required.size {
    Some(size) => (start + size).min(file_data.len()),
    None => file_data.len(),
};
let data = &file_data[start..end];
```

- `start > file_data.len()` の場合、スライスの左端がすでに範囲外で panic。
- `end` は `.min(file_data.len())` で頭打ちだが、`start` 側の検証がない。

## 設計方針

1. `start` の境界チェックを `end` 計算より先に行う。`start > file_data.len()` であれば `end` の計算（`start + size`）をせず即座に `Err` を返す。`start == file_data.len()` の場合のみ `get(start..)` で空スライスを取得する（後述）。
2. `start` が範囲内であれば `file_data.get(start..end)` でスライスを取得する。`get()` が `None` を返した場合、即座に `Err` を返す。
   - **空スライスを `handle_input` に渡してエラー遷移させる方式は採用しない**。`handle_input` が空スライスを受け取った際の挙動は保証されておらず、`required_input()` が `None` を返さず無限ループに陥る可能性があるため。
3. `end` 計算時の `start + size` 加算は `usize` オーバーフローを考慮し、`start.checked_add(size)` または `saturating_add` で安全に計算する。ただし、手順 1 で `start <= file_data.len()` を確認済みであれば `start + size` がオーバーフローすることは事実上ないため、簡略化も可。
4. 新たに `Mp4Error::InputPositionOutOfRange` バリアントを追加する:
   - フィールド: `position: u64` と `file_size: usize`
   - `Display` 実装: `"入力位置がファイルサイズ範囲外です: position={position}, file_size={file_size}"`（既存の `Mp4Error::Display` の日本語メッセージパターンに合わせる）
   - `Error::source()` アーム: 既存の `NoVideoTrack | NoVideoSamples | UnsupportedVideoCodec | InvalidNalLengthSize(_) => None` のアームに追記する
   - 上位の `Error::Mp4` への変換は既存の `From<Mp4Error> for Error` 実装が `err.to_string()` で `reason` を生成するため、新規バリアントの追加のみで伝搬する
5. `get_sample` 側の同種パニックは #0062 で対応する。`required_input` ループと `get_sample` では呼び出し元のコンテキスト（構築時 vs 再生時）とエラー伝搬経路（`Result` return vs panicking thread）が異なるため、独立した issue として扱う。
6. 呼び出し側で panic には至らないことを、単体テストで担保する。

## 解決方法

`Mp4SampleReader::new_inner` の `required_input()` 結果のスライス操作前に境界チェックを追加した。
- `start > file_data.len()` の場合、加算を行わず即座に `Mp4Error::InputPositionOutOfRange` を返す。
- `file_data.get(start..end)` で安全にスライスを取得し、`None` の場合も同様にエラーを返す。
- `Mp4Error::InputPositionOutOfRange` バリアントを新設し、`Display` と `Error::source()` に対応させた。
- truncated MP4 フィクスチャを用いた単体テストを追加した。

## 完了条件

- `demuxer.required_input()` の `position` がファイルサイズを超える MP4 を渡しても panic せず、`sora_sdk::Error::Mp4` として呼び出し側に返る。
- `Mp4Error::InputPositionOutOfRange` バリアントが追加され、`Display` と `Error::source()` の match arm が追加されている。
- `cargo test --workspace` に、truncated MP4 フィクスチャを使った単体テストが追加されている。
  - テストの配置先は `tests/test_mp4.rs`（公開 API のテスト）または `src/video_codecs/mp4.rs` 内の `#[cfg(test)] mod tests`（`new_inner` を直接テストする場合）。`Mp4SampleReader::new` 経由でのテストが現実的であれば `tests/test_mp4.rs` に配置する。
  - テストフィクスチャは既存 H.264 フィクスチャ (`testdata/archive-red-320x320-h264.mp4`) を `include_bytes!` で読み込み、ftyp ボックスもしくは moov ボックスヘッダの `box_size` を書き換えて巨大な値にする方式を基本とする。切り詰め方式も併用してよい。
- `cargo fmt --all -- --check` が通る。
- `cargo clippy --workspace --all-features -- -D warnings` が通る。
