# libcamera の WdrMode / WdrStrength コントロールを設定可能にする

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-libcamera-wdr-controls
- Polished: {YYYY-MM-DD}

## 目的

libcamera の `WdrMode` / `WdrStrength` コントロールを `--libcamera-control` で文字列指定したときに正しい enum 値へ解決し、設定可能なはずのコントロールが静かに無視される問題を解消する。

## 現状

`src/libcamera.rs` の `all_control_ids()` には `core::WDR_MODE` (Int32, InOut) と `core::WDR_STRENGTH` (Float, In) は既に含まれており、`find_control_id` も成功する。

一方 `resolve_enum_value` には `WdrMode` の enum 解決分岐がない。shiguredo_libcamera の control_ids.rs には `wdr_mode` の enum 定義（OFF / LINEAR / POWER / EXPONENTIAL / HISTOGRAM_EQUALIZATION）があるのに参照されていない。

`parse_control_value` の Int32 分岐は `resolve_enum_value(id.name(), value)` を呼ぶため、`--libcamera-control WdrMode=Off` のような文字列指定は enum 解決に失敗し、コントロールが静かに無視される。数値指定（`WdrMode=1` 等）は `parse::<i32>()` で解決できるため設定可能。

## 設計方針

- `resolve_enum_value` に `WdrMode` の enum 解決を追加する（shiguredo_libcamera の control_ids.rs の `core::wdr_mode` を参照）
- `all_control_ids()` への追加は不要（`WDR_MODE` / `WDR_STRENGTH` は既に含まれている）
- `--libcamera-control` 指定時に unknown コントロールが警告のみでなく、必要に応じてエラーになる挙動を検討する

## 完了条件

- `resolve_enum_value` で `WdrMode` の各 enum 値（OFF / LINEAR / POWER / EXPONENTIAL / HISTOGRAM_EQUALIZATION）が解決される
- `--libcamera-control WdrMode=Off` のような文字列指定で実際に設定が渡される
- `WdrStrength` は Float 型のため、`parse_control_value` の Float 分岐で既に設定可能であることを確認する
- enum 解決の単体テストがある（モックやスタブは使わない）
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs`
- `CHANGES.md`
