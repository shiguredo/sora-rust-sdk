# AMD-AMF 環境で simulcast テストが SIGSEGV でクラッシュする

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-amf-simulcast-segv
- Polished: {YYYY-MM-DD}

## 目的

AMD-AMF self-hosted ランナーで e2e-tests の `simulcast` バイナリが SIGSEGV (signal 11) で異常終了した事象を調査・記録する。同一事象が再発した場合の切り分けや、原因が特定された際の修正方針に必要な情報を残す。

## 優先度根拠

- 現時点では 1 度の発生のみで、flaky か恒常的なバグか不明
- ただし SIGSEGV はメモリ違反であり、再現するなら即座に重大度が上がる事象
- 他プラットフォーム（ubuntu / windows / macOS / Intel-VPL / NVIDIA / Raspberry-Pi 等 11 ジョブ）は同一 push で全て success のため、AMD-AMF 固有の問題に切り分けられる
- 詳細調査が必要なので Medium

## 現状

2026-06-23 の develop CI run [28010373428](https://github.com/shiguredo/sora-rust-sdk/actions/runs/28010373428) で発生:

- 失敗ジョブ: `ci-self-hosted (self-hosted, linux, x64, AMD-AMF, --features openh264,amf)`
- 失敗テストバイナリ: `e2e-tests` の `simulcast`
- 直前まで通過: `test_sendonly_simulcast_outbound_layers_openh264 ... ok`
- その次に実行されようとしたテストで segfault

ログ抜粋:

```
error: test failed, to rerun pass `-p e2e-tests --test simulcast`

Caused by:
  process didn't exit successfully: `.../simulcast-... --skip test_video_capture_frame_received --skip test_video_capture_session_create` (signal: 11, SIGSEGV: invalid memory reference)
##[error]Process completed with exit code 101.
```

同 push の他 11 ジョブ（`ubuntu-22.04` / `ubuntu-22.04-arm` / `ubuntu-24.04` / `ubuntu-24.04-arm` / `windows-2025` / `macOS Apple-M1` / `Intel-VPL self-hosted` / `NVIDIA-Video-Codec-SDK self-hosted` / `Raspberry-Pi self-hosted` 等）はいずれも success。

### 直前の関連 CI

1 つ前の push [28007985159](https://github.com/shiguredo/sora-rust-sdk/actions/runs/28007985159) では別の事象（`ci (ubuntu-24.04)` の `test_sendonly_data_channel_signaling` がタイムアウト）で失敗したが、rerun で success に転じている（flaky 扱い）。本件はそれとは別ジョブ・別テスト・別の失敗モード（タイムアウトではなく segfault）。

### トリガーになった push との関係

本件の push（commit `284301f` / `fc8db06`）は時雨堂依存クレートを完全 pin に統一する作業。`shiguredo_amf` は `Cargo.toml` の表記が `"2026.3"` から `"=2026.3.0"` に変わっただけで、`Cargo.lock` の解決値は前回 push と同じ `2026.3.0` のため、ビルドされるバイナリは前回 push と同一。コード差分が SIGSEGV を引き起こした可能性は低い。

## 設計方針

まずは flaky / 恒常バグの切り分けに集中する。修正は原因特定後に判断する。

1. CI を rerun し、同事象が再現するかを確認する
2. 後続の develop push で同事象が再発するかを継続観測する
3. 再現する場合は segfault 直前で実行されたテストを特定する（`e2e-tests/tests/simulcast.rs` のテスト並び順、`--skip` で除外しているテストの周辺）
4. AMD-AMF self-hosted ランナーの環境情報（カーネル、AMF SDK バージョン、GPU ドライバ、GPU 状態）を記録する
5. 再現性が確認できたら、ローカル再現を試み coredump / gdb で stack trace を取得する
6. 上流（`shiguredo_amf` クレート）か SDK 側（`src/amf.rs` 等）かを切り分け、必要なら shiguredo-amf-rs に詳細を共有する

## 完了条件

- 事象が flaky と判定され、一定期間（例: 直近 10 回の AMD-AMF ジョブ）再発しない
- または恒常バグと特定され、原因が明らかになり、修正または回避策（テスト分離、ランナー設定変更、上流バグ修正 + 該当バージョン pin 等）が反映されている

## 解決方法

調査結果に応じて以下のいずれか:

- flaky 確定なら closed にして経過観察。再発したら reopened する
- 上流の bug なら `shiguredo_amf` 側で修正してもらい、修正版が出るまで該当バージョンを避ける pin / skip を入れる
- SDK 側の bug ならコードを修正してテストを追加する
