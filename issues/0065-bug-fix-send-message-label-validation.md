# `SoraConnectionHandle::send_message` にラベル検証を追加し、内部 DataChannel の横取りを防ぐ

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-send-message-label-validation
- Polished: {YYYY-MM-DD}

## 目的

`SoraConnectionHandle::send_message` → `SendMessage` コマンド → `send_data_channel_message` の経路で、ラベルの検証が一切ない。SKILL.md には「`send_message` / `on_message` はユーザー定義 DataChannel 専用」「SDK 内部用ラベル (`signaling` 等) を渡すと `Error::DataChannelMissing` になる」と明記されているが、実装上はそうなっておらず、`self.data_channels` に登録済みの内部ラベル (`signaling` / `stats` / `push` / `notify` / `rpc`) 宛にユーザーから任意バイナリを送信できてしまう。ラベル検証を追加して仕様と実装を一致させる。

## 優先度根拠

High。API 契約違反。ユーザーが「`#` プレフィックスのユーザー定義 DataChannel 専用」の約束のもとで `send_message` を呼んでいる前提が、実装で守られていない。誤って `"signaling"` などを渡すと Sora シグナリングチャネルに任意バイナリが注入され、Sora 側のプロトコル状態が破壊される。

## 現状

`src/connection.rs:1731` あたり:

```rust
fn send_data_channel_message(&mut self, label: &str, data: &[u8]) -> Result<()> {
    let managed =
        self.data_channels
            .get_mut(label)
            .ok_or_else(|| Error::DataChannelMissing { label: label.to_string() })?;
    ...
}
```

`label.starts_with('#')` の検証は無い。`SoraConnectionHandle::send_message` (connection.rs:509 付近) 側でも検証していない。

`handle_datachannel_message` (受信側) は `label.starts_with('#')` の場合のみ `on_message` を呼ぶ非対称設計になっており、送信側だけが素通し。

## 設計方針

1. `SoraConnectionHandle::send_message` の入口で `label.starts_with('#')` を検証し、違反時は `Error::DataChannelMissing { label }` (もしくは新規 `Error::InvalidDataChannelLabel { label }`) を返す。
2. または、より深い層 `send_data_channel_message` (connection.rs:1731) で検証する。どちらでも動作は同じだが、`send_data_channel_message` は SDK 内部の `send_datachannel_message` からも呼ばれる可能性があるため、`SoraConnectionHandle::send_message` 側で弾くのが安全。
3. エラー型を新設する場合は `Error::DataChannelSendFailed` のように既存の粒度に揃える。
4. 単体テストで「`"signaling"` / `""` / `"stats"` を渡すと Err、`"#chat"` は Ok 経路に入る」ことを確認する。
5. SKILL.md 側と挙動が一致していることを確認する。

## 完了条件

- `SoraConnectionHandle::send_message("signaling", ...)` が Err を返し、内部 DataChannel にメッセージが送信されない。
- `SoraConnectionHandle::send_message("#chat", ...)` は従来通り送信できる。
- SKILL.md の「`#` プレフィックス以外のラベルは触らない」記述と実装が一致している。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。
