# 0044 SoraConnection のコールバックをトレイト化する で ubuntu-22.04 の proxy_sendrecv テストが SIGSEGV でクラッシュする

- Priority: High
- Created: 2026-07-06
- Completed: YYYY-MM-DD
- Model: DeepSeek V4 Pro
- Branch: feature/fix-callback-to-trait-sigsegv
- Polished: YYYY-MM-DD

## 目的

`feature/change-callback-to-trait` ブランチ (0044) で ubuntu-22.04 CI の e2e-tests `proxy_sendrecv` が SIGSEGV (signal 11) でクラッシュするようになった。develop ブランチでは発生しておらず、0044 の変更によるリグレッションである。0044 の develop マージを進めるためにはこの問題を解消する必要がある。

## 優先度根拠

- SIGSEGV はメモリ違反であり、即座に対処が必要な重大バグ
- ubuntu-22.04 環境でのテスト失敗により CI が通らず、0044 を develop にマージできない
- 他プラットフォーム全 9 ジョブ (ubuntu-22.04 以外) は同一 push で success のため、環境固有の要素が絡む intermittent failure の可能性がある

## 現状

### 発生状況

`feature/change-callback-to-trait` ブランチの push による CI run [28789292988](https://github.com/shiguredo/sora-rust-sdk/actions/runs/28789292988) で発生:

- 失敗ジョブ: `ci (ubuntu-22.04)`
- 失敗テストバイナリ: `e2e-tests` の `proxy_sendrecv`
- 同ブランチの直前に実行された `test_openh264_sendrecv` は ok

ログ抜粋:

```
(sora_sdk::connection.rs:972): Received disconnect request
(sora_sdk::connection.rs:975): DataChannel 'stats' closed
(sora_sdk::connection.rs:975): DataChannel 'push' closed
(sora_sdk::connection.rs:975): DataChannel 'rpc' closed
(sora_sdk::connection.rs:975): DataChannel 'signaling' closed
(sora_sdk::connection.rs:975): DataChannel 'notify' closed
(sora_sdk::connection.rs:1342): Shutting down
error: test failed, to rerun pass `-p e2e-tests --test proxy_sendrecv`

Caused by:
  process didn't exit successfully: `.../proxy_sendrecv-aa2e5ea975b47c2c` (signal: 11, SIGSEGV: invalid memory reference)
```

"Shutting down" は `run()` の最後 (connection.rs:1342) で出力される。SIGSEGV は `run()` が `Ok(())` を返した後の、`SoraConnection` の Drop 処理中に発生していると推定される。"Shutting down" から SIGSEGV までの間 (~550ms) は `PeerConnection` や `PeerConnectionObserver` の C++ デストラクタによる後始末に費やされていると考えられる。

### 非発生確認

- `develop` ブランチの CI run [28788164513](https://github.com/shiguredo/sora-rust-sdk/actions/runs/28788164513): ubuntu-22.04 の `proxy_sendrecv` は success
- `develop` ブランチの CI run [28788164513] の ubuntu-22.04 では 19 件の "Shutting down" が正常に記録され、SIGSEGV は発生していない

### 0044 の主な変更点

0044 は以下の破壊的変更を含む:

1. **コールバックの統合**: 12 個の独立した `Box<dyn Fn(...) + Send>` を 1 個の `Box<dyn SoraConnectionEventHandler + Send>` に統合
2. **イベント発火タイミングの変更**: `on_track` / `on_remove_track` を `PeerConnectionObserver` 内の直接コールバック呼び出しから `SoraEvent` mpsc チャネル経由の非同期通知に変更
3. **所有権の移動**: `event_handler` を `SoraConnectionBuilder` (config フィールド) から `SoraConnection` 構造体の直接フィールドに移動
4. **DataChannelMessageCallbacks の廃止**: 5 個のコールバックを個別に `handle_datachannel_message` へ受け渡していたのを、`handler` 単体の値受け渡しに変更

### 推定される原因

`run()` 内での変数 drop 順序の変化が疑われる:

- **旧コード**: 12 個のコールバック box は `SoraConnectionBuilder` (config) が `SoraConnection` の最終フィールドとして保持。`SoraConnection` の Drop 時に config が最後に破棄されるため、`pc` (PeerConnection) や `pc_observer` の C++ デストラクタが後始末でオブザーバーコールバックを発火しても、コールバック box はまだ有効
- **新コード**: `handler` (単一の `Box<dyn SoraConnectionEventHandler>`) は `run()` のローカル変数となり、`SoraConnection` の Drop より **先に破棄** される。`PeerConnection` の C++ デストラクタが `pc_observer` を介して `on_track` 等を発火した場合、コールバックオブジェクトは既に解放されている

加えて、`on_track` / `on_remove_track` の非同期化により、`SoraEvent::Track(RtpTransceiver)` が mpsc チャネル経由で送信されるようになった。これが Drop 中にチャネルの receiver 側が既に解放されている状況で送信された場合、`RtpTransceiver` オブジェクトの構築時に partially-destroyed な C++ ポインタにアクセスして use-after-free を起こしている可能性がある。

## 設計方針

1. まず `shiguredo_webrtc` の `PeerConnectionObserver` / `PeerConnection` の Drop 時の挙動を確認し、デストラクタ中にオブザーバーコールバックが発火しうるか調査する
2. 発火しうる場合、以下のいずれかの対策を検討する:
   - `event_handler` の解放を `SoraConnection` の Drop 後まで遅延させる (明示的な Drop 順序の制御)
   - Drop パスではオブザーバーコールバックを無視する (フラグによる gating)
   - `run()` 終了時に `pc.close()` を明示的に呼び出し、コールバック発火が終わった後に handler を解放する
3. 対策後、ubuntu-22.04 CI で再現確認を行う
4. 再現しない場合は一時的に CI を rerun し、intermittent な再現率を評価する

## 完了条件

- `feature/change-callback-to-trait` ブランチの ubuntu-22.04 CI で `proxy_sendrecv` テストが SIGSEGV なく成功すること
- 他のプラットフォームでリグレッションがないこと

## 解決方法

(調査後に記述)
