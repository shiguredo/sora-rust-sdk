# リリース公開前の検証 gate を強化する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-release-publication-gates
- Polished: {YYYY-MM-DD}

## 目的

誤った tag、未検証 commit、manifest と不一致な version を crates.io へ公開できないリリース手順にする。

## 優先度根拠

High。公開済み crate version は削除できず、誤公開は yank と後続 version でしか回復できない。

## 現状

Release workflow は任意の tag push で起動する。
tag と package version の一致、既定ブランチへの包含、CI 成功、package 検証を確認せず GitHub Release を作成し、その後 publish する。

## 設計方針

- 許容する tag 形式と package version の完全一致を検証する
- tag commit が既定ブランチに含まれることを検証する
- fmt、clippy、test、doc、package の成功を publish の前提にする
- crates.io publish 成功後に GitHub Release を作成するか、失敗時に不整合を残さない順序へ変更する
- workflow の同時実行と再実行を安全にする

## 完了条件

- 誤形式 tag、version 不一致、未検証 commit では publish が実行されない
- 検証失敗時に GitHub Release だけが残らない
- 正式版と canary の両方で release classification が正しい
- `cargo package` の内容と検証 build が release 前に確認される
