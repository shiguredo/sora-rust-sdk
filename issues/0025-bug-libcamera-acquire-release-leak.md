# libcamera の `camera.acquire()` 後の早期 return で `release()` / `stop()` が漏れる

- Priority: High
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-libcamera-acquire-release-leak
- Polished: {YYYY-MM-DD}

## 目的

`src/libcamera.rs:378` の `run_libcamera_loop` は L405 で `camera.acquire()` を呼んだ後、設定生成・バッファ確保・ストリーム開始など多数の処理を `?` 演算子で連結している。これらが失敗して早期 return した場合、唯一の `camera.release()` 呼び出し (L663) と `camera.stop()` 呼び出し (L662) は実行されない。libcamera は acquire したカメラを release しないと次回 acquire が拒否される実装であり、SDK プロセスを再起動するまでカメラが使えなくなる。

## 優先度根拠

High。

- libcamera リソースは acquire 中はプロセス間で排他されており、release を漏らすと同じプロセスからも他プロセスからもカメラを取得できなくなる
- 早期 return は実運用で発生し得る (設定検証失敗・stride 取得失敗・FrameBuffer 確保失敗・request 構築失敗等)
- 修正規模は限定的 (RAII ガードの追加または関数分割)
- 単発の異常系を契機にカメラが完全に使えなくなる体験は、組み込み用途 (Raspberry Pi 等) で致命的

## 現状

`src/libcamera.rs:405-663` の構造:

```rust
camera.acquire()?;

// 以下、? で早期 return し得る経路が多数:
let mut camera_config = camera.generate_configuration(&[StreamRole::VideoRecording])?;     // L408
{
    let mut stream_config = camera_config.at(0)?;                                          // L412
    ...
}
let mut status = camera_config.validate();
if status.is_err() {
    {
        let mut stream_config = camera_config.at(0)?;                                      // L423
        ...
    }
    status = camera_config.validate();
}
let status = status?;                                                                       // L429
camera.configure(&mut camera_config)?;                                                      // L431

let (width, height, stride, frame_pixel_format) = {
    let stream_config = camera_config.at(0)?;                                              // L434
    ...
    let frame_pixel_format = match pixel_format.fourcc {
        ...
        _ => {
            return Err(Error::LibcameraMessage { ... });                                   // L441-444
        }
    };
    ...
};

let stream = {
    let stream_config = camera_config.at(0)?;                                              // L464
    stream_config.stream().ok_or_else(|| Error::LibcameraMessage { ... })?                 // L467-469
};

let allocator = FrameBufferAllocator::new(&camera);
let buffer_count = allocator.allocate(&stream)?;                                            // L473

...
let stride_i32 = i32::try_from(stride).map_err(|_| Error::LibcameraMessage { ... })?;       // L499

for index in 0..buffer_count {
    let buffer = allocator.get_buffer(&stream, index)?;                                    // L510
    let frame_buffer_layout = collect_frame_buffer_layout(&buffer)?;                       // L511
    ...
    let request = camera.create_request(index as u64)?;                                    // L522
    request.add_buffer(&stream, &buffer)?;                                                 // L523
    ...
}

camera.start()?;                                                                            // L528

for request in &requests {
    camera.queue_request(request)?;                                                        // L531
}

// メインループ ...

let _ = camera.stop();                                                                      // L662
let _ = camera.release();                                                                   // L663
```

問題:

- L405 の `acquire()` 成功後、L408 以降で `?` を踏んで return すると `release()` が呼ばれない
- L528 の `start()` 成功後、L531 で `?` を踏んで return すると `stop()` も `release()` も呼ばれない
- 正常終了経路 (L545 以降のループ break) でしか `stop()` / `release()` に到達しない

`shiguredo_libcamera::Camera` の Drop 実装が release を肩代わりしているかどうかは未確認だが、現状の正常終了で明示的に release を呼んでいる事実から、Drop には依存していない設計と推測される (要確認)。

## 設計方針

選択肢は 2 系統あり、いずれかを採用する:

### 方針 A: RAII ガード型を追加する

`AcquiredCamera` のような所有権付きラッパー型を `src/libcamera.rs` 内に追加する:

- `Camera::acquire()` 成功時にこのガードを構築する
- ガードが Drop されるときに `started` フラグを見て必要なら `stop()` を呼び、続けて `release()` を呼ぶ
- `start()` / `stop()` はガード経由でのみ呼ぶ。ガードが `started: bool` を内部に保持する
- 早期 return 時はガードのスコープが切れて自動的に `stop()` + `release()` が走る

メリット: スコープと整合し、追加の経路 (新しい `?` の挿入) でも安全。
デメリット: 構造体追加とメソッド設計が必要。`Camera` の他のメソッド (`generate_configuration` 等) をガード経由で呼ばせるか、内側の参照 `&Camera` を露出するかの API 設計が要る。

### 方針 B: 関数分割で acquire/release をペアにする

`run_libcamera_loop` の中身を内側関数 (`run_libcamera_loop_inner` 等) に分離し、外側で acquire/release を呼ぶ:

```rust
fn run_libcamera_loop(...) -> Result<()> {
    let manager = CameraManager::new()?;
    ...
    let mut camera = manager.get_camera(camera_index as usize)?;
    camera.acquire()?;
    let result = run_libcamera_loop_inner(&mut camera, ...);
    let _ = camera.stop();   // start 済みでなくても無害なら無条件で呼ぶ
    let _ = camera.release();
    result
}

fn run_libcamera_loop_inner(camera: &mut Camera, ...) -> Result<()> {
    // 早期 return しても外側で release される
    ...
}
```

メリット: 構造体追加なし、変更が局所的。
デメリット: `stop()` を `start()` していない状態で呼んで安全かを libcamera 側で確認する必要がある (安全でない場合は started フラグ管理が必要)。

どちらを採るかは実装フェーズで判断する。`shiguredo_libcamera` の API ドキュメントや実装を確認したうえで決める。

## 完了条件

- `camera.acquire()` 成功以降の任意の早期 return 経路で `release()` (および必要なら `stop()`) が呼ばれることが、コードを読んで自明に分かる構造になっている
- 異常系を意図的に発生させた状態 (例: 不正な camera_index ではない別の失敗注入) で、当該プロセスが再度 libcamera キャプチャを開始できる
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る (libcamera feature を含む)

## 解決方法

1. `shiguredo_libcamera::Camera::stop()` / `release()` の事前条件を確認する (acquire のみで stop を呼んでも安全か、start 済みでないと UB か)
2. 上記方針 A または B のうち、`shiguredo_libcamera` の API と相性が良い方を選ぶ
3. 実装する
4. 異常系のテストは libcamera 実機が必要なため、最小限の単体テスト (RAII ガードを採るなら Drop が release を呼ぶことをモック越しに……ではなく、`Camera` を本物で呼べる範囲で確認するか、コードレビューで担保) + e2e の手動確認で完了とする
   - 「モックやスタブは絶対に利用しないこと」(AGENTS.md) のため、libcamera を持たない CI 環境では実機テストは不可。修正自体はコードレビューと実機での手動確認で担保する
