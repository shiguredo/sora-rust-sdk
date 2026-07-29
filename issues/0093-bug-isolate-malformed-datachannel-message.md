# 不正 DataChannel メッセージの影響を接続全体へ波及させない

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-malformed-datachannel-message
- Polished: {YYYY-MM-DD}

## 目的

不正な圧縮データ、UTF-8、JSON を含む DataChannel メッセージを受信しても、`SoraConnection::run` 全体を終了させない。

## 優先度根拠

High。リモートから不正メッセージを 1 件送るだけで、正常な PeerConnection と他の DataChannel を含む接続全体を切断できる。

## 現状

main event loop は `SoraConnection::handle_datachannel_message` のエラーを `?` で伝播する。
同関数は zlib、UTF-8、JSON の parse error をそのまま返す。

## 設計方針

- parse error をメッセージ単位で処理する
- label と message type に応じて、破棄、対象 DataChannel の close、接続終了を明示的に分類する
- エラー内容へ受信本文や秘密情報を含めない

## 完了条件

- 不正 zlib、UTF-8、JSON を受信しても接続全体が終了しない
- 対象メッセージが callback や RPC response として処理されない
- 正常メッセージを続けて受信できる
- 各異常入力のテストがある
