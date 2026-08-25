# DegradationPreference を設定できるようにする

- Created: 2026-08-25
- Completed: {YYYY-MM-DD}
- Branch: feature/add-degradation-preference
- Polished: {YYYY-MM-DD}

## 目的

Sora C++ SDK は接続時に映像の DegradationPreference（負荷時の映像品質制御の優先度）を設定できるが、Rust SDK では未実装のため、C++ SDK で動作していたユースケースを Rust SDK で再現できない。Rust SDK でも同等の設定を提供する。

## 現状

- `docs/SORA_CPP_SDK.md` の「接続設定」表、`docs/SUMOMO.md` の「接続・セキュリティ」表で `degradation_preference` / `--degradation-preference` は「o | 未実装」に記載されている
- `SoraConnectionBuilder`（src/connection.rs）に degradation_preference の設定口が存在しない
- Sora C++ SDK は `SoraSignalingConfig::degradation_preference` を持ち、ネゴシエーション後の `CreateAnswer` 時に video sender の `RtpParameters.degradation_preference` を SetParameters で設定する（シグナリングメッセージには含めない、クライアント側の設定）
- shiguredo_webrtc は対応済み: `RtpParameters::set_degradation_preference` / `degradation_preference` と `DegradationPreference` enum（Disabled / MaintainFramerate / MaintainResolution / Balanced / Unknown）
- 注: libwebrtc の `DegradationPreference`（api/rtp_parameters.h）では `DISABLED` は `MAINTAIN_FRAMERATE_AND_RESOLUTION` と同値（W3C の 4 値 + disabled のうち 2 つが同一値）。したがって API 上の有効な値は C++ SDK 同様 4 種でよい

## 設計方針

- `SoraConnectionBuilder::degradation_preference(mut self, value: DegradationPreference)` を追加する（shiguredo_webrtc::DegradationPreference をそのまま使う。VideoTrack 等の既存 API と同じ扱い）
- 適用タイミングはネゴシエーション後: src/connection.rs の `apply_simulcast_encodings` と同じ位置（set_remote_description 成功後、create_answer 前）で video_sender の RtpParameters に degradation_preference を設定して SetParameters する
- シグナリングメッセージには含めない（C++ SDK と同等のクライアント側設定）
- examples/sumomo に `--degradation-preference` オプションを追加する（値は C++ sumomo と同じ `disabled` / `maintain_framerate` / `maintain_resolution` / `balanced`）

## 完了条件

- `SoraConnectionBuilder::degradation_preference()` で設定した値が、接続後に video sender の RtpParameters へ反映される
- sumomo の `--degradation-preference` で指定した値が C++ sumomo と同じ動作になる
- `docs/SORA_CPP_SDK.md` / `docs/SUMOMO.md` の機能対応表が更新されている
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- コメントは日本語、ログメッセージは英語、テストの assertion message は日本語で書く
- モックやスタブは使用しない
- `CHANGES.md` の develop セクションに `[ADD]` エントリを追記する

## 変更対象

- `src/connection.rs`（`SoraConnectionBuilder` の拡張、反映処理の追加）
- `examples/sumomo/src/args.rs` / `examples/sumomo/src/main.rs`（CLI オプションの追加）
- `docs/SORA_CPP_SDK.md` / `docs/SUMOMO.md`（機能対応表の更新）
- `CHANGES.md`
