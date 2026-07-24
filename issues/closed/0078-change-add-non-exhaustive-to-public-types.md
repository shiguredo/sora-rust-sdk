# 全公開 enum / struct に `#[non_exhaustive]` を付与する

- Priority: High
- Created: 2026-07-24
- Completed: 2026-07-24
- Model: Opus 4.7
- Branch: feature/change-add-non-exhaustive-to-public-types
- Polished: {YYYY-MM-DD}

## 目的

公開型に `#[non_exhaustive]` が一切付与されていない (`grep -rn "non_exhaustive" src/` が 0 件)。将来バリアント / フィールド追加が確実に発生する型に付与し、SemVer 上の破壊的変更なしで拡張できるようにする。

## 優先度根拠

High。**公開 API 破壊的変更のため canary 期間中に確定必須**。特に `Error` は feature フラグでバリアントを追加する既存パターンがあり、`#[non_exhaustive]` 非付与のまま正式リリースすると、以降のバリアント追加がすべて破壊的変更になる。

## 現状

`src/` 全体で `#[non_exhaustive]` が 0 件。以下は将来変更が発生する可能性が高いのに付与されていない:

- `enum Error` (30+ バリアント、feature 追加で必ず増える)
- `enum Video`, `enum Audio`, `enum RpcResponse`
- `enum AdmConfig`, `enum SignalingType`, `enum SignalingDirection`, `enum Role`, `enum CodecDirection`, `enum VideoCodecType`, `enum AudioCodecType`
- `struct VideoVP9Params`, `struct VideoH264Params`, `struct VideoH265Params`, `struct VideoAV1Params`, `struct AudioOpusParams`
- `struct ConnectDataChannel`, `struct ForwardingFilter`, `struct ForwardingFilterRule`, `struct ProxyInfo`
- `struct RpcRequestOptions`, `struct Mp4EncodedSample`
- `struct SoraConnectionContextConfig`, `struct LibcameraVideoCapturerBuilder`

## 設計方針

1. 上記のすべての公開 enum / struct に `#[non_exhaustive]` を付与する。
2. `#[non_exhaustive]` を付けると、ユーザーは以下ができなくなる:
   - Enum の網羅的 match (`_ =>` が必須になる)
   - Struct の literal 構築 (`SomeStruct { field1, field2 }`)。代わりに Default + update syntax や新設の Builder を使う。
3. `Default` が実装できる struct には `Default` を追加する (`AudioOpusParams` など既に derive されていれば OK)。
4. `Struct` にフィールドが 1 個しか無く、コンストラクタが 1 種のみのもの (例: `PreferenceCodec` など内部の狭い型) は判断次第で除外可。
5. rustdoc / SKILL.md にサンプルコードで `#[non_exhaustive]` に対応した書き方 (`Default::default()` + update syntax) を掲載する。

## 完了条件

- 上記の公開型すべてに `#[non_exhaustive]` が付与されている。
- 既存のユーザーコード (sumomo / e2e-tests / tests) がビルドできる (対応が必要な箇所は修正する)。
- SKILL.md / README のサンプルコードが `#[non_exhaustive]` に対応した書き方に更新されている。
- `cargo test --workspace` と `cargo clippy --workspace --all-features -- -D warnings` が通る。

## 解決方法

本 issue の方針 (全公開型への `#[non_exhaustive]` 一斉付与) は shiguredo-rust プロジェクト規約と直接衝突し、かつ親 issue で既に決着済みであることが判明したため、コード変更を行わず closed にする。

### 規約による禁止

`shiguredo-rust` スキル (`~/.claude/skills/shiguredo-rust/SKILL.md:37-40`) は `#[non_exhaustive]` の使用を明示的に禁止している。

- 利用側で match の網羅性チェックの恩恵が失われる
- 将来 variant や field を追加するときは素直に破壊的変更として扱うべき
- どうしても必要な場合は個別に許可を得ること

本 issue の設計方針は「公開型に `#[non_exhaustive]` を一斉付与する」であり、規約の運用意図 (網羅性チェックの維持) と真っ向から対立する。

### 親 issue での既決着

`closed/0020-other-prepare-stable-release-2026-1-0.md:59` の Must 派生 M3 として、同じ「公開 API への `#[non_exhaustive]` 一斉付与」が正式リリースブロッカーとして検討済みであり、`[x] M3. 公開 API への #[non_exhaustive] 一斉付与（shiguredo-rust 規約の #[non_exhaustive] 禁止により対応しない）` と、規約により対応しないという結論が明記されている。同ファイル L31 にも「`#[non_exhaustive]` は規約により付与しない方針のまま（M3）」と再掲されている。

本 issue はこの決着済みの決定を独立に再起票してしまったもので、実施すれば規約違反となり、実施しなくても closed/0020 の重複判断と一致するため、いずれにせよ open のまま残す価値がない。

### 個別型に対する将来的な検討

もし将来「特定の公開型 (例: `Error` の feature 追加パターン) に限って `#[non_exhaustive]` を付ける許可を取りたい」という判断が必要になった場合は、対象型を絞った別 issue として個別に起票し、shiguredo-rust 規約の例外条項 (「どうしても必要な場合は許可を得ること」) を根拠に個別許可を得るプロセスを踏む。本 issue の「全公開型に一斉付与」というスコープでは再オープンしない。
