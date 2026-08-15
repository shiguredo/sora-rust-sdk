# 切断時にサーバーが WebSocket を RST で切っていると close handshake エラーで disconnect が失敗する

- Priority: High
- Created: 2026-08-16
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-close-handshake-error-on-user-disconnect
- Polished: 2026-08-16

## 目的

ユーザー主導の切断 (`SoraConnectionHandle::disconnect`) で `SoraConnection::run` が、既にサーバーに RST で切断された WebSocket への close handshake 書き込みエラーを `Err` として返し、正常な切断なのに `disconnect_and_wait` が失敗する問題を修正する。

## 優先度根拠

High。

- CI の macOS self-hosted ジョブで `e2e-tests` の `messaging` テストが 1 回失敗している (CI run [31841177792](https://github.com/shiguredo/sora-rust-sdk/actions/runs/31841177792))。同じコード経路のレースで、環境・タイミング次第で再発する
- ユーザーが切断を要求したにもかかわらず `run()` が `Err` を返すため、SDK の公開 API として誤った挙動である (後述のとおり、既存コードのコメント自身がこの挙動を問題として認識している)
- 修正自体はフラグの追加とエラー吸収条件の拡張のみで、リスクが低い (close handshake の関数抽出は挙動不変のリファクタリング)
## 現状

### 発生した CI 失敗

CI run [31841177792](https://github.com/shiguredo/sora-rust-sdk/actions/runs/31841177792) の `ci-self-hosted (self-hosted, macOS, ARM64, Apple-M1, --features openh264)` ジョブで、`e2e-tests/tests/messaging.rs` の `test_messaging_sendrecv` が失敗した。

```
thread 'test_messaging_sendrecv' panicked at e2e-tests/tests/messaging.rs:143:10:
クライアント 2 の disconnect に失敗しました: Io(Os { code: 32, kind: BrokenPipe, message: "Broken pipe" })
```

### 失敗時のタイムライン (CI ログから確認)

1. クライアント 1 の `disconnect_and_wait` が成功し、サーバーへ disconnect メッセージが送信される
2. その切断を契機に、サーバーがクライアント 2 の WebSocket を RST で切断する
3. クライアント 2 の run ループは WebSocket のクローズを検知する前に `SoraConnectionCommand::Disconnect` を処理して抜ける
4. run ループ後の close handshake 処理 (現行は `SoraConnection::run` 内のインラインコード) で Close フレームを書き込もうとし、dead socket への書き込み (`flush_ws_output` → `ClientStream::write_all`) が BrokenPipe (macOS) / ConnectionReset (Linux) で失敗する
5. `e2e-tests/src/test_connection.rs` の `disconnect_and_wait` が `run()` のエラーを失敗として返すため、テストが panic する

### 根本原因

`SoraConnection::run` の終了フェーズの close handshake 処理は、close handshake 中の I/O エラーを次の 2 経路で warning に落として `Ok(())` を返す。

- DataChannel 経由の server Close による終了 (`server_close_received`)
- switched 後の ignore 構成 (`switched_ignore_disconnect_websocket && use_data_channel_signaling`)

ユーザー主導の切断 (`SoraConnectionCommand::Disconnect`) はこの吸収条件に含まれていないため、エラーがそのまま `Err` として `run()` の返り値になる。

なお、close handshake 処理の直上のコメント (websocket_closed によるスキップの説明) には「死んだソケットへの I/O が失敗し、**ユーザー主導の正常切断にもかかわらず** run() が Err を返してしまう」ため close handshake をスキップすると記載されており、この挙動が問題であることは既存コードのコメントでも認識されている。この挙動をバグとして扱うことは既存コードの意図と一致する。

### 再現性

サーバーがクライアント 2 の WebSocket を RST で切断するタイミングと、クライアント 2 の close handshake の書き込みタイミングのレースであり、低頻度で発火する。macOS の CI で観測されたが、コード上は OS 非依存のバグである (エラーの詳細は macOS で BrokenPipe、Linux で ConnectionReset になるだけで、`Err` が返る点は同一)。

### 決定的な再現テスト

`close_websocket_handshake` を `run()` から関数として切り出してから、`close_websocket_handshake_does_not_fail_user_disconnect_on_dead_socket` を追加する。このテストは本 issue の調査で作成したもので、現在は失敗するレッドテストである (未コミットの調査用 WIP が git stash 上に存在する。実装時はこの内容を参考にしてよい)。実 TCP ペア・実 WebSocket ハンドシェイク・OS の RST で決定的に再現する。

- 実 TCP ペア上で実 WebSocket ハンドシェイクを行い、`WebSocketClientConnection` を `Connected` まで駆動する
- サーバー側ソケットを SO_LINGER=0 でクローズして RST を送る
- 修正前の吸収条件 (`server_close_received=false` / ignore 構成の合成 bool は `false`) で `close_websocket_handshake` を実行すると `Err(Error::Io(...))` を返すため、`Ok(())` を期待するアサーションが失敗する (stash の WIP テストはこの形で合成 bool を `false` で渡している)
- 修正後は、ユーザー主導の切断を表す合成 bool に `true` を渡して実行し、`Ok(())` を返すことを確認する (引数を `true` に変える必要がある)

モックやスタブは使わず、実 TCP・実 WebSocket・OS の RST で再現する。テストで SO_LINGER を設定するため、`socket2` を dev-dependency に追加する。

## 設計方針

- `SoraConnection::run` に、ユーザー主導の切断 (`SoraConnectionCommand::Disconnect`) による終了かどうかを表すフラグ (`user_initiated_disconnect`) を持たせ、Disconnect コマンドの処理で `true` にセットする
- `run()` 内のインライン close handshake 処理を `close_websocket_handshake` 関数に抽出する (挙動不変。レッドテストを書くための前提)
- `close_websocket_handshake` の I/O エラー吸収条件を、`server_close_received` / ignore 構成に加えてユーザー主導の切断まで拡張する
- 吸収条件の合成 (致命かどうかの判定) は呼び出し側の `run()` で行い、`close_websocket_handshake` へは合成した bool を渡す。合成対象は ignore 構成の bool (`switched_ignore_disconnect_websocket && use_data_channel_signaling`) と `user_initiated_disconnect` の 2 つで、`server_close_received` は close handshake 中の server Close フレーム検出による `on_websocket_close` 通知にも使うため別引数のまま残す。個別に渡すと 8 引数になり clippy の `too_many_arguments` に違反するため
- ユーザー主導の切断以外 (サーバーが一方的に切断した場合など) は、従来どおり close handshake の I/O エラーを `Err` として返す挙動を維持する

## 変更対象

- `src/connection.rs` (`close_websocket_handshake` の抽出、`user_initiated_disconnect` フラグ、吸収条件の拡張、レッドテスト追加)
- `Cargo.toml` / `Cargo.lock` (`socket2` の dev-dependency 追加)
- `CHANGES.md`

## 完了条件

- `close_websocket_handshake_does_not_fail_user_disconnect_on_dead_socket` がグリーンになる
- 既存の unit test が全て通過する (現状 189 件。close handshake の抽出は挙動不変であること)
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --features openh264 -- -D warnings` が成功する
- production log は英語、コメントとテストの assertion message は日本語にする
