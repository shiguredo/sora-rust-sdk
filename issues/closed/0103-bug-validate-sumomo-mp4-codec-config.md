# sumomo の MP4 codec 設定を整合させる

- Priority: Medium
- Created: 2026-07-29
- Completed: 2026-08-11
- Model: GPT-5
- Branch: feature/fix-sumomo-mp4-codec-config
- Polished: 2026-08-07

## 目的

MP4 passthrough capability、手動 codec implementation、シグナリングへ指定する codec を一致させ、事前 encode 済み frame を誤った encoder へ渡さない。

## 優先度根拠

Medium。特定の CLI option 組み合わせが必要だが、接続成功後に映像が送信できない、または誤った codec として通知される。

## 現状

`build_context_config` は MP4 passthrough preference を先に追加し、後から手動 codec implementation を merge する。
`VideoCodecPreference::merge` は方向・codec が一致する既存エントリの implementation を上書きする後勝ち規則のため、実 codec を encoder 方向でサポートする手動 implementation を指定した場合（Manual）は preference の実装指定が passthrough へ上書きされる。`video_codec_capabilities` には passthrough と手動の両方の capability が残るが、preference の実装指定が置き換わることで encoder 選択が手動 implementation になる。
`prepare_mp4_state` が取得した実 codec と CLI の video codec type は独立して扱われ、`--video-codec-type` が実 codec と異なっても接続前に失敗しない。

## 設計方針

issue 0096 が `Mp4PassthroughVideoCodecCapability` を codec type ではなく reader から構築する方式へ変更し、0096 / 0097 が MP4 の実 fixture を追加する。
issue 0101 が raw-player 経路のエラー伝播を修正する。
本 issue は 0096 / 0101 と同一ファイル（`examples/sumomo/src/main.rs`、`examples/sumomo/src/tests.rs`）を変更するため、0096 / 0101 完了後に着手する。
着手時は 0096 で確定した reader ベース API の形状（`Mp4PassthroughVideoCodecCapability` の構築方法等）を確認してから、本 issue の設計を適用する。
issue 0102 も `examples/sumomo/src/tests.rs` を変更するため、0102 を先に完了・merge してから本 issue の tests.rs 変更を進める（0102 は High・小規模のため先行して着手できる）。
実 MP4 を使うテストは、完了条件に記載の既存 H.264 fixture で行う（helper は codec type 単位で判定するため、codec ごとの実 fixture は不要）。

実 codec に依存しない新規検証は本 issue に存在しないため、`args.rs` の `validate_args` は変更しない。

### 映像送信の有効性

映像送信が有効（role が SendOnly / SendRecv かつ `--video` が false でない）かは、共通の判定 helper（`video_send_enabled(&Args) -> bool`）に集約する。
codec 決定の helper、`build_context_config` の呼び出し側、既存の `attach_sender_tracks`（現在は `args.video.unwrap_or(true)` + `role.wants_send()` をインラインで持つ）の 3 箇所で同じ helper を使い、判定の乖離を防ぐ。
`attach_sender_tracks` は挙動を変えず helper へ置き換える（判定の単一ソース化）。
`build_context_config` は呼び出し側で計算した `video_send_enabled(&args)` の結果を bool で受け取り、codec 決定 helper は `args` から内部で同 helper を呼ぶ（同じ helper を共有しており、両経路で判定は一致する）。

### シグナリング codec の決定と一致検証

- シグナリング codec は MP4 の実 codec から決定し、`(args, Option<VideoCodecType>) -> Result<Option<Video>>` の pure helper に切り出す
  - 実 codec が `None`（非 MP4）、または映像送信が無効の場合は `Ok(None)` を返し、従来の `apply_video_options` ロジックに委ねる
  - 映像送信が有効な MP4 では、`--video-codec-type` 未指定時は実 codec、指定時は実 codec との一致を検証し、不一致は `Err` にする
  - `apply_video_options` は helper の `Some` / `Err` を優先し、`None` のときだけ従来ロジックを実行する（戻り値は `Result` 化する）
  - 実 codec を `apply_video_options` へ届けるため、`build_connection_builder` / `apply_common_builder_options` のシグネチャへ実 codec を追加する
  - 一致判定は codec type 単位（exact SDP format の広告は 0096 / 0097 の範囲。connect メッセージの `video` フィールドは codec type 単位であり、exact profile の広告は 0096 の reader ベース capability / SDP 側で扱う）
  - `--video-codec-type` は小文字文字列のため、`VideoCodecType` との比較は小文字へ正規化して行う（`apply_video_options` の既存分岐と同じ扱い）
  - helper の `VideoCodecType` は `reader.codec_type()` が返す `shiguredo_webrtc::VideoCodecType` を使う（SDK 独自型との混在を避ける）
  - helper が返す `Video` へは既存の `apply_video_options` と同様に `args.video_bit_rate` を渡し、params（`h264_params` 等）は渡さない（既存分岐と同じ `None`）
  - 送信方向の connect メッセージの `video` フィールドが実 codec を明示するようになる。SendRecv + MP4 では受信方向の codec negotiation も実 codec に制約され得るが、送信 codec を明示するための必然的な帰結であり許容する（受信方向の詳細な negotiation の変更は本 issue の対象外）。SendRecv + MP4 で `--video-codec-type` が実 codec と不一致の場合は接続前に拒否される（`video` フィールドが送受信双方の codec に波及するため）
  - 実 codec を明示すると `video_bit_rate` も connect メッセージへ載り得る。シグナリング差分は `--video-codec-type` 未指定時を前提に、`--video true` 明示時は `Video::new_bool(true)` → codec 明示、`--video` 未指定時は `video` フィールド無し → codec 明示 の 2 通りがある（`--video-codec-type` 指定一致時は既存分岐と同じため差分が無い）。映像送信が無効（`--video false`、RecvOnly）の場合は helper が `Ok(None)` を返して従来どおり `video(false)` になるため差分が無い
- `prepare_mp4_state` は変更しない（既に実 codec を返している。reader ベース化は 0096 の範囲）

### passthrough の選択

- MP4 入力時は、MP4 の実 codec の encoder に passthrough implementation を使う
- 映像送信が有効な場合、MP4 の実 codec の encoder を提供する手動 implementation の選択は接続前にエラーにする（passthrough を上書きするため）
  - 判定は Manual 選択時のみ、選択された手動 implementation のいずれかが実 codec を encoder 方向でサポートするかを、構築した capability の `is_supported(Encoder, codec)` で確認する（passthrough と Auto モードの既定 capability は対象外）
  - 判定ロジックは実 codec（`VideoCodecType`）と選択された手動 capability 一覧を受け取る pure helper に切り出し、単体テスト可能にする（`build_context_config` は reader の `codec_type()` を helper へ渡すだけで、helper 自体は reader に依存しない）
  - helper は拒否時に英語 + オプション名形式のエラー（例: `--video-codec-implementation internal is not allowed with --input-mp4`。拒否された実装名と実 codec を含める）を返し、単体テストで形式を検証できる
  - 拒否テストは実 codec を `VideoCodecType::Vp8` として直接 helper に渡して行う（`internal` の encoder factory は libvpx の VP8 を常に提供するため決定性がある）。このため VP8 の実 MP4 fixture は不要で、「codec ごとの実 fixture は不要」と整合する。前提が崩れた場合に失敗原因が明確になるよう、テスト冒頭で `internal` の `is_supported(Encoder, VideoCodecType::Vp8)` を assert する
  - 既存の手動実装の多くは H.264 encoder を提供する。拒否される組み合わせはビルドに依存する（例: `internal` の H.264 encoder 提供状況は libwebrtc のビルド構成に依存し、提供しないビルドでは H.264 MP4 + `internal` は拒否されず passthrough が正しく選択される）。拒否判定は構築済み capability の `is_supported(Encoder, codec)` の実結果に委ね、特定のビルドを前提にしない。fail-fast による明示を意図しており、拒否される場合はエラーにする
  - 検証は `main` / `run_with_raw_player` が共有する `build_context_config` 内で行い、映像送信の有効性を引数で受け取ってゲートする（シグネチャ変更）
  - 実 codec に依存する検証は MP4 読み込み後に行う
  - 手動拒否判定に使う実 codec は、0096 完了後の reader ベース API に合わせて `build_context_config` へ渡す reader の `codec_type()` から取得する。reader 自体は `attach_sender_tracks` の capturer へも渡すため、`build_context_config` では借用し、move しない
  - raw-player 経路のエラー伝播は issue 0101 の修正を前提とする
  - openh264 は既存の `--input-mp4` + `--openh264-path` 排他チェックで既に到達不能のため、この判定の対象外
  - 手動拒否（`build_context_config`）は codec 不一致（helper）より先に実行されるため、両方該当する場合は手動拒否エラーが先に報告される（`build_context_config` は `build_connection_builder` より先に呼ばれる実行順に従う。両方とも接続前エラーであり、修正して再実行すれば次のエラーが報告される）
  - capability 構築自体が失敗する場合（例: `AmfVideoCodecCapability::new()` の初期化失敗）は構築エラーが先に報告される
- 映像送信が無効（`--video false`、RecvOnly）でも passthrough capability は従来どおり追加し、検証だけがゲートされる
- 手動 implementation の拒否により、映像送信が有効な sendrecv では、実 codec の encoder を提供する手動 implementation を受信デコード専用にも使えない（従来の誤動作を防ぐための制約であり、許容する）
  - passthrough を手動 merge の後に適用して encoder を固定する案（merge 順序変更）も、実 codec の encoder を passthrough に固定でき、手動 implementation の decoder preference が残る利点がある。しかし利用者が `--video-codec-implementation` で明示した手動 encoder 選択が黙って passthrough へ差し替えられる点で、本 issue の基本方針（明示指定は黙って無視しない。既定値だけを黙って上書きする Auto は許容）に反するため採用しない。明示的な fail-fast を優先する
- シグナリング codec の一致（helper）と encoder 選択の一致（`build_context_config`）は別経路であり、両方が揃って初めてバグが解消する

### その他

- エラーは既存の `validate_args` と同じ英語 + オプション名の形式にする

## 設計からの変更点

当初の設計方針（上記）から、実装時に以下の変更を行った。設計方針・変更対象・完了条件は当初の計画として残し、変更理由をここに記す。

### 手動 implementation の fail-fast 拒否 → passthrough 固定 + Decoder 維持

- 当初設計: 実 codec の encoder を提供する手動 implementation の選択は接続前にエラーにする（fail-fast）。passthrough を手動 merge の後に適用する方式は「明示指定を黙って passthrough へ差し替える」ため不採用とした（設計方針 75 行）
- 実装: `--video-codec-implementation` は `--input-mp4` と併用許可し、手動 merge の後に passthrough を適用して Encoder のみ passthrough に固定した。指定された実装は受信 (Decoder) にのみ使われる
- 理由: 一律拒否すると、MP4 使用時に受信デコーダーが使えなくなる（passthrough は Encoder 専用でデコーダーを提供しない）。当初設計が不採用とした方式を「Encoder のみ固定・Decoder は維持」の形で採用した

### video_send_enabled helper による判定の単一ソース化 → 従来の判定を維持

- 当初設計: 映像送信の有効性を `video_send_enabled(&Args) -> bool` に集約し、codec 決定 helper・`build_context_config` 呼び出し側・`attach_sender_tracks` の 3 箇所で共有
- 実装: 従来どおり `attach_sender_tracks` は `role.wants_send()` + `args.video_enabled()` を使う（helper は追加しない）
- 追加対応: RecvOnly + `--input-mp4` で MP4 の実 codec がシグナリングの `video` フィールド（受信設定）に漏れる問題が判明したため、`apply_video_options` で `role.wants_send()` によるゲートを追加し、受信専用では MP4 の実 codec を明示しないようにした

### codec 決定 helper + `--video-codec-type` 一致検証 → 併用拒否

- 当初設計: `(args, Option<VideoCodecType>) -> Result<Option<Video>>` の pure helper で `--video-codec-type` の一致を検証し、不一致は `Err` にする
- 実装: `--input-mp4` と `--video-codec-type` の併用自体を `validate_args` で拒否した（実 codec と一致する正しい指定も含む）。`Args.video_codec_type` を `Option<VideoCodecType>` に正規化し、`video_from_codec_type` で `Video` を生成した
- 理由: MP4 の実 codec はファイルから自動検出されるため `--video-codec-type` の明示指定は冗長であり、不一致時の誤った codec 通知を防ぐため接続前の早い段階で拒否する方が確実

### 依存 issue（0096 / 0102）との関係

- 当初設計: 0096 の reader ベース API 完了後に着手し、0102 を先に完了・merge
- 実装: 0096 は pending のまま、codec type ベース API（`Mp4PassthroughVideoCodecCapability::new(codec_type)`）で実装した。0096 実装時に `examples/sumomo/src/main.rs` / `tests.rs` の追随が必要

### 完了条件のうち未実施の項目

- `video_send_enabled` の単体テスト、手動拒否判定 helper の単体テスト、実 MP4 fixture を使うテスト（完了条件 107-113）は、上記の設計変更（helper 群の廃止・併用拒否化）に伴い実施しない
- `CHANGES.md` の develop セクションへの `[FIX]` 追記（完了条件 116）は、対応方針の判断により未実施

## 変更対象

- `examples/sumomo/src/main.rs`
  - `build_context_config`（0096 完了後の reader ベース API に合わせて調整、映像送信の有効性を受け取り手動 implementation の拒否判定）
  - `apply_video_options`（codec 決定の pure helper を使用、戻り値を `Result` 化）
  - `build_connection_builder` / `apply_common_builder_options`（実 codec の引き回し）
  - `attach_sender_tracks`（`video_send_enabled` を使うよう変更）
  - 映像送信の有効性の共通判定 helper（新規）
  - codec 決定の pure helper（新規）
  - 手動拒否判定 helper（新規）
- `examples/sumomo/src/tests.rs`
  - `build_context_config` の既存 4 テスト（`build_context_config_auto_uses_default_capabilities`、`build_context_config_manual_internal_only`、`build_context_config_rejects_internal_apple_on_unsupported_platform`、`build_context_config_manual_order_prefers_later_selection_on_apple`）のシグネチャ追随
  - `video_send_enabled` の単体テスト（新規）
  - codec 決定の pure helper の単体テスト（新規）
  - 手動拒否判定 helper の単体テスト（新規）
  - `build_context_config` の新規テスト（Auto + MP4 の passthrough 選択、映像送信無効時（`video_send_enabled == false`）の Manual + MP4 で拒否されず構築されること。手動拒否は判定 helper の単体テストで担保する。新規）
  - 実 MP4 を使うテスト（`prepare_mp4_state` → codec 決定 helper → シグナリング codec 一致。新規）
- `CHANGES.md`

## 完了条件

- 映像送信が有効な場合、Auto では MP4 の実 codec の encoder に passthrough implementation が使われ、Manual では実 codec の encoder を提供する手動 implementation が接続前に拒否される（どちらでも passthrough が手動 implementation に上書きされない）
- 映像送信が有効な場合、シグナリング codec が MP4 の実 codec と一致する（codec type 単位）
- 映像送信が有効な場合、`--video-codec-type` の不一致と、実 codec の encoder を提供する手動 implementation の選択は接続前に失敗する（エラーは英語 + オプション名の形式であることも検証する）
- 映像送信が無効（`--video false`、RecvOnly）の場合は従来どおり動作する
- codec 決定の pure helper の単体テストで、`--video-codec-type` の一致 / 不一致・未指定時の実 codec 選択・非 MP4 時 / 映像送信無効時の `Ok(None)` を検証する（不一致の `Err` が英語 + `--video-codec-type` を含む形式であることも検証する）
- 手動拒否判定 helper の単体テストで、`internal` + `VideoCodecType::Vp8` の拒否（`is_supported(Encoder, Vp8)` の assert つき）と、`internal` が encoder として提供しない codec（例: H.265。`is_supported(Encoder, H265)` が false であることを assert）の受理を検証する（拒否・受理の両方の codec 選択はビルド構成に依存するため、冒頭の assert で前提を検証してから判定を進める）
  - 拒否時のエラーが英語 + `--video-codec-implementation` を含む形式であることも検証する
- `video_send_enabled` の単体テストで、SendOnly / SendRecv / RecvOnly と `--video` の None / true / false の組み合わせを検証する
- `apply_video_options` の helper 優先配線を確認する。helper 単体テストで `Err`（codec 不一致）の生成を検証し、`apply_video_options` の `Some` / `Err` 優先配線は `SoraConnectionBuilder` が opaque なため code review で確認する（実コンテキストを起動して配線だけを検証する重量級テストは追加しない）
- `build_context_config` のテストで、Auto + MP4 で passthrough が選択されること、映像送信無効時（`video_send_enabled == false`）に Manual + MP4 でも拒否されず従来どおり構築されることを検証する（手動拒否は判定 helper の単体テストで担保する。実 MP4 は下記と同じ H.264 fixture を reader として使う。`build_context_config` の手動拒否の配線（実 codec の渡し方・`video_send_enabled` によるゲート）は `apply_video_options` と同様に code review で確認する）
- 実 MP4 を使ったテストで、`prepare_mp4_state` から得た実 codec を helper に渡すと、シグナリング codec と一致する Video が得られることを検証する（既存の H.264 fixture `testdata/red-320x320-h264.mp4` を `examples/sumomo/src/tests.rs` から `include_bytes!` の相対パス `../../../testdata/red-320x320-h264.mp4` で参照し一時ファイルへ書き出して `Mp4SampleReader` に渡す。`src/video_codecs/mp4.rs` の既存テストと同方式）
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `cargo clippy -p sumomo --features raw-player --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションへ `[FIX]` と担当者 `@voluntas` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 解決方法

- `Args.video_codec_type` を `Option<String>` から `Option<VideoCodecType>` へ変更し、`parse_args` で vp8/vp9/av1/h264/h265 の 5 値をパース時に変換するようにした
- `--input-mp4` と `--video-codec-type` の併用拒否を `validate_args` に追加した
  - 当初は `parse_args` で拒否し、`--video-codec-implementation` も併用拒否していたが、MP4 使用時に受信デコーダーが使えなくなる問題があったため、実装の併用は許可へ、併用チェック自体は `validate_args` へ移動した
- `validate_args` の `--input-mp4` と `--openh264-path` の排他チェックを削除し、openh264 を受信デコーダーとして併用できるようにした
- `build_context_config` を、MP4 使用時は選択された codec 実装と passthrough の両方を持つ構成へ変更した
  - 送信 (Encoder) の preference は MP4 の実 codec の passthrough のみに固定し、指定された実装は受信 (Decoder) にのみ使われる
- `apply_video_options` に `mp4_codec_type` を渡し、MP4 使用時は実 codec をシグナリングの `video` フィールドへ明示するようにした
  - 受信専用 (RecvOnly) では `role.wants_send()` でゲートし、MP4 の実 codec を受信設定へ波及させない
- `video_from_codec_type` を新設し、`VideoCodecType` からシグナリング用 `sora_sdk::Video` を生成するようにした（`Generic` / `Unknown` はエラー）
- `apply_common_builder_options` を `build_connection_builder` にインライン化した
- `docs/INPUT_MP4.md` から `--video-codec-type` を使う例を削除し、codec 自動検出と併用不可を明記した
- `--input-mp4` と `--video-codec-type` の併用拒否、MP4 使用時に送信エンコーダーが passthrough のみになること・受信デコーダーが維持されることのテストを追加した
