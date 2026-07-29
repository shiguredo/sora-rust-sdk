# libcamera 停止時のバッファ寿命を保証する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-libcamera-shutdown-lifetime
- Polished: {YYYY-MM-DD}

## 目的

libcamera の停止完了と native frame の返却を保証してから allocator と FrameBuffer を破棄し、停止時のクラッシュや無効な DMA-BUF 参照を防ぐ。

## 優先度根拠

High。停止失敗を無視したままリソースを破棄する経路があり、解放済み FrameBuffer へのアクセスによる SIGSEGV が発生し得る。

## 現状

`run_libcamera_loop` は `camera.stop()` の結果を破棄する。
`LibcameraNativeFrameBuffer` は raw file descriptor と requeue token を保持するが、allocator や camera の寿命を保持しない。
`run_capture_loop` は下流が native frame を保持しているか確認せず終了できる。

## 設計方針

- `camera.stop()` の失敗を呼び出し側へ伝播する
- outstanding native frame を追跡し、停止時に安全に返却を待つ
- native frame に allocator と camera の共有 lifetime guard を持たせる案を検討する
- timeout 時も非同期アクセス終了の保証なしに関連リソースを破棄しない

## 完了条件

- stop 失敗が成功として扱われない
- native frame の利用中に allocator と FrameBuffer が破棄されない
- stop と frame 返却が競合する実機テストがある
- start、stop、再 start を繰り返してクラッシュや file descriptor error が発生しない
