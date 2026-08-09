# AMF エンコーダーのサーフェス高さ不一致で panic しないようにする

- Priority: High
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-amf-surface-height-assert-panic
- Polished: {YYYY-MM-DD}

## 目的

AMF サーフェスの高さがフレームの高さと一致しない場合に `assert_eq!` で panic せず、他箇所と同じく `VideoCodecStatus::Error` を返す。

## 現状

`src/video_codecs/amf.rs` の AMF エンコーダーのエンコード処理で、以下の `assert_eq!` が本番コードに残っている。`assert_eq!` は release ビルドでも有効で、サーフェス高さがドライバによって align up される環境 (奇数高さ、HEVC/AV1 の CTU 境界等) でアプリケーション全体が panic する。

```rust
let surface_height = plane_y.get_height();
assert_eq!(surface_height as u32, frame_height);
```

同一ファイル内の他の検証 (`i32::try_from` 等) はすべて失敗時に `VideoCodecStatus::Error` を返す方針で統一されている。

## 設計方針

- 上記の `assert_eq!` を削除し、不一致時は `VideoCodecStatus::Error` を返す (必要に応じて英語でエラーログを出力する)
- サーフェス高さとフレーム高さが異なる場合の扱い (align up された高さでのコピー・エンコード) を明確にする

## 完了条件

- サーフェス高さとフレーム高さが不一致でも panic しない
- 不一致時は `VideoCodecStatus::Error` が返る
- 正常系のエンコード挙動が変わらない
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/video_codecs/amf.rs`
- `CHANGES.md`
