# AMF エンコーダのエンコードホットパスの `assert_eq!` を除去する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-24
- Model: Opus 4.7
- Branch: feature/fix-amf-assert-eq-hot-path-panic
- Polished: {YYYY-MM-DD}

## 目的

AMF エンコーダのエンコードホットパスに残っている `assert_eq!` を除去し、AMF がフレーム高さより大きな surface を返した場合にプロセスがクラッシュしないようにする。

## 優先度根拠

High。エンコードは libwebrtc のワーカースレッド上でフレームごとに呼ばれるため、`assert_eq!` の失敗はプロセス全体をクラッシュさせる。AMF の `alloc_surface()` はコーデック (特に AV1 / HEVC) や実装によって要求より高さをアライン up して返す可能性があり、条件が揃えば実運用中に SIGABRT する。Issue 0023 (AMF simulcast SEGV) と関連する可能性も否定できない。

## 現状

`src/video_codecs/amf.rs:414` に以下のアサーションがある:

```rust
let y_stride = plane_y.get_hpitch();
let uv_stride = plane_uv.get_hpitch();
let surface_height = plane_y.get_height();
assert_eq!(surface_height as u32, frame_height);

let Some(y_size) = (y_stride as usize).checked_mul(surface_height as usize) else {
    return VideoCodecStatus::ErrParameter;
};
```

`frame_height` は libwebrtc から渡された VideoFrame の高さ、`surface_height` は AMF が `alloc_surface()` で確保した surface の高さ。両者が「必ず一致する」という前提でアサーションが書かれているが、この前提は AMF の仕様として保証されていない。

同じ箇所には UV プレーンの高さを `(surface_height as usize).div_ceil(2)` で計算する箇所もあり、こちらも `plane_uv.get_height()` を使わずに Y プレーン高さから推定している。

## 設計方針

1. `assert_eq!` を除去する。
2. `surface_height as u32 >= frame_height` を条件チェックとし、満たさない場合は `rtc_log_error!` で以下をログした上で `VideoCodecStatus::Error` を返す:
   - コーデック種別 (`self.codec_type`)
   - 要求フレーム高 (`frame_height`)
   - AMF から返された surface 高 (`surface_height`)
3. 後続の `y_size` / `uv_size` 計算では引き続き AMF が実際に返した `surface_height` を使う (既に checked_mul が入っており計算自体は安全)。
4. UV プレーンについては `plane_uv.get_height()` を別途取得してサイズ計算するのが理想だが、本 issue のスコープからは外し、副次的な整合性課題として issue を分ける。

## 完了条件

- `src/video_codecs/amf.rs:414` の `assert_eq!` が除去されている。
- `surface_height < frame_height` になる異常ケースで、プロセスがクラッシュせず `VideoCodecStatus::Error` を返す。
- `cargo clippy --workspace --all-features -- -D warnings` と `cargo test --workspace` が通る。

## 解決方法

本 issue は既にクローズ済みの `closed/0024-bug-amf-encoder-hot-path-panic.md` と対象箇所・修正方針が完全に重複していた。0024 は同一の `assert_eq!(surface_height as u32, frame_height);` について、AMF SDK のヘッダ・API リファレンス・公式サンプルコード (`RawStreamReader.cpp`) を調査した結果、以下を確認したうえで **コード変更不要** として closed になっている。

- `AMFPlane::GetHeight()` は crop region の高さ（未設定時は surface 全体の高さ）を返し、アライメントによるパディングは含まない
- パディングを含むスキャンライン数は `AMFPlane::GetVPitch()` が返す
- 公式サンプルでも要求高さと `GetVPitch()` は明確に区別されている

したがって `surface_height` と `frame_height` は仕様上常に一致し、`assert_eq!` は不変条件を表明する妥当な記述である。0057 の「AMF の `alloc_surface()` はアライン up して返す可能性がある」という優先度根拠は、AMF が返す `GetHeight()` の実際の挙動を誤解したものだった。

本 issue は 0024 の重複として closed にする。0024 の調査結論を疑って再検証する場合は、AV1 / HEVC + 特定 GPU / 特定ドライバでの実機検証を伴う別 issue として起票する。
