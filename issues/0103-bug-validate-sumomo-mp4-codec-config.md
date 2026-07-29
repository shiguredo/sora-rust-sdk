# sumomo の MP4 codec 設定を整合させる

- Priority: Medium
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-sumomo-mp4-codec-config
- Polished: {YYYY-MM-DD}

## 目的

MP4 passthrough capability、手動 codec implementation、シグナリングへ指定する codec を一致させ、事前 encode 済み frame を誤った encoder へ渡さない。

## 優先度根拠

Medium。特定の CLI option 組み合わせが必要だが、接続成功後に映像が送信できない、または誤った codec として通知される。

## 現状

`build_context_config` は MP4 passthrough preference を先に追加し、後から手動 codec implementation を merge する。
同一 codec の後勝ち規則により passthrough が上書きされる。
`prepare_mp4_state` が取得した実 codec と CLI の video codec type は独立して扱われる。

## 設計方針

- MP4 入力時は passthrough implementation を必須にする
- MP4 の実 codec から signaling codec を決定する
- 明示 CLI 指定を許可する場合は実 codec との一致を検証する
- 両立しない手動 implementation を argument validation で拒否する

## 完了条件

- MP4 native frame が passthrough encoder だけへ渡される
- signaling codec が MP4 の実 codec と一致する
- 不整合な option 組み合わせが接続前に失敗する
- 対応する各 codec の実 MP4 を使ったテストがある
