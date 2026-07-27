# AMF デコーダの `alloc_buffer` 直後の生ポインタに null チェックを追加する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-27
- Model: Opus 4.7
- Branch: feature/fix-amf-decoder-null-ptr-check
- Polished: {YYYY-MM-DD}

## 目的

AMF デコーダの `decode` 経路で、`alloc_buffer` から得た生ポインタに null チェックが入っていないため、null 時に `std::ptr::copy_nonoverlapping` に null を渡して UB (未定義動作) を引き起こしている。エンコード側と同じ堅牢性を持たせる。

## 優先度根拠

High。UB は最悪プロセスクラッシュや任意コード実行につながる。デコードはリモートから送られてくる映像フレーム経路であり、悪意ある / 破損したパケットの受信で発火し得る。エンコード側 (`amf.rs:155-161`) には null / size 0 チェックが既に入っているのに対して、デコード側だけが対称性を欠いている。

## 現状

`src/video_codecs/amf.rs:691-703` にデコード側の実装がある:

```rust
let ptr = buffer.get_native() as *mut u8;
let size = buffer.get_size();
if encoded_bytes.len() > size {
    rtc_log_error!(
        "AMF decoder: encoded data size {} exceeds buffer size {}",
        encoded_bytes.len(),
        size
    );
    return VideoCodecStatus::Error;
}
unsafe {
    std::ptr::copy_nonoverlapping(encoded_bytes.as_ptr(), ptr, encoded_bytes.len());
}
```

一方エンコード側の callback (`amf.rs:155-161`) では:

```rust
let ptr = buffer.get_native() as *const u8;
let size = buffer.get_size();
if ptr.is_null() || size == 0 {
    rtc_log_error!("AMF encode callback: buffer is null or empty");
    return;
}
```

と null / size 0 のガードがある。デコード側は同種の防御がなく、AMF 側が null や無効ポインタを返した場合、直後の `copy_nonoverlapping` が UB を起こす。

## 設計方針

`amf.rs:691` の直後 (`let ptr = buffer.get_native() as *mut u8;` の直後) に以下を追加する:

```rust
if ptr.is_null() {
    rtc_log_error!(
        "AMF decoder: alloc_buffer returned null pointer for {:?}",
        self.codec_type
    );
    return VideoCodecStatus::Error;
}
```

`size` 側は既存の `if encoded_bytes.len() > size` チェックで実質カバーされているため、そちらは変更しない。エンコード側の `size == 0` 追加チェックとの対称性は、テスト観点で明示的に別対応するのが望ましいが、本 issue のスコープからは外す。

## 完了条件

- `src/video_codecs/amf.rs` のデコード側で、`alloc_buffer` の戻り値の `get_native()` に対する null チェックが追加されている。
- null 時にプロセスがクラッシュせず `VideoCodecStatus::Error` を返す。
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace` が通る。

## 解決方法

null チェックの追加は行わず、現状のままとした。理由は以下の通り：

- `shiguredo_amf::Context::alloc_buffer()` (`shiguredo_amf-2026.3.0/src/amf.rs:654-658`) が `AMFContext::AllocBuffer` 呼び出し後の `*ppBuffer` に対して既に null チェックを実装しており、AMF が `AMF_OK` を返しながら `*ppBuffer` に null を設定した場合でも `Err` として返る。
- したがって `alloc_buffer` が `Ok(Buffer)` を返した時点で内部の AMFBuffer ポインタは非 null であることが保証されており、後続の `get_native()` に対してデコード側で追加の null チェックは不要。
- なおエンコード側の `amf.rs:155-161` にある null チェックは `shiguredo_amf` の `extract_encoded_output()` (`encode.rs:1087-1091`) が既に同様のチェックを行っており冗長であるため、別途 issue 0083 で除去する。
