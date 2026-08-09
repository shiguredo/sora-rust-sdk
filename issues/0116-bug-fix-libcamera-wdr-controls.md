# libcamera の WdrMode / WdrStrength コントロールを設定可能にする

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-libcamera-wdr-controls
- Polished: {YYYY-MM-DD}

## 目的

libcamera の `all_control_ids` に `WdrMode` / `WdrStrength` を追加し、設定可能なはずのコントロールが静かに無視される問題を解消する。

## 現状

`src/libcamera.rs` の `all_control_ids()` は libcamera の設定可能な In / InOut コントロールの一覧を返すが、`core::WDR_MODE` (Int32, InOut) と `core::WDR_STRENGTH` (Float, In) の 2 つが欠落している。shiguredo_libcamera の control_ids.rs には両方が定義されている。

ユーザーが `--libcamera-control WdrMode=...` 等で指定しても、`find_control_id` が失敗して「unknown libcamera control」警告が出るだけで無視される。

## 設計方針

- `all_control_ids()` に `WdrMode` / `WdrStrength` を追加する
- `WdrMode` の enum 値解決 (`resolve_enum_value`) を追加する (shiguredo_libcamera の control_ids.rs に `wdr_mode` の enum 定義があるため)
- `--libcamera-control` 指定時に unknown コントロールが警告のみでなく、必要に応じてエラーになる挙動を検討する

## 完了条件

- `WdrMode` / `WdrStrength` が `all_control_ids()` に含まれる
- `--libcamera-control WdrMode=...` で実際に設定が渡される
- 設定可能な In / InOut コントロールの一覧と control_ids.rs の突き合わせで欠落がないことを確認するテストがある
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs`
- `CHANGES.md`
