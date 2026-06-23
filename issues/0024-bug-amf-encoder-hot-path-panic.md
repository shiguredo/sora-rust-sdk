# AMF エンコーダーの `assert_eq!` がホットパスでパニックを引き起こす

- Priority: High
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-amf-encoder-hot-path-panic
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 (video codec 層の致命的バグ修正) のうち「`amf.rs` のホットパス `assert_eq!`」分。

## 目的

`src/video_codecs/amf.rs` の AMF エンコーダー実装で、`encode()` のホットパスに残った `assert_eq!(surface_height as u32, frame_height);` が、入力フレームの高さと AMF が確保した surface の高さが一致しない条件で毎フレーム panic する。AMF SDK は内部で surface サイズをアライメント境界に切り上げる実装が一般的であり、特定解像度では確実に発火する。WebRTC のエンコーダーは外部スレッド (内部実装では VideoStreamEncoder の encoder queue 等) から呼ばれるため、panic は親プロセスを巻き込んでクラッシュさせる。

他のコーデック実装 (`vpl.rs` / `nvcodec.rs`) は同種の不一致を `rtc_log_error!` + `VideoCodecStatus::Error` で扱っており、AMF エンコーダーだけが panic に倒している。本 issue ではこの不整合を解消する。

## 優先度根拠

High。

- 本番で AMF を有効にしたビルドが特定解像度で確実にクラッシュする可能性がある (毎フレーム panic)
- WebRTC のエンコーダースレッドからの panic は SDK 利用側 (Sora C++ SDK 経由のアプリケーション) を巻き込むため、影響範囲がアプリケーションプロセス全体に及ぶ
- 修正は数行で済み、他のコーデック実装と同じパターンに揃えるだけ
- ただし発火条件は「frame_height が AMF の内部アライメント境界に一致しない」場合に限られるため、運用解像度によっては顕在化していない可能性もある (それでも対策コストが低いので即座に直すべき)

## 現状

`src/video_codecs/amf.rs:411` 付近 (`AmfEncoder::encode` の中):

```rust
let y_stride = plane_y.get_hpitch();
let uv_stride = plane_uv.get_hpitch();
let surface_height = plane_y.get_height();
assert_eq!(surface_height as u32, frame_height);

let Some(y_size) = (y_stride as usize).checked_mul(surface_height as usize) else {
    return VideoCodecStatus::ErrParameter;
};
```

- `frame_height` は `frame.height()` 由来 (アプリ側が要求した解像度)
- `surface_height` は `encoder.alloc_surface()` で確保した AMF surface の実高さ
- AMF は実装上 16 / 32 等の境界に align up することがあるため、両者が一致する保証はない
- 不一致のときに `assert_eq!` が panic する

問題点:

- 不一致は正常な動作の範囲であり、panic で扱う性質のものではない
- WebRTC のエンコーダースレッドから呼ばれるため panic でプロセスが落ちる
- `vpl.rs` / `nvcodec.rs` は同種の取り回しを `rtc_log_error!` + `VideoCodecStatus::Error` で行っており、AMF だけ取り扱いが不揃い

なお後続の `i420_to_nv12()` 呼び出しでは書き込み高さに `frame_height_i32` を渡しており、書き込み先バッファは `y_stride * surface_height` で確保されている (`amf.rs:413-427`)。`surface_height >= frame_height` の場合は書き込み量がバッファ内に収まるため安全側に倒せるが、`surface_height < frame_height` のケース (実際に起きるかは未確認だが理論上は要ガード) では out-of-bounds 書き込みになり得るため、明示的にエラー判定する必要がある。

## 設計方針

- `assert_eq!` を削除する
- `surface_height` と `frame_height` を比較し、`surface_height < frame_height` の場合は `rtc_log_error!` でログを出して `VideoCodecStatus::Error` を返す
- `surface_height >= frame_height` の場合は処理を継続する (AMF の align up は正常動作の範囲)
- ログメッセージは英語 (AGENTS.md 規約)、コーデック種別と両 height をメッセージに含める
- 他のコーデック実装 (`vpl.rs` / `nvcodec.rs`) のエラーログ書式に合わせる

H.264 エンコーダーだけでなく AV1 エンコーダー側にも同じ箇所がある場合は併せて修正する (要再確認)。

## 完了条件

- `src/video_codecs/amf.rs` から本件 `assert_eq!` が削除されている
- `surface_height < frame_height` 時に `VideoCodecStatus::Error` を返す経路が追加されている
- AMF surface の align up を発火させる解像度 (例: 高さが 16/32 等のアライメント境界をまたぐもの) を含む単体テストもしくは e2e テストで panic しないことが確認できる
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/video_codecs/amf.rs:411` の `assert_eq!` を削除する
2. 同位置に `if (surface_height as u32) < frame_height { rtc_log_error!(...); return VideoCodecStatus::Error; }` 相当のガードを追加する (実コードのスタイルに合わせて記述する)
3. ログメッセージは英語で、`codec_type` / `surface_height` / `frame_height` を含める
4. 既存の `vpl.rs` / `nvcodec.rs` の同等処理を参照しスタイルを揃える
5. 0023 (AMD-AMF simulcast SEGV) との関連可能性についても確認する (もし同事象なら本 issue の修正で再現しなくなる可能性がある)

## 関連

- `issues/0023-bug-amf-simulcast-segv.md`: AMD-AMF self-hosted ランナーで simulcast が SIGSEGV する事象。原因が本件と一致するかは未確認だが、ホットパス panic が原因なら本修正で改善する可能性がある
