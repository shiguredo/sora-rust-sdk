# `SoraConnectionHandle::send_message` にラベル検証を追加し、内部 DataChannel の横取りを防ぐ

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-send-message-label-validation
- Polished: 2026-07-27

## 目的

`SoraConnectionHandle::send_message` → `SendMessage` コマンド → `send_data_channel_message` の経路で、ラベルの検証が一切ない。SKILL.md (skills/sora-rust-sdk/SKILL.md:546) には「`send_message` / `on_message` はユーザー定義 DataChannel 専用」「SDK 内部用ラベル (`signaling` 等) を渡すと `Error::DataChannelMissing` になる」と明記されているが、実装上はそうなっていない。内部ラベル (`signaling` / `stats` / `push` / `notify` / `rpc`) も DataChannel として `self.data_channels` に登録済みのため、`send_data_channel_message` の `get_mut(label)` は成功し、ユーザーから任意バイナリを送信できてしまう。SKILL.md の「`DataChannelMissing` になる」という記述自体も実態と合っていない。ラベル検証を追加して仕様と実装を一致させる。

## 優先度根拠

High。API 契約違反。ユーザーが「`#` プレフィックスのユーザー定義 DataChannel 専用」の約束のもとで `send_message` を呼んでいる前提が、実装で守られていない。誤って `"signaling"` などを渡すと Sora シグナリングチャネルに任意バイナリが注入され、Sora 側のプロトコル状態が破壊される。

## 現状

`src/connection.rs:1731`:

```rust
fn send_data_channel_message(&mut self, label: &str, data: &[u8]) -> Result<()> {
    let managed =
        self.data_channels
            .get_mut(label)
            .ok_or_else(|| Error::DataChannelMissing { label: label.to_string() })?;
    ...
}
```

`label.starts_with('#')` の検証は無い。`SoraConnectionHandle::send_message` (connection.rs:509) 側でも検証していない。

`handle_datachannel_message` (受信側) は `label.starts_with('#')` の場合のみ `on_message` を呼ぶ非対称設計になっており、送信側だけが素通し。

## 設計方針

`SoraConnectionHandle::send_message` の入口で `label.starts_with('#')` を検証し、違反時はエラーを返す。

検証箇所は `SoraConnectionHandle::send_message` (connection.rs:509) とする。`send_data_channel_message` (connection.rs:1731) は `SendMessage` コマンドハンドラ (connection.rs:1012) からのみ呼ばれるため、どちらの層で検証しても動作は同じだが、公開 API 入口で弾くほうが早期リターンとして適切。なお、SDK 内部の `send_datachannel_message` (connection.rs:1709、アンダースコアなし) は `send_data_channel_message` (connection.rs:1731、アンダースコアあり) とは別関数であり、`send_signaling_message` / `send_stats_message` / RPC 送信から呼ばれる内部経路で、本検証の影響を受けない。

エラー型は新規の `Error::InvalidDataChannelLabel { label }` を設ける。`DataChannelMissing` は「チャネルが見つからない」意味であり、本件は「チャネルは存在するがラベルが API 契約に違反している」ため意味が異なる。既存の `Error::InvalidVideoCodecCapability { reason }` (error.rs:191) 等の `Invalid*` パターンに揃える。

単体テストで「`"signaling"` / `""` / `"stats"` を渡すと `Error::InvalidDataChannelLabel` を返す」ことを確認する。`SoraConnectionHandle::send_message` はチャネル経由で接続タスクにコマンドを送る設計のため、検証エラーはコマンド送信前に同期적으로返る。`"#chat"` は検証を通過するが、実際の送信完了には接続タスクの起動が必要。テストは `src/connection.rs` の `#[cfg(test)] mod tests` (connection.rs:2573) に追加する。

SKILL.md (skills/sora-rust-sdk/SKILL.md:546) の「SDK 内部用ラベル (`signaling` 等) を渡すと `Error::DataChannelMissing` になる」という記述は、内部ラベルが `data_channels` に登録済みである実態と合っていない。本修正で `Error::InvalidDataChannelLabel` を返すようにしたうえで、SKILL.md の記述を「`#` プレフィックスのないラベルを渡すと `Error::InvalidDataChannelLabel` になる」に更新する。あわせて SKILL.md:528 のエラーカタログ表 (`DataChannel / RPC` 行) にも `InvalidDataChannelLabel` を追加する。

`send_message` の rustdoc (connection.rs:506-508) にもエラー条件 (`#` プレフィックス必須、違反時は `Error::InvalidDataChannelLabel`) を追記する。

## 完了条件

- `SoraConnectionHandle::send_message("signaling", ...)` が `Error::InvalidDataChannelLabel` を返し、内部 DataChannel にメッセージが送信されない。
- `SoraConnectionHandle::send_message("#chat", ...)` は従来通り送信できる。
- SKILL.md の「`#` プレフィックス以外のラベルは触らない」記述と実装が一致している。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` エントリが追加されている。
