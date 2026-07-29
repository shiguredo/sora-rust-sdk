# シグナリング URL の query をログとエラーから除去する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-redact-signaling-url-query
- Polished: {YYYY-MM-DD}

## 目的

シグナリング URL の query に認証情報が含まれる構成でも、その実値がログやエラー表示へ残らないようにする。

## 優先度根拠

High。成功時と失敗時の両方に直接の出力経路があり、ログ集約先へ機密情報が残り得る。

## 現状

シグナリング URL の parse 結果は query を含む path 表現を保持する。
`connect_signaling_urls` は接続試行と成功時にこの path をログ出力し、失敗時には入力 URL 全体をエラー情報へ保存する。

## 設計方針

- ログと `Display` 用に query、fragment、userinfo を含まない安全な URL 表現を用意する
- 接続処理に必要な原値と表示用文字列を分離する
- parse 失敗時も入力文字列をそのまま表示しない

## 完了条件

- query を含む URL の成功・失敗ログに query の実値が含まれない
- aggregate error の表示にも query の実値が含まれない
- URL の host、port、path など診断に必要な情報は安全な範囲で残る
- ダミー値を使った秘匿テストがある
