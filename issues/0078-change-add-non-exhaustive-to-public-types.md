# 全公開 enum / struct に `#[non_exhaustive]` を付与する

- Priority: High
- Created: 2026-07-24
- Completed: {YYYY-MM-DD}
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
