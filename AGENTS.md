# AGENTS

- Premature Optimization is the Root of All Evil
- 一切忖度しないこと
- 常に日本語を利用すること
- 全角と半角の間には半角スペースを入れること
- 絵文字を使わないこと

## レビューについて

- レビューはかなり厳しくすること
- レビューの表現は、シンプルにすること
- レビューの表現は、日本語で行うこと
- レビューの表現は、指摘内容を明確にすること
- レビューの表現は、指摘内容を具体的にすること
- レビューの表現は、指摘内容を優先順位をつけること
- レビューの表現は、指摘内容を優先順位をつけて、重要なものから順に記載すること
- ドキュメントは別に書いているので、ドキュメトに付いては考慮しないこと
- 変更点とリリースノートの整合性を確認すること

## コミットについて

- 勝手にコミットしないこと
- コミットメッセージは確認すること
- コミットメッセージは日本語で書くこと
- コミットメッセージは命令形で書くこと
- コミットメッセージは〜するという形で書くこと

## サンプルについて

- サンプルは **お手本** なので性能と堅牢性を両立させること

## issues について

- 番号が大きい issues から順番に対応すること
- `{seqnum}-{category}-{short-description}.md` という命名規則を守ること
  - seqnum は `issues/SEQUENCE` ファイルの値を使うこと（9999 を超えたら 5 桁にする）
  - issue を新規作成したら `issues/SEQUENCE` の値を +1 して更新すること
  - 例: `0001-bug-fix-parse-error.md`
  - 例: `0002-fmt-enhance-support-for-joins.md`
- 仕様的に対応が難しい場合は issues/pending/ へ移動すること
- 1 issue 完了ごとに 1 コミットすること

### issue が実は解決してなかった場合

- reopen の理由を issue に書いて issues/closed から issues/ に移動すること (git mv を使うこと)

### バグが見つかった場合

- issues/ 以下にバグを markdown 形式で登録すること
- バグは再現手順を明確にすること
- できる限りの情報を

### バグを修正した場合

- issues/ 以下のバグを修正した場合は、修正内容を markdown 形式で記載すること
- issues/closed に移動すること (git mv を使うこと)
- issues/closed に移動するときは issue ファイルに「## 解決方法」セクションを追記し、何をどう修正したかを明記すること

### 設計判断が必要な issue の場合

- 外部依存の追加や設計判断が必要で保留中の issue は `issues/pending/` に置くこと
- issues/pending に移動するときは issue ファイルに pending にした理由を明記すること
- pending の issue は修正せずそのまま残す（close しない）

## テストについて

- pbt 以下に unittest を書かないこと
- unittest は pbt で実現できないものだけを書くこと

## Rust

- 性能より堅牢性を優先すること
- PBT(Property-Based Testing) や Fuzzing で必ずテストを行うこと
- serde_json を使わず nojson <https://github.com/sile/nojson> を使うこと
- clap を使わず noargs <https://github.com/sile/noargs> を使うこと
- anyhow や thiserror を使わないこと
