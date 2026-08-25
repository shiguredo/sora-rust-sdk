# preference validation の bare 検証を削除し factory pass-through を明文化する

- Priority: Medium
- Created: 2026-08-13
- Completed: 2026-08-17
- Branch: feature/refactor-preference-validation-and-factory-passthrough
- Polished: {YYYY-MM-DD}

## 目的

`validate_video_codec_preference` から bare `SdpVideoFormat` を `resolve_sdp_format` へ渡す重複検証を削除し、`is_supported` を preference validation の source of truth にする。
併せて `SoraVideoEncoderFactory::create` / `SoraVideoDecoderFactory::create` が `capability.resolve_sdp_format` の返り値を `create_video_encoder` / `create_video_decoder` にそのまま渡す pass-through 挙動を production コメントで固定し、回帰テストを追加する。

## 優先度根拠

Medium。
本 issue 単独では user-visible な挙動変化はゼロ（既存 capability はすべて default `is_supported` を使い、bare `resolve_sdp_format` の判定と等価な結果を返すため）。
一方で、codec 固有の required parameter を capability 側で advertise する後続対応（issue 0140 / 0141 / 0097）は、この重複検証を残したままでは preference validation で拒否されて実装できない。
本 issue はその土台を、user-visible な挙動変化を伴わずに独立して整備する structural refactor。

## 現状

`src/video_codec_preference.rs` の `validate_codec` は、`is_supported` の判定に加えて、以下の bare `SdpVideoFormat` を `resolve_sdp_format` に渡す重複検証を行っている。

```rust
let requested = SdpVideoFormat::new(
    codec
        .codec_type()
        .as_str()
        .expect("known codec type must be converted to codec name"),
);
if capability
    .resolve_sdp_format(codec.direction(), requested.as_ref())
    .is_none()
{
    return Err(Error::InvalidVideoCodecPreference {
        reason: format!(
            "codec format not found: codec_preference={}, codec_capability={}",
            Json(codec),
            codec_capability_summary(capability, codec.codec_type())
        ),
    });
}
```

`VideoCodecCapability::is_supported` のデフォルト実装は、bare `SdpVideoFormat` を組み立てて `resolve_sdp_format` に渡し解決可否で判定する。
`resolve_sdp_format` のデフォルトは `get_supported_formats` との fuzzy match。
したがって現在の全 capability（`InternalVideoCodecCapability` / `InternalAppleVideoCodecCapability` / `AmfVideoCodecCapability` / `NvCodecVideoCodecCapability` / `OpenH264VideoCodecCapability` / `V4l2VideoCodecCapability` / `VplVideoCodecCapability` / `Mp4PassthroughVideoCodecCapability`）は default `is_supported` を使用しており、`validate_codec` の重複検証は `is_supported == true` なら必ず成功する dead check になっている。

`src/video_codec.rs` の `SoraVideoEncoderFactory::create` と `SoraVideoDecoderFactory::create` は、以下の pass-through 実装になっている。

```rust
let resolved = capability.resolve_sdp_format(direction, format)?;
capability.create_video_encoder(env, resolved.as_ref())  // Decoder も同様
```

現行の全 capability の `create_video_encoder` / `create_video_decoder` は `format` の parameter を見ておらず、この pass-through が壊れても現行の user-visible な挙動には現れない。
一方、後続対応（issue 0140 の identity 照合、issue 0141 の H.264 profile-level-id、issue 0097 の AV1 profile / level / tier）は、negotiated 済み parameter が `create_video_encoder` に届くことを前提とする。

## 設計方針

### 対象範囲

- `src/video_codec_preference.rs` の `validate_codec` の bare 検証削除のみを対象とする
- `src/video_codec.rs` の `SoraVideoEncoderFactory::create` / `SoraVideoDecoderFactory::create` は code 変更しない（コメント追加と回帰テストのみ）
- `is_supported` の trait デフォルト実装は変更しない
- capability の追加や既存 capability の `is_supported` override は本 issue で行わない（0140 / 0141 / 0097 の担当）
- `SdpVideoFormat` の unused import 除去などの周辺整理は必要最小限にとどめる

### `validate_codec` の bare 検証削除

`validate_codec` を以下の形にする。

- 対象 capability を `find_capability` で取得
- `is_supported(Encoder, codec_type)` と `is_supported(Decoder, codec_type)` を取得
- 指定方向で supported なら Ok、そうでなければ既存の `Error::InvalidVideoCodecPreference` を返す
- 削除対象は「bare `SdpVideoFormat` を組み立てて `resolve_sdp_format` に渡し、`None` なら reject」の分岐だけ
- 削除理由（`is_supported` を source of truth にする、codec 固有 `is_supported` override を capability 側で導入可能にする）を日本語コメントで残す
- `SdpVideoFormat` の import が孤立した場合は削除する（`use shiguredo_webrtc::{SdpVideoFormat, VideoCodecType}` → `use shiguredo_webrtc::VideoCodecType`）
- `codec_capability_summary` が他の分岐でも使われている場合は残し、使われなくなった場合は削除する

### factory pass-through の明文化

`SoraVideoEncoderFactory::create` と `SoraVideoDecoderFactory::create` に日本語コメントを追加する。

- `capability.resolve_sdp_format` の返り値（negotiated 済み format）を `create_video_encoder` / `create_video_decoder` にそのまま渡す挙動を明記する
- この pass-through が codec 固有 parameter を保持する経路として、MP4 passthrough（0140）や後発の profile-level-id 対応（0141）・AV1 profile 対応（0097）が前提とすることを記載する
- bare な入力 format を capability に渡さず、必ず resolve を通す挙動を維持する旨を明記する

### factory pass-through の回帰テスト

`src/video_codec.rs` の test module に、pass-through 挙動を回帰保護する test を追加する。

- テスト用の capability を定義し、`resolve_sdp_format` で bare 入力を無視して常に parameter 付きの format を返すようにする
- `create_video_encoder` が受け取った format の parameter を `Arc<Mutex<Option<HashMap<String, String>>>>` に記録する
- `SoraVideoEncoderFactory::create` を bare な `SdpVideoFormat::new("H264")` で呼び、記録された parameter が `resolve_sdp_format` が付けた値と一致することを確認する
- Decoder factory は本 issue の回帰対象に含めない（現行 capability に Decoder side の parameter 依存がない）

## 変更対象

- `src/video_codec_preference.rs`
- `src/video_codec.rs`
- `CHANGES.md`

## 完了条件

- `validate_codec` から bare `SdpVideoFormat` を組み立てて `resolve_sdp_format` に渡す分岐が削除される
- 既存の `validate_video_codec_preference` の unit test（`validate_succeeds_with_supported_capabilities` など計 8 件）が変更なく通過する
- `SdpVideoFormat` の import が `video_codec_preference.rs` から除去される（孤立した場合）
- `SoraVideoEncoderFactory::create` / `SoraVideoDecoderFactory::create` に pass-through 挙動と後続対応の前提を記載した日本語コメントが追加される
- 新規回帰テスト `encoder_factory_passes_resolved_format_to_create_video_encoder`（相当名）が追加され、bare 入力に対して `resolve_sdp_format` が付けた parameter が `create_video_encoder` まで届くことを検証する
- 本 issue の unit test は mock / stub、sleep、`#[ignore]`、外部 command、ネットワークを使用しない
- `cargo test --workspace` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションの `### misc` サブセクションに変更概要を追記する（機能に直接影響しないリファクタリングのため、`[CHANGE]` / `[ADD]` / `[UPDATE]` / `[FIX]` は使わない）
- production log は英語、コメントとテストの assertion message は日本語にする

## 解決方法

### 実装

- `src/video_codec_preference.rs` の `validate_codec` から「bare `SdpVideoFormat` を組み立てて `capability.resolve_sdp_format` に渡し、`None` なら reject」する分岐を削除し、`is_supported` を preference 検証の source of truth に一本化した
- 削除により孤立した `use shiguredo_webrtc::SdpVideoFormat` の import と `codec_capability_summary` ヘルパーを撤去した
- 削除方針とその根拠（`is_supported` を source of truth にすること、コーデック固有 `is_supported` override を capability 側で導入可能にすること）を日本語コメントとして `validate_codec` に残した
- `src/video_codec.rs` の `SoraVideoEncoderFactory::create` / `SoraVideoDecoderFactory::create` の本体挙動は変更せず、`capability.resolve_sdp_format` の返り値を `create_video_encoder` / `create_video_decoder` にそのまま渡す pass-through 挙動と、後続の MP4 パススルー・コーデック固有 required parameter 対応がこの経路に依存する旨を日本語コメントで固定した

### テスト

- `validate_succeeds_when_supported_even_if_resolve_sdp_format_is_none`: `resolve_sdp_format` が常に `None` を返す capability でも、`is_supported` が `true` なら `validate_video_codec_preference` が成功することを確認する
- `encoder_factory_passes_resolved_format_to_create_video_encoder`: `SoraVideoEncoderFactory::create` を bare な `SdpVideoFormat::new("H264")` で呼び、`resolve_sdp_format` が付けた `packetization-mode=1` が `create_video_encoder` の受信 format に届いていることを、テスト用の `RecordingCapability` に記録された parameter で確認する
- `src/testing.rs` の共有 `TestVideoCodecCapability` に `without_sdp_format_resolution()` builder を追加し、`resolve_sdp_format` が常に `None` を返す capability を回帰テストから組み立てられるようにした（先行マージされた refactor で `TestVideoCodecCapability` が共有モジュールに移動していたため、後続 merge 時にこちらへ吸収した）

### CHANGES.md

- `### misc` サブセクションに `[UPDATE] ビデオコーデック preference の可否判定を is_supported に一本化する` を追加し、削除した検証の内容と MP4 パススルーのユースケースで具体的にどう詰まるかを順序付きリストで説明した
