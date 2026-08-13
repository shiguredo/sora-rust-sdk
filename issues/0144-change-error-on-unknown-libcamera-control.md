# --libcamera-control で unknown コントロール指定時に警告でなくエラーにする

- Priority: Medium
- Created: 2026-08-14
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/change-error-on-unknown-libcamera-control
- Polished: {YYYY-MM-DD}

## 目的

`--libcamera-control` で unknown コントロール（`find_control_id` が失敗するコントロール名）を指定したときに、警告ログだけに留めずエラーで通知し、タイポや設定ミスに気づけるようにする。

## 現状

`src/libcamera.rs` の `parse_controls` は、`find_control_id(key)` が失敗した場合に `rtc_log_warning!("unknown libcamera control: {}")` を出力して `continue` し、コントロールを静かに無視する。ユーザーはコントロール名を typo しても気づけない。

## 設計方針

- `parse_controls` の unknown コントロール検出時に警告のみでなくエラーを返す（`Error` バリアントを追加する）
- エラーは既存のコントロール指定とは独立して、`run_libcamera_loop_inner` の呼び出し元へ伝搬させる
- 設定可能なコントロール名の typo 検出を目的とするため、unknown コントロールのみを対象とし、invalid value（`parse_control_value` の失敗）は対象としない

## 完了条件

- unknown コントロールを `--libcamera-control` で指定するとエラーが返る
- invalid value の挙動（警告のまま）は変わらない
- 正常なコントロール指定の挙動は変わらない
- `cargo test --workspace --features libcamera` が成功する（`libcamera` feature は default に含まれず、システム依存のため self-hosted の Raspberry Pi CI でのみ検証される。ローカルで feature を有効にできない環境では CI の結果で確認する）
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs`
- `src/error.rs`
- `CHANGES.md`（`[CHANGE]` エントリを追加）
