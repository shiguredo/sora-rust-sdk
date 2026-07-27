# AMF エンコードコールバック内の冗長な null チェックを除去する

- Priority: Low
- Created: 2026-07-27
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-remove-redundant-null-check-from-amf-encode-callback
- Polished: 2026-07-27

## 目的

`src/video_codecs/amf.rs:155-161` の `handle_amf_encode_callback` 内にある null チェックは `shiguredo_amf` 側の `extract_encoded_output()` が既に行っており冗長であるため、除去する。

## 優先度根拠

Low。冗長な防御コードであり、現状誤動作はなく、除去しなくても動作上の問題はない。ただし無駄なコードを残すと将来の読者が「なぜここだけ二重チェックしているのか」と混乱する可能性がある。

## 現状

`src/video_codecs/amf.rs:155-161`:

```rust
let buffer = encoded.buffer();
let ptr = buffer.get_native() as *const u8;
let size = buffer.get_size();
if ptr.is_null() || size == 0 {
    rtc_log_error!("AMF encode callback: buffer is null or empty");
    return;
}
let data = unsafe { std::slice::from_raw_parts(ptr, size) };
```

一方、このコードに到達する前に `shiguredo_amf` の `extract_encoded_output()` (`shiguredo_amf-2026.3.0/src/encode.rs:1067-1095`) が以下を全てチェックしている:

1. `data as *mut AMFBuffer` が null でないこと (encode.rs:1072)
2. `buffer.get_size()` が 0 でないこと (encode.rs:1080)
3. `buffer.get_native()` が null でないこと (encode.rs:1087)

いずれかのチェックに引っかかった場合、`extract_encoded_output()` は `Err` を返し、`handle_amf_encode_callback` 内の `match result { Err(e) => { ... return; } }` (amf.rs:141-145) で処理されるため、amf.rs:155 以降には到達しない。

## 設計方針

1. `amf.rs:158-161` の `if ptr.is_null() || size == 0 { ... return; }` ブロックを削除する。`Ok(EncodedFrame)` に到達した時点で `Buffer` のポインタ非 null・サイズ非 0 は `extract_encoded_output()` によって検証済みであり、同一オブジェクトに対する再チェックは不要である。
2. 後続の `std::slice::from_raw_parts(ptr, size)` はそのまま残す（`ptr` / `size` の取得は残す）。

## 完了条件

- `amf.rs:158-161` の null / size 0 チェックブロックが除去されている。
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace --all-features` が通る。
