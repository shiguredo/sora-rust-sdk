# README の MP4 パススルー表記を実装と一致させる

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/fix-readme-mp4-audio-description
- Polished: {YYYY-MM-DD}

## 目的

README の MP4 パススルー機能の説明を実装と一致させ、音声送信を謳う誤表記を修正する。

## 現状

README の特徴一覧と「MP4 無変換送信」セクションが音声のパススルー送信を謳っているが、実装はビデオトラックのみを抽出しており音声は無視される。自プロジェクトの `docs/INPUT_MP4.md` と直接矛盾する。

- README の特徴一覧: 「MP4 ファイルから無変換での音声・映像送信対応」
- README の MP4 無変換送信セクション: 「MP4 ファイルに含まれる音声・映像トラックをデコード/エンコードを挟まず、そのまま Sora に送信できる独自機能です」
- 実装: `src/video_codecs/mp4.rs` の `Mp4SampleReader::new_inner` は「最初に見つかったビデオトラックを使用する (音声トラックは無視)」
- `docs/INPUT_MP4.md`: 「映像のみ送信する (音声は無視される)」

## 設計方針

- README の該当箇所を「映像のみ」の記載に修正し、`docs/INPUT_MP4.md` と一致させる
- 必要に応じて音声が対象外である理由 (パススルーで音声を扱わない設計) を 1 行で補足する

## 完了条件

- README に「音声・映像送信」を謳う記述が残っていない
- README と `docs/INPUT_MP4.md` の記述が一致する
- 実装の変更はしない

## 変更対象

- `README.md`
