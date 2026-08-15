# --libcamera-control で unknown コントロール指定時に警告でなくエラーにする

- Priority: Medium
- Created: 2026-08-14
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/change-error-on-unknown-libcamera-control
- Polished: 2026-08-16

## 目的

`--libcamera-control` で unknown コントロール（`find_control_id` が失敗するコントロール名）を指定したときに、警告ログだけに留めずエラーで通知し、タイポや設定ミスに気づけるようにする。

## 現状

`src/libcamera.rs` の `parse_controls` は、`find_control_id(key)` が失敗した場合に `rtc_log_warning!("unknown libcamera control: {}")` を出力して `continue` し、コントロールを静かに無視する。ユーザーはコントロール名を typo しても気づきにくい。

## 設計方針

- `parse_controls` の戻り値を `Result<Vec<ParsedControl>>` にし、unknown コントロール検出時にエラーを返す
  - コントロール名を保持する `Error` バリアントを追加する（例: `Error::UnknownLibcameraControl { name }`）。既存の `Error::LibcameraMessage` と同様に `#[cfg(feature = "libcamera")]` でゲートする
  - Display は `unknown libcamera control: <名前>` にする（`libcamera error:` のプレフィックスを付けない。`Error::LibcameraMessage` を流用すると完了条件のログが二重になるため専用バリアントにする）
- エラーは `parse_controls` → `run_libcamera_loop_inner` → `run_libcamera_loop` と `Result` で伝搬させ、キャプチャが開始されないようにする
  - エラーは `LibcameraVideoCapturer::start` が spawn したスレッド内で `rtc_log_error!("libcamera capture failed: {}")` によりログされる（`start()` 自身は `Ok` のまま）
- 設定可能なコントロール名の typo 検出を目的とするため、unknown コントロールのみを対象とし、次の 2 つは従来どおり警告のまま無視する
  - invalid value（`parse_control_value` の失敗）
  - read-only コントロール（`id.direction() == Direction::Out`）

## 完了条件

- unknown コントロールを `--libcamera-control` で指定すると、キャプチャが開始されず `libcamera capture failed: unknown libcamera control: <名前>` のエラーログが出力される
- invalid value と read-only コントロールの挙動（警告のまま）は変わらない
- 正常なコントロール指定の挙動は変わらない
- `parse_controls` が unknown コントロールでエラーを返すこと、正常なコントロールで従来どおりパースされること、invalid value と read-only コントロールは従来どおりスキップされることを単体テストで検証する
- `cargo test --workspace --features libcamera` が成功する（`libcamera` feature は default に含まれず、システム依存のため self-hosted の Raspberry Pi CI でのみ検証される。ローカルで feature を有効にできない環境では CI の結果で確認する）
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/libcamera.rs`
  - `parse_controls`: 戻り値を `Result` に変更し、unknown コントロールでエラーを返す
  - `run_libcamera_loop_inner`: `parse_controls` のエラーを伝搬させる
- `src/error.rs`
- `CHANGES.md`（`[CHANGE]` エントリを追加）
