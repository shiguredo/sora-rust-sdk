# `SoraConnectionHandle::send_message` にラベル検証を追加し、内部 DataChannel の横取りを防ぐ

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-28
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

### 検証方式

`label.starts_with('#')` のような命名規約チェックではなく、ユーザーが `SoraConnectionBuilder::data_channels()` でシグナリング時に指定したラベルセットと照合する。

`SoraConnectionHandle` に `user_data_channel_labels: HashSet<String>` フィールドを追加する。`SoraConnection::new()` (connection.rs:674) 内で `config.data_channels` からラベルを抽出し、ハンドルに保持させる。`send_message` の入口で `self.user_data_channel_labels.contains(label)` を検査し、未登録なら `Error::InvalidDataChannelLabel` を返す。

この方式の利点:
- 命名規約 (`#` プレフィックス) に依存せず、ユーザーの明示的意図と照合するため、規約変更に強い。
- ユーザーが指定していないラベル（内部ラベルだけでなく任意の未定義ラベル）も弾ける。

### 実装ステップ

1. `SoraConnectionHandle` (connection.rs:445) にフィールド `user_data_channel_labels: HashSet<String>` を追加する。

2. `SoraConnection::new()` (connection.rs:674) で `config.data_channels` からラベルを収集する。`config.data_channels` は `Option<Vec<ConnectDataChannel>>` のため、`None` の場合は空の `HashSet` になる（`data_channels()` 未指定時はすべてのラベルが弾かれるが、DataChannel を全く使わないケースでは問題ない）:
   ```rust
   let user_data_channel_labels = config
       .data_channels
       .iter()
       .flat_map(|dcs| dcs.iter().map(|dc| dc.label.clone()))
       .collect::<HashSet<String>>();
   ```
   ハンドル生成時に渡す。現状 `let handle = SoraConnectionHandle { command_tx };` の直接構築もあるため、`command_tx` を先に変数化してから `SoraConnectionHandle { command_tx, user_data_channel_labels }` とする:
   ```rust
   let handle = SoraConnectionHandle {
       command_tx,
       user_data_channel_labels,
   };
   ```

3. `SoraConnectionHandle::send_message` (connection.rs:509) でラベル検証:
   ```rust
   if !self.user_data_channel_labels.contains(label) {
       return Err(Error::InvalidDataChannelLabel {
           label: label.to_string(),
       });
   }
   ```

4. テスト (connection.rs:2573) を追加する。テストから `SoraConnectionHandle` を直接構築するには `mpsc::unbounded_channel::<SoraConnectionCommand>()` で `command_tx` を生成する。ラベル検証失敗は `send_command` 到達前に `return Err(...)` するため、同期コードと同じ感覚でテストできる（`#[tokio::test]` は必要だが、`send_message` の await が即座に完了する）。テストケース:
   - ユーザー指定ラベル `"#chat"` → 通過（ラベル検証を通過し `send_command` に到達する。テストでは下記の「テスト構成上の注意」に従って `command_rx` タスクで `response_tx.send(Ok(()))` を返せば `Ok(())` が返る。`InvalidDataChannelLabel` ではないことを検証する）
   - 内部ラベル `"signaling"` / `"stats"` / `"push"` / `"notify"` / `"rpc"` → `InvalidDataChannelLabel`
   - 空文字列 `""` → `InvalidDataChannelLabel`
   - 未指定ラベル `"#unknown"` → `InvalidDataChannelLabel`
   - `data_channels()` 未指定（空の HashSet）→ すべてのラベルで `InvalidDataChannelLabel`

   テスト構成上の注意:
   - `SoraConnectionHandle::send_message` はラベル検証を通過した後 `send_command` → `command_tx.send(...)` → `rx.await` を実行する。テストでは `command_tx` と対になる `command_rx` を別タスクで受信し、`response_tx.send(Ok(()))` を返すことで通過系のテストがハングしないようにする。モックやスタブは使わず、実際の `mpsc::unbounded_channel` のペアを利用する。
   - `#[cfg(test)]` モジュール (connection.rs:2573) は private フィールドにアクセス可能なため、構造体リテラルで直接構築できる。

5. `SoraConnectionHandle` の全構築箇所（`grep "SoraConnectionHandle {" src/connection.rs` で特定）をフィールド追加に対応させる。現状は `SoraConnection::new()` 内 (connection.rs:676) とテスト `url_getters_return_send_error_after_run_loop_stops` (connection.rs:2717) の 2 箇所。

なお、`send_datachannel_message` (connection.rs:1709) と `send_data_channel_message` (connection.rs:1731) は異なる関数である。前者はテキスト送信用で `send_signaling_message` / `send_stats_message` / RPC 送信から呼ばれ、本検証の影響を受けない。後者はバイナリ送信用で `SendMessage` コマンドハンドラからのみ呼ばれる。検証は `SoraConnectionHandle::send_message` 側に入れるため、両方とも本修正の対象外。

### エラー型

新規の `Error::InvalidDataChannelLabel { label }` を設ける。`DataChannelMissing` は「チャネルが見つからない」意味であり、本件は「チャネルは存在するがラベルが API 契約に違反している」ため意味が異なる。既存の `Error::InvalidVideoCodecCapability { reason }` (error.rs:191) 等の `Invalid*` パターンに揃える。以下の対応が必要:
- `error.rs` の `Error` enum にバリアントを追加する。既存バリアントと同様に doc comment (`///`) を付与する。
- `error.rs` の `Display` impl (error.rs:294) に `InvalidDataChannelLabel` のマッチアームを追加する。メッセージは「シグナリング時に指定されていないラベルです: {label}」等。

### SKILL.md 更新

- SKILL.md:546 の「SDK 内部用ラベル (`signaling` 等) を渡すと `Error::DataChannelMissing` になる」を「シグナリング時に指定していないラベルを渡すと `Error::InvalidDataChannelLabel` になる」に更新する。
- SKILL.md:155 の `send_message` の説明（「`#` プレフィックス DataChannel にバイナリ送信」）も同様に allowlist 方式に合わせて更新する。
- SKILL.md:528 のエラーカタログ表 (`DataChannel / RPC` 行) に `InvalidDataChannelLabel` を追加する。

### rustdoc 更新

`send_message` の rustdoc (connection.rs:506-508) を更新する。「`#` プレフィックス付きラベルのユーザー定義 DataChannel にバイナリデータを送信する」という既存の説明を、allowlist 方式に合わせて「シグナリング時に `data_channels` で指定したラベルの DataChannel にバイナリデータを送信する」に書き換え、エラー条件（違反時は `Error::InvalidDataChannelLabel`）を追記する。

## 解決方法

以下の 2 段階で実装した。

### 初期実装 (2683127)

- `Error::InvalidDataChannelLabel` バリアントを追加し、`Display` 実装を追加。
- `SoraConnectionHandle` に `user_data_channel_labels: HashSet<String>` フィールドを追加し、`SoraConnection::new()` で `config.data_channels` から収集。
- `send_message` 入口で synchronous にラベル検証を行い、未登録ラベルに対して `Error::InvalidDataChannelLabel` を返す。
- 内部ラベル (`signaling`, `stats`, `push`, `notify`, `rpc`) を拒否する 7 ケースの単体テストを追加。

### 設計変更 (bf3c977)

上記の初期実装を根本的に見直し、検証を `SoraConnection` コマンドハンドラ側に移動した。

- `SoraConnectionHandle` から `user_data_channel_labels` フィールドを削除し、`command_tx` のみに戻した。`send_message` の rustdoc を「SDK 内部用ラベルおよび `#` プレフィックスのないラベル、Offer 応答の `data_channels` に含まれていないラベルを渡すと `Error::InvalidDataChannelLabel` を返す」に更新。
- `SendMessage` コマンドハンドラ内で `self.data_channel_configs.iter().any(|c| c.label == label) && label.starts_with('#')` により検証。`data_channel_configs` は Offer / re-offer 受信時に更新される動的な値であり、redirect 後のリセットや re-offer による差し替えに追従する。
- テストを `command_tx + tokio::spawn` 方式に書き換え、コマンドハンドラ相当のロジックで `#` プレフィックスと `data_channel_configs` 照合を再現。
- SKILL.md の記述を新しい方式に合わせて更新。

## 完了条件

- `SoraConnectionHandle::send_message("signaling", ...)` が `Error::InvalidDataChannelLabel` を返し、内部 DataChannel にメッセージが送信されない。
- `SoraConnectionHandle::send_message("#chat", ...)` はユーザー指定ラベルとして登録されていれば送信できる。
- 未指定ラベル（例: `"#unknown"`）も `Error::InvalidDataChannelLabel` で弾かれる。
- `Error::InvalidDataChannelLabel` の `Display` 実装が適切な日本語メッセージを出力する。
- SKILL.md の記述と実装が一致している。
- `cargo fmt --check --all`、`cargo clippy --workspace -- -D warnings`、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` エントリが追加されている。
