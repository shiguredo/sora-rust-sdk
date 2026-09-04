# DegradationPreference を設定できるようにする

- Created: 2026-08-25
- Completed: {YYYY-MM-DD}
- Branch: feature/add-degradation-preference
- Polished: 2026-09-04

## 目的

Sora C++ SDK は接続時に映像の DegradationPreference （負荷時の映像品質制御の優先度）を設定できるが、Rust SDK では未実装のため、C++ SDK で動作していたユースケースを Rust SDK で再現できない。Rust SDK でも同等の設定を提供する。

## 現状

- `docs/SORA_CPP_SDK.md` の「接続設定」表、`docs/SUMOMO.md` の「接続・セキュリティ」表で `degradation_preference` / `--degradation-preference` は「o | 未実装」に記載されている
- `SoraConnectionBuilder` (`src/connection.rs`) に `degradation_preference` の設定口が存在しない
- `shiguredo_webrtc` は対応済み: `RtpParameters::set_degradation_preference` / `degradation_preference` と `DegradationPreference` enum (`MaintainFramerateAndResolution` / `MaintainFramerate` / `MaintainResolution` / `Balanced` / `Unknown`)
- `Unknown` は整数ペイロード付き (`Unknown(i32)`) であり、unit variant ではない

## 設計方針

- `SoraConnectionBuilder::degradation_preference` を追加する。引数は `shiguredo_webrtc::DegradationPreference` を値で受け取り、内部では `Option<DegradationPreference>` で保持する。未設定 (`None`) の場合は何も設定せず libwebrtc の既定に任せる
- 適用はネゴシエーション後に行う。`src/connection.rs` の `handle_offer` 内で `set_remote_description` 成功後、`create_answer` 前に、`apply_simulcast_encodings` と同じ区間で video sender の `RtpParameters` に設定して `SetParameters` する
- 適用条件は「ビルダーに値が設定済み」かつ「`video_sender` が存在する」場合のみとする。`recvonly` や送信映像なし、未設定の場合はスキップし、エラーにしない
- 初回 offer と re-offer を区別せず、`handle_offer` が呼ばれるたびに同じ条件で再適用する。`SetParameters` 失敗時の扱いは `apply_simulcast_encodings` に準じる
- シグナリングメッセージ (`OutgoingMessage::Connect`) には含めないクライアント側設定とする
- `examples/sumomo` に `--degradation-preference` オプションを追加する。CLI 文字列と variant の対応は次のとおりとし、`Unknown` は CLI から指定できない
  - `disabled` → `MaintainFramerateAndResolution`
  - `maintain_framerate` → `MaintainFramerate`
  - `maintain_resolution` → `MaintainResolution`
  - `balanced` → `Balanced`

## 完了条件

- 送信ありの role で値を設定した場合に video sender の `RtpParameters` へ反映され、未設定時と `recvonly` 時はスキップされて従来どおり接続できることをテストで確認する
- `sumomo` の `--degradation-preference` で指定した値が設計方針の対応表どおりの variant で反映される
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
