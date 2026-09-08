# sumomo の MP4 パススルーで H.264 の h264_params を自動補完する

- Created: 2026-09-08
- Completed: 2026-09-08
- Branch: feature/add-sumomo-mp4-h264-params
- Polished: {YYYY-MM-DD}

## 目的

sumomo の `--input-mp4` で H.264 を送るとき、connect の `h264_params.profile_level_id` を
MP4 の avcC 由来の実値で自動補完し、Sora offer の `profile-level-id` と bitstream の不一致による
video m-line reject を防ぐ。

## 現状

- issue 0141 で SDK 側の `Mp4PassthroughVideoCodecCapability` は required SDP format に
  `profile-level-id` を広告する
- 一方 Sora は offerer のため、クライアント answer 側の capability だけでは足りない。
  connect の `h264_params` が無いと、Sora が載せる offer の PLID が bitstream とずれることがある
- `examples/sumomo/src/main.rs` の `video_from_codec_type` は H.264 でも常に
  `Video::new_h264(bit_rate, None)` を渡し、`h264_params` を載せない
- `apply_video_options` は MP4 から得た `VideoCodecType` だけを使い、reader の PLID までは渡さない
- SDK の `VideoH264Params` / `Video::new_h264` は用意済み。`docs/SUMOMO.md` の
  `--video-h264-params` CLI は未実装のままで、本 issue の対象外とする

## 設計方針

- 送信方向かつコーデック付き `Video` を connect に載せるときだけ、
  `Mp4SampleReader::passthrough_capability()` の encoder format から `profile-level-id` を取り、
  `VideoH264Params { profile_level_id: Some(...), b_frame: None }` を補完する
- H.264 以外は補完しない。`--video false` / RecvOnly では補完もログも出さない
- H.264 なのに format / PLID が無い場合は reader / capability の不変条件破壊として扱う
- `--video-h264-params` CLI の追加は行わない（MP4 実値からの自動補完に限定）
- Sora 側で `signaling_h264_params` が有効であることは利用前提として docs に注記する

## 完了条件

- H.264 の `--input-mp4` 送信で connect の `h264_params.profile_level_id` が MP4 実値になる
- 非 H.264 MP4、および `--video false` / RecvOnly では `h264_params` を補完しない
- 上記を fixture ベースのテストで固定する
- `docs/INPUT_MP4.md` / `docs/SUMOMO.md` に自動補完と Sora 前提を注記する

## 変更対象

- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/tests.rs`
- `docs/INPUT_MP4.md`
- `docs/SUMOMO.md`

## 解決方法

### 実装

- `h264_params_from_mp4_passthrough` を追加し、H.264 のときだけ
  `passthrough_capability` の encoder format から `profile-level-id` を取り
  `VideoH264Params` を返すようにした
- `apply_video_options` は `Mp4SampleReader` を受け取り、送信方向かつコーデック付き
  `Video` を載せるときだけ補完する。`--video false` / RecvOnly では補完もログも出さない
- `video_from_codec_type` の H.264 分岐で `Video::new_h264` に `h264_params` を渡すようにした
- `docs/INPUT_MP4.md` の sumomo 節と `docs/SUMOMO.md` に自動補完を注記し、
  INPUT_MP4 では `signaling_h264_params`（デフォルト無効）が必要な旨も書いた
- `CHANGES.md` に `[ADD]` を追記した

### テスト

- H.264 fixture で `profile_level_id=640015` が補完され、connect 用 Video JSON に載ることを固定した
- AV1 fixture では `h264_params` が `None` になることを固定した
- `--video false` / RecvOnly のゲートは実装で守り、境界表によるレビューで確認した。
  Builder が opaque なため、ゲート単体の fixture 切り出しはコストに見合わず見送った
