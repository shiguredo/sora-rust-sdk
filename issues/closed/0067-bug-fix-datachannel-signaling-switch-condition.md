# DataChannel シグナリング切替判定を修正し、シグナリング DC が無い構成での誤発火を防ぐ

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-27
- Model: Opus 4.7
- Branch: feature/fix-datachannel-signaling-switch-condition
- Polished: {YYYY-MM-DD}

## 目的

`handle_datachannel_state` の DataChannel シグナリング切替判定 `opened_datachannels.len() == self.data_channel_configs.len()` は、ユーザー定義 DC のみを開く構成 (`data_channel_signaling=false` + `data_channels(vec![ConnectDataChannel{ label: "#chat", ... }])`) で誤発火する。`#chat` が open した瞬間 `1==1` で `use_datachannel_signaling=true` が立ち、以降のシグナリング送信が「存在しない signaling ラベル」を探して `Error::DataChannelMissing` で run() が終了する。判定条件に「シグナリング用 DC が実在すること」と「switched 済みであること」を追加する。

## 優先度根拠

High。API 契約違反かつ実運用で発生する。sora-rust-sdk の主要な使い方の 1 つ (WebSocket シグナリング + ユーザー定義 DataChannel) で発生する SDK バグ。ユーザーはコードに問題がないのに接続が切れる原因を追跡できない。

## 現状

`src/connection.rs:1687-1698` あたり:

```rust
if self.is_datachannel_open(label) && !opened_datachannels.contains(label) {
    ...
    opened_datachannels.insert(label.to_string());
    handler.on_data_channel_open(label);
    if opened_datachannels.len() == self.data_channel_configs.len() {
        *use_datachannel_signaling = true;   // ← switched とは無関係に立つ
    }
} else if self.is_datachannel_closed(label) && opened_datachannels.contains(label) {
    ...
}
```

`self.data_channel_configs` は offer の `data_channels` から作られるため、`data_channel_signaling=false` でも `data_channels(...)` を渡していれば非空になる。ユーザー定義 DC が全部 open した瞬間に `use_datachannel_signaling=true` が立ってしまう。

## 設計方針

1. 判定条件に「`switched_received == true`」を必須で加える。Sora からの `switched` メッセージを受信するまでは DataChannel シグナリング切替は成立しないことを型 (または関数の前提) として明示。
2. さらに「シグナリング用 DC (`signaling`, `notify`, `push`, `stats`)」のセットが実在することを明示的に確認するのが望ましい。ユーザー定義 DC (`#` プレフィックス) はカウントから除外する。
3. 実装上は `data_channel_configs` を 2 種類 (内部シグナリング用 / ユーザー定義) に分けて管理し、内部シグナリング用 DC の全 open で切替を発火する。
4. Close 分岐 (line 1694) も「一度も Open にならずに Closed になった DataChannel」の close 通知が抜ける問題があるので、合わせて見直す。

## 完了条件

- `data_channel_signaling=false` + `data_channels(vec![ConnectDataChannel{ label: "#chat", ... }])` の構成で `#chat` が open しても `use_datachannel_signaling=true` が立たない。
- `switched` を受信していない状態では切替が発火しない。
- 単体テストで上記のシナリオが検証されている (SoraConnection のフィールド初期化コストのため、内部関数の単体テストで代替可)。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。

## 解決方法

Sora のドキュメント上、`data_channel_signaling=false` + `data_channels`（ユーザー定義 DataChannel）の構成はサポートされていない。リアルタイムメッセージング機能を利用するには `data_channel_signaling=true` が必須であり（MESSAGING より）、`data_channel_signaling=false` の場合 Sora は offer に `data_channels` を含めない（WEBSOCKET_SIGNALING より）。そのため、この構成で offer を送信することはできず、実際に問題が発生する可能性はない。よって close する。
