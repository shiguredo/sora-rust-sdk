# 公開 API のパラメータ検証を実装する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-validate-public-api-parameters
- Polished: {YYYY-MM-DD}

## 目的

公開 API のドキュメントに記載されている制約を検証し、不正なパラメータをサーバーに送信しないようにする。

## 現状

以下の制約がドキュメントに記載されているが、`SoraConnectionBuilder` / `ConnectDataChannel` / `Audio` / `Video` のどこにも検証されていない。

- `client_id` / `bundle_id` の 1〜255 バイト制約 (`src/connection.rs` の `SoraConnectionBuilder::client_id` / `bundle_id`)
- `ConnectDataChannel::label` の「`#` で始まる」「最大 32 文字 (`#` を含む)」制約 (`src/types.rs`)
- `ConnectDataChannel::max_packet_life_time` / `max_retransmits` の同時指定禁止 (`src/types.rs`)
- `Audio::Opus` の `bit_rate` 6〜510 の範囲 (`src/types.rs`)
- `Video` 各バリアントの `bit_rate` 1〜50000 の範囲 (`src/types.rs`)

また、`Audio::Opus.bit_rate` / `Video::*.bit_rate` のドキュメントは単位を「kbps」と記載しているが、テストコードでは bps 単位の値 (64_000 等) が使われており、実態は bps である。ドキュメントの単位表記が誤っている。

## 設計方針

- 各制約の検証をビルダー / コンストラクタ時または `build()` 時に追加する
- 検証失敗時は具体的なエラーメッセージを持つ `Error` バリアントを返す
- `bit_rate` のドキュメント単位を実態 (bps) に修正する
- 各検証の単体テストを追加する

## 完了条件

- 上記 5 つの制約が検証され、違反時にエラーが返る
- 検証の単体テストがある
- ドキュメントの単位表記が実態と一致する
- 既存の正常な設定の挙動が変わらない
- `cargo test --workspace` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `src/types.rs`
- `src/error.rs`
- `CHANGES.md`
