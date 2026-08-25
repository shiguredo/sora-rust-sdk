# libcamera の WdrMode コントロールを enum 文字列で設定可能にする

- Priority: Medium
- Created: 2026-08-10
- Completed: 2026-08-15
- Model: deepseek-v4-flash
- Branch: feature/fix-libcamera-wdr-controls
- Polished: 2026-08-14

## 目的

libcamera の `WdrMode` コントロールを `--libcamera-control` で文字列指定したときに正しい enum 値へ解決し、設定可能なはずのコントロールが適用されない問題を解消する。

## 現状

`src/libcamera.rs` の `all_control_ids()` には `core::WDR_MODE` (Int32, InOut) と `core::WDR_STRENGTH` (Float, In) は既に含まれており、`find_control_id` も成功する。

一方 `resolve_enum_value` には `WdrMode` の enum 解決分岐がない。shiguredo_libcamera の control_ids.rs には `wdr_mode` の enum 定義（OFF / LINEAR / POWER / EXPONENTIAL / HISTOGRAM_EQUALIZATION）があるのに参照されていない。

`parse_control_value` の Int32 分岐は `resolve_enum_value(id.name(), value)` を呼ぶため、`--libcamera-control WdrMode=Off` のような文字列指定は enum 解決に失敗し、`rtc_log_warning!` による警告ログを出した上でコントロールが適用されない（`parse_controls` の `invalid libcamera control value` 分岐）。数値指定（`WdrMode=1` 等）は `parse::<i32>()` で解決できるため設定可能。

## 設計方針

- `resolve_enum_value` に `WdrMode` の enum 解決を追加する（shiguredo_libcamera の control_ids.rs の `core::wdr_mode` を参照）
- 入力文字列は既存分岐と同じ CamelCase とし、大文字小文字は厳密一致で判定する（`Off` / `Linear` / `Power` / `Exponential` / `HistogramEqualization`）
- `all_control_ids()` への追加は不要（`WDR_MODE` / `WDR_STRENGTH` は既に含まれている）
- `WdrStrength` は Float 型のため enum 解決の対象外（`parse_control_value` の Float 分岐で既に設定可能であり、コード変更は不要）
- `resolve_enum_value` に enum 解決分岐がない Int32 コントロールは `WdrMode` 以外にも存在するが、本 issue では `WdrMode` のみを対象とする（他のコントロールへの対応は必要に応じて別 issue で扱う）

## 完了条件

- `resolve_enum_value` で `WdrMode` の各 enum 値（`Off` / `Linear` / `Power` / `Exponential` / `HistogramEqualization`）が対応する定数値へ解決される
- `--libcamera-control WdrMode=Off` のような文字列指定で実際に設定が渡される（単体テストで `parse_control_value` が `WdrMode=Off` を `I32(0)` へ解決することを検証する）
- enum 解決の単体テストがある（モックやスタブは使わない）。全 enum 値・無効文字列（`WdrMode=Foo`）・数値指定（`WdrMode=1`）・小文字（`WdrMode=off`）の各ケースを検証する
- `cargo test --workspace --features libcamera` が成功する（`libcamera` feature は default に含まれず、システム依存のため self-hosted の Raspberry Pi CI でのみ検証される。ローカルで feature を有効にできない環境では CI の結果で確認する）
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs`
- `CHANGES.md`（`[FIX]` エントリを追加）

## 解決方法

- `resolve_enum_value` に `WdrMode` の enum 解決分岐を追加し、`core::wdr_mode` の定数へ解決するようにした
- `ControlValue` に `Debug` を導出してテストの assertion メッセージで値を表示できるようにした
- 単体テストを追加した（全 enum 値・無効文字列・数値指定・小文字指定の各ケース）
