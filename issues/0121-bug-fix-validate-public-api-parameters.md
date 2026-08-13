# 公開 API のパラメータ検証を実装する

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-validate-public-api-parameters
- Polished: 2026-08-14

## 目的

公開 API のドキュメントに記載されている制約を検証し、不正なパラメータをサーバーに送信しないようにする。

## 現状

以下の制約がドキュメントに記載されているが、`SoraConnectionBuilder` / `ConnectDataChannel` / `Audio` / `Video` のどこにも検証されていない。

- `client_id` / `bundle_id` の 1〜255 バイト制約 (`src/connection.rs` の `SoraConnectionBuilder::client_id` / `bundle_id`)
- `ConnectDataChannel::label` の制約 (`src/types.rs`)。Sora 仕様では正規表現 `^#[a-zA-Z0-9][a-zA-Z0-9-]{1,30}$` で表され、「`#` で始まる」「最大 32 文字 (`#` を含む)」「英数字と `-` のみ」「`#` の直後は `-` でない」のすべてを満たす必要がある
- `ConnectDataChannel::max_packet_life_time` / `max_retransmits` の同時指定禁止 (`src/types.rs`)
- `Audio::Opus` の `bit_rate` 6〜510 の範囲 (`src/types.rs`)
- `Video` 各バリアントの `bit_rate` 1〜50000 の範囲 (`src/types.rs`)

また、`Audio::Opus.bit_rate` / `Video::*.bit_rate` は Sora のドキュメント上 kbps 単位 (音声 6〜510、映像 1〜50000) であるが、テストコードでは bps 単位の値 (64_000 等) が使われており、kbps として扱うと範囲外の不正な値になる。テストコードの値が誤っている。

## 設計方針

- 各制約の検証を `SoraConnectionBuilder::build()`（`SoraConnection::new`）に集約する
  - `Audio::new_opus` / `Video::new_vp8` 等のコンストラクタと `SoraConnectionBuilder::client_id` 等の setter はすべて `-> Self` を返す公開 API であり、検証を置くとシグネチャを `Result<Self>` に変える破壊的変更が必要になる
  - `ConnectDataChannel` は公開フィールドのみの構造体でコンストラクタがなく、コンストラクタ時検証は定義上不可能
  - したがって、5 制約すべてを公開 API のシグネチャを壊さずに検証できるのは `build()` が適切な場所である（`run()` は接続開始後で遅すぎる）
- 検証失敗時は具体的なエラーメッセージを持つ `Error` バリアントを返す
  - 既存の `Error::InvalidDataChannelLabel` は `send_message` 実行時のラベル検証用で意味が異なるため流用せず、新規バリアントを追加する
  - 新規バリアントの `Display` は既存の慣習に合わせ日本語にする（`production log` は英語だが、`Error` の `Display` は利用者向けの日本語が規約）
- ラベル制約は Sora 仕様の正規表現 `^#[a-zA-Z0-9][a-zA-Z0-9-]{1,30}$` をそのまま検証対象とし、`src/types.rs` の `ConnectDataChannel::label` の doc コメントも正規表現に合わせて更新する。実装は依存を増やさないため正規表現クレートを使わず、文字単位の検証で行う
- `bit_rate` の範囲検証は role に関わらず行う（recvonly で使用する場合でも範囲外の値を拒否するのは正当）。映像 `bit_rate` は Sora 仕様で「15 Mbps より大きい値は現時点ではサポート外」とされるが、本 issue ではドキュメントに明記された範囲 1〜50000 の検証のみを対象とし、15000 超の拒否は対象外とする
- `channel_id` の 1〜255 バイト制約は Sora 仕様に存在するが、`src/connection.rs` の `SoraConnectionBuilder` の doc に制約が明記されていないため本 issue の対象外とする（client_id / bundle_id は doc に明記されているため対象）
- テストコードの `bit_rate` 値を kbps 単位の範囲内 (音声 6〜510、映像 1〜50000) に修正し、期待 JSON 文字列も同時に更新する
- 各検証の単体テストを追加する。検証ロジックは `build()` 内の private ヘルパーとして実装し、`src/connection.rs` の `#[cfg(test)]` モジュールにテストを配置する（公開 API 経由で `build()` のエラーを検証する）

## 完了条件

- 上記 5 つの制約が `build()` で検証され、違反時にエラーが返る
- 検証の単体テストがある（モックやスタブは使わない）
- テストコードの `bit_rate` 値と期待 JSON 文字列が kbps 単位の範囲内 (音声 6〜510、映像 1〜50000) になっている
- 既存の正常な設定の挙動が変わらない
- `cargo test --workspace` が成功する
- `CHANGES.md` の `## develop` に `[FIX]` エントリを追加する
- production log は英語、コメントとテストの assertion message は日本語にする

## 変更対象

- `src/connection.rs`
- `src/types.rs`
- `src/error.rs`
- `CHANGES.md`
