# libcamera の `camera.acquire()` 後の早期 return で `release()` / `stop()` が漏れる

- Priority: High
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-libcamera-acquire-release-leak
- Polished: 2026-06-25

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 (video codec 層の致命的バグ修正) のうち「`libcamera.rs` の `acquire()` 後リソースリーク」分。

## 目的

`src/libcamera.rs:378` の `run_libcamera_loop` は L405 で `camera.acquire()` を呼んだ後、設定生成・バッファ確保・ストリーム開始など多数の処理を `?` 演算子で連結している。これらが失敗して早期 return した場合、唯一の `camera.release()` 呼び出し (L663) と `camera.stop()` 呼び出し (L662) は実行されない。libcamera は acquire したカメラを release しないと次回 acquire が拒否される実装であり、SDK プロセスを再起動するまでカメラが使えなくなる。

## 優先度根拠

High。

- libcamera リソースは acquire 中はプロセス間で排他されており、release を漏らすと同じプロセスからも他プロセスからもカメラを取得できなくなる
- 早期 return は実運用で発生し得る (設定検証失敗・stride 取得失敗・FrameBuffer 確保失敗・request 構築失敗等)
- 修正規模は限定的 (関数分割)
- 単発の異常系を契機にカメラが完全に使えなくなる体験は、組み込み用途 (Raspberry Pi 等) で致命的

## 現状

`src/libcamera.rs:405-663` の `run_libcamera_loop` は `camera.acquire()?` (L405) 成功後、以下の全 16 経路で `?` 演算子または明示的 `return Err(...)` により早期 return し得る。いずれの経路でも L662 `camera.stop()` と L663 `camera.release()` には到達しない:

| # | 行番号 | 早期 return の原因 |
|---|--------|--------------------|
| 1 | L408 | `camera.generate_configuration()?` 失敗 |
| 2 | L412 | `camera_config.at(0)?` 失敗 (1 回目) |
| 3 | L423 | `camera_config.at(0)?` 失敗 (NV12 フォールバック時) |
| 4 | L429 | `status?` (validate 失敗) |
| 5 | L431 | `camera.configure()?` 失敗 |
| 6 | L434 | `camera_config.at(0)?` 失敗 (stream config 再取得時) |
| 7 | L441-444 | 非対応 pixel format による `return Err(...)` |
| 8 | L464 | `camera_config.at(0)?` 失敗 (stream 取得時) |
| 9 | L467-469 | `stream_config.stream()` が `None` |
| 10 | L473 | `allocator.allocate(&stream)?` 失敗 |
| 11 | L499 | `i32::try_from(stride)` 変換失敗 |
| 12 | L510 | `allocator.get_buffer(&stream, index)?` 失敗 |
| 13 | L511 | `collect_frame_buffer_layout(&buffer)?` 失敗 |
| 14 | L514 | `build_mapped_frame_buffer_planes(&frame_buffer_layout)?` 失敗 |
| 15 | L522 | `camera.create_request(index as u64)?` 失敗 |
| 16 | L523 | `request.add_buffer(&stream, &buffer)?` 失敗 |

加えて:

- **L528 の `camera.start()?` 自体が失敗した場合**も、`acquire()` 後の早期 return であり `stop()` / `release()` が漏れる（`start()` 失敗時、libcamera 内部で全クリーンアップが行われるかは `shiguredo_libcamera` の実装依存であり、未確認）
- **L528 の `start()?` 成功後**、L531 `camera.queue_request(request)?` の 2 回目以降のイテレーションで失敗した場合、`stop()` も `release()` も漏れる
- 正常終了経路 (ループ break) でしか `stop()` / `release()` に到達しない

`shiguredo_libcamera::Camera` の Drop 実装 (`libcamera-rs/src/camera.rs:168-172`) は `lc_camera_release_ref` を呼ぶのみで、これは C++ ラッパー構造体 `lc_camera` の `delete` (`libcamera-rs/c-api/camera.cpp:19-21`) に相当する。`delete` は内部の `std::shared_ptr<Camera>` の参照カウントを減らすだけであり、libcamera の `Camera::release()` (パイプラインハンドラの排他ロック解放と状態を `CameraAvailable` に戻す処理) は一切呼ばれない。`Camera::Private::~Private()` (`libcamera/src/libcamera/camera.cpp:597-601`) も状態が `Available` でなければエラーログを出力するのみで、`release()` を呼ぶことはない。

したがって **`Camera::Drop` は `release()` を肩代わりしない**。明示的な `release()` 呼び出しは必須である。

また `Camera::stop()` の内部実装 (`libcamera/src/libcamera/camera.cpp:1453-1454`) は `isRunning()` でなければ即座に `0` を返す no-op であり、`start()` 前の `stop()` 呼び出しは安全である。`stop()` → `release()` の呼び出し順序は、running 状態での `release()` の挙動が未確認であるため安全側に倒した順序とする。

注: 上記の行番号は issue 作成時点 (`src/libcamera.rs` が最後に変更されたコミット `2e53faa` 時点) のものである。

## 設計方針

関数分割で acquire/release をペアにする方法を採用する。

`run_libcamera_loop_inner` を内側関数として導入し、外側で `acquire()` → 内側呼び出し → `stop()` → `release()` の順に実行する。内側関数が早期 return しても必ず外側で `stop()` / `release()` が呼ばれる。

`Camera::stop()` は `isRunning()` でなければ no-op であることが確認されたため (`libcamera/src/libcamera/camera.cpp:1453-1454`)、`start()` 前の無条件呼び出しは安全。`stop()` の後に `release()` を呼ぶ順序とする（running 状態での `release()` の挙動が未確認のため安全側に倒す）。

### 関数分割の構造

```rust
fn run_libcamera_loop(
    source: AdaptedVideoTrackSource,
    camera_index: u32,
    width: i32,
    height: i32,
    native_frame_output: bool,
    controls: Vec<(String, String)>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let manager = CameraManager::new()?;
    if manager.cameras_count() == 0 {
        return Err(Error::LibcameraMessage { message: "camera was not found".to_string() });
    }
    if camera_index >= manager.cameras_count() as u32 {
        return Err(Error::LibcameraMessage {
            message: format!("camera index is out of range: index={} count={}", camera_index, manager.cameras_count()),
        });
    }

    let mut camera = manager.get_camera(camera_index as usize)?;
    camera.acquire()?;

    let result = run_libcamera_loop_inner(
        &mut camera,
        source,
        width,
        height,
        native_frame_output,
        controls,
        stop,
    );

    let _ = camera.stop();
    let _ = camera.release();
    result
}

fn run_libcamera_loop_inner(
    camera: &mut Camera,
    source: AdaptedVideoTrackSource,
    width: i32,
    height: i32,
    native_frame_output: bool,
    controls: Vec<(String, String)>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    // camera.acquire()? 以降の全処理をここに移す
    // 早期 return しても外側で stop() + release() される
}
```

注: `FrameBufferAllocator::new(camera)` や `camera.on_request_completed(...)`、`camera.create_request(...)`、`camera.start()?`、`camera.queue_request(...)` はすべて `&Camera` / `&mut Camera` で動作する。現行コードが問題なくコンパイル可能であることから、`FrameBufferAllocator::new()` は Rust の借用を持ち越さず内部的にハンドルを複製していると判断できる。したがって、`FrameBufferAllocator` や `on_request_completed` のクロージャ（内部の mpsc channel）は内側関数内で生成すればよく、外側で事前に確保する必要はない。

### 制約: パニック安全性

関数分割によるアプローチでは、内側関数で panic が発生した場合、スタック巻き戻しにより外側の `camera.stop()` / `camera.release()` も実行されず、カメラリソースがリークする。現行コードも同様の問題を抱えているが、本 issue のスコープではこの制約を受け入れる。panic 経路の保護（`std::panic::catch_unwind` 等）が必要になった場合は別 issue で対応する。

## 完了条件

- `camera.acquire()?` の成功から `camera.release()` の呼び出しまでが単一の関数（外側の `run_libcamera_loop`）の字面の範囲内に収まり、中間に `return` 文や `?` による早期脱出経路が存在しないこと
- `camera.start()?` の成功から `camera.stop()` の呼び出しまでが上記と同様の構造であること
- 内側関数 `run_libcamera_loop_inner` の任意の `?` または `return Err(...)` による早期 return が発生しても、外側で必ず `camera.stop()` と `camera.release()` が呼ばれることがコードレビューで確認できること
- 上記「現状」に列挙した 16 経路 + `camera.start()?` 失敗（計 17 経路）すべてについて、早期 return 後に `stop()` / `release()` が呼ばれることをコードレビューで網羅的に確認できること
- `camera.release()` が失敗した場合に `rtc_log_error!` でエラーログを出力すること（現状は `let _ =` で握りつぶしている）
- `src/libcamera.rs:1313-1497` の既存単体テスト (`copy_i420_planes_to_buffer` / `copy_nv12_planes_to_buffer` / `native_frame_buffer` の crop_and_scale と requeue 通知 / ビルダーのデフォルト値 / `CapturedFrameBuffers` のバリアント排他性) が修正後もすべて通過すること
- 実機確認: `camera.release()` 漏れを誘発する失敗注入（例: 非対応の pixel format を返すカメラ、`camera.start()` 成功後の `queue_request()` 失敗）を意図的に発生させた状態で、プロセスを再起動せずに `LibcameraVideoCapturer::stop()` → `start()` → `stop()` のサイクルが成功すること
- 公開 API (`LibcameraVideoCapturer` / `LibcameraVideoCapturerBuilder`) のシグネチャと振る舞いに変更がないこと
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る (libcamera feature を含む)

## 解決方法

1. `run_libcamera_loop_inner` 関数を新設し、`run_libcamera_loop` の `camera.acquire()?` 以降の全処理を移す
   - 内側関数は `fn`（プライベート）とする
   - 引数は設計方針に記載の 8 つ: `camera`, `source`, `width`, `height`, `native_frame_output`, `controls`, `stop`（戻り値 `Result<()>`）
2. `run_libcamera_loop` の外側で `camera.acquire()?` → 内側関数呼び出し → `camera.stop()` → `camera.release()` の順に実行する
3. `camera.release()` の戻り値を `let _ =` から `if let Err(err) = camera.release() { rtc_log_error!(...) }` に変更する
4. 既存単体テスト (`src/libcamera.rs:1313-1497`) が通過することを確認する
5. 異常系の実機確認: `camera.release()` 漏れを誘発する失敗注入（非対応 pixel format カメラ、`start()` 後の `queue_request()` 失敗等）を実施する。libcamera 実機が必要で、モックやスタブは不可 (AGENTS.md)
6. `CHANGES.md` に `[FIX] libcamera の acquire() 後の早期 return で release() / stop() が漏れる問題を修正する` エントリを追記する
7. `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通ることを確認する
