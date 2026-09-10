# adaptivePtime を SDK オプションで音声トラックに適用する

- Created: 2026-09-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-adaptive-ptime
- Polished: {YYYY-MM-DD}

## 目的

音声送信の adaptivePtime（適応的パケット化時間）を SDK の設定で有効化できるようにする。

adaptivePtime は W3C の `RTCRtpEncodingParameters.adaptivePtime` に対応する音声向けパラメータで、libwebrtc では audio の voice engine だけが `webrtc::RtpEncodingParameters::adaptive_ptime` を参照する。映像側では参照されない。サイマルキャストの有無に関係なく、SDK の設定で音声トラックに適用できるようにする。

## 現状

- `src/signaling_types.rs` の `SimulcastEncodingConfig` が offer の `adaptivePtime` を `adaptive_ptime` として保持している
- `src/connection.rs` の `apply_simulcast_encodings` が `RtpEncodingParameters::set_adaptive_ptime` を呼び、`video_sender` に設定している
- `add_sender_tracks` は audio の `add_sender_media_track` の戻り値 (`RtpSender`) を捨てており、audio sender を保持していない
- libwebrtc で `adaptive_ptime` を参照するのは audio の voice engine だけなので、video sender への設定は実効しない

## 設計方針

- `SoraConnectionBuilder` に `adaptive_ptime(bool)` を追加し、内部で `Option<bool>` として保持する。未設定 (`None`) の場合は何も設定せず libwebrtc の既定に任せる（`degradation_preference` と同じ方針）
- `add_sender_tracks` で audio の `RtpSender` を保持する
- `set_remote_description` 成功後、`create_answer` 前に、値が設定済みかつ audio sender が存在する場合に `get_parameters` → `set_adaptive_ptime` → `set_parameters` で適用する。`recvonly` や送信音声なし、未設定の場合はスキップし、エラーにしない
- offer の `simulcast_encodings[].adaptive_ptime` は video sender に設定しない。`SimulcastEncodingConfig` の `adaptive_ptime` と `apply_simulcast_encodings` の適用処理を削除する
- 初回 offer と re-offer を区別せず、`handle_offer` が呼ばれるたびに同じ条件で再適用する。`SetParameters` 失敗時の扱いは `apply_simulcast_encodings` に準じる
- `examples/sumomo` に `--adaptive-ptime` オプションを追加する
- `docs/SORA_CPP_SDK.md` / `docs/SUMOMO.md` の機能対応表を更新する
- 変更履歴（`CHANGES.md`）の `## develop` に `[ADD]` エントリを追記する

## 完了条件

- 送信ありの role で `adaptive_ptime(true)` を設定した場合に audio sender の `RtpParameters` の encoding の `adaptive_ptime` が true になること
- 未設定 / `recvonly` / 送信音声なしの場合はスキップされ、従来どおり接続できること
- video sender の encoding に `adaptive_ptime` が設定されないこと
- `sumomo` の `--adaptive-ptime` で指定した値が反映されること
- `docs` の機能対応表が更新されていること
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功すること
- コメントは日本語、ログメッセージは英語、テストの assertion message は日本語で書くこと
- モックやスタブは使用しないこと
- `CHANGES.md` の `## develop` に `[ADD]` が追記されていること

## 変更対象

- `src/connection.rs`（`SoraConnectionBuilder` の拡張、audio sender の保持、適用処理の追加）
- `src/signaling_types.rs`（`SimulcastEncodingConfig::adaptive_ptime` の削除）
- `examples/sumomo/src/args.rs` / `examples/sumomo/src/main.rs`
- `docs/SORA_CPP_SDK.md` / `docs/SUMOMO.md`
- `CHANGES.md`

## テスト方針

- 送信ありの role で `adaptive_ptime(true)` を設定した場合に audio sender の `RtpParameters` の encoding に反映され、未設定 / `recvonly` 時は従来どおり接続できることをテストで確認する
- 実際の libwebrtc の getter API を利用して適用結果を検証する
