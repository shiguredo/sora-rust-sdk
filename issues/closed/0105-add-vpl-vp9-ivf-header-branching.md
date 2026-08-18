# VPL の VP9 出力の IVF ヘッダー有無で payload 処理を分岐させる

- Priority: Medium
- Created: 2026-08-01
- Completed: 2026-08-18
- Model: deepseek-v4-flash
- Branch: feature/add-vpl-vp9-ivf-header-branching
- Polished: 2026-08-07

## 目的

VPL の VP9 encoder の出力が IVF ヘッダー付きか raw VP9 かを、byte 列の推測ではなく shiguredo_vpl が設定した値に基づいて判定し、どちらの設定でも正しく payload を処理できるようにする。

## 優先度根拠

Medium。現状の設定では正しく動作しており実害はないが、byte 列の推測による判定を排除して将来の設定変更や wrapper の変更に堅牢にするための対応。
なお「堅牢にする」の範囲は、SDK が shiguredo_vpl へ明示設定した値（要求値）に基づく分岐までであり、wrapper や oneVPL 実装が設定を無視して実際の出力形式を変えた場合の追従（実効値の読み戻し）は含まない（実効値報告は将来課題）。

## 現状

VPL の VP9 encoder は `mfxExtVP9Param::WriteIVFHeaders` の設定で出力が変わり、現在利用している shiguredo_vpl 2026.3.0 は同フィールドを設定しないため oneVPL 実装のデフォルト (ON) が適用される。

- ON (デフォルト): 先頭フレームは IVF ファイルヘッダー (DKIF, 32 byte) + フレームヘッダー (12 byte)、2 フレーム目以降はフレームヘッダー (12 byte) だけが付く（前提として記述。ファイルヘッダーが全フレームに付く場合でも、後述の per-frame DKIF 判定により正しく処理できるため、この構造の実測は本 issue の正しさに依存しない）
- OFF: IVF ヘッダー無しの raw VP9 elementary stream

sora-rust-sdk の `vp9_payload_from_vpl` は、`DKIF` で始まる場合は 32 byte のファイルヘッダーを除去し、その後常に 12 byte をフレームヘッダーとして除去する。

これは現状の出力 (WriteIVFHeaders ON) に対しては正しく動くが、出力が raw VP9 (WriteIVFHeaders OFF) の場合、DKIF 前置の入力では 32 byte 除去後に 12 byte 除去、それ以外では 12 byte 除去が行われ、DKIF 前置で 32〜43 byte の入力はフレームヘッダー不足で拒否され、非 DKIF で 13〜23 byte の入力は除去後に 1〜11 byte が残り、そのまま誤った先頭として渡る。

## 設計方針

- shiguredo_vpl に `Vp9EncoderConfig.write_ivf_headers: bool` フィールドを追加する（oneVPL の `mfxExtVP9Param::WriteIVFHeaders` を明示設定）。`Encoder` は設定した値を返す getter を持つ
  - フィールド型は素の `bool` とし、呼び出し側が必ず指定する（Rust の bool にデフォルトはないため、「デフォルトは oneVPL の既定に合わせ true」は shiguredo_vpl が oneVPL へ未設定時の既定を指す）。SDK は `true` を明示する
  - getter は設定値（shiguredo_vpl が oneVPL へ要求した値）を返す。実効値（`GetVideoParam` による読み戻し）の報告は将来課題（優先度根拠参照）
  - 非 VP9 コーデック時の getter の戻り値は `false` 固定とする（本 SDK の契約上、write_ivf_headers を扱うのは VP9 だけとする。oneVPL には AV1 / VP8 にも `WriteIVFHeaders` が存在するが、本 SDK の VPL 経路で IVF ヘッダーを除去するのは VP9 のみ）。shiguredo_vpl 側の API 契約として定める
  - shiguredo_vpl のリリースを先に完了してから本 issue に着手する（git 依存は使わない）。更新先バージョンを確定し、`Cargo.toml` の `shiguredo_vpl` を更新する。`~2026.3` の tilde レンジは 2026.3.x のパッチリリースのみ許可する（`>=2026.3.0, <2026.4.0`）。write_ivf_headers の追加が 2026.4.0 等のマイナーリリースで出た場合は `~2026.4` 等へのレンジ変更が必須になる
- sora-rust-sdk は `encoder_codec_config` で `Vp9EncoderConfig { profile: Some(Vp9Profile::Profile0), write_ivf_headers: true }` を明示設定し（フィールド追加でコンパイルエラーになるため必須）、`VplVideoEncoder` が rebuild 後に getter から値を取得して保持し、`encode()` で `EncoderCallbackValue` に含めて `handle_vpl_encode_callback` へ渡す
  - 配線が getter 経由なのは、SDK 側が設定値を直接持つのでなく、shiguredo_vpl が oneVPL へ要求した値を単一の情報源とすることで、将来の実効値報告（`GetVideoParam` 読み戻し）への拡張点を確保するため。本 issue の完了条件の範囲では getter の戻り値は SDK が設定した値と一致する
  - `VplVideoEncoder` が保持する値は `rebuild_encoder()` 内で getter から取得して保持する（`init_encode` / `encode` の rebuild 経路の両方で、値が更新されることを保証する）。初期値は `false` とし、rebuild 前に使われることはない（encode は rebuild 後にしか呼ばれない）
  - `EncoderCallbackValue` は全コーデック共通のため、非 VP9 コーデックでは getter の戻り値（`false`）をそのまま載せる
- `vp9_payload_from_vpl(data, write_ivf_headers: bool)` を分岐させる
  - `write_ivf_headers == true`（IVF 付き）: 元データの先頭が `DKIF` で始まる場合（IVF コンテナのファイルヘッダー）は 32 byte を除去し、その後 12 byte のフレームヘッダーを除去する。先頭フレームは 32 + 12 byte 除去、2 フレーム目以降は 12 byte 除去になる
  - `write_ivf_headers == false`（raw）: 無加工で返す（空 payload は拒否する）
- byte 列による「IVF ヘッダー付きか raw か」の分岐判定（推測）を廃止し、報告された `write_ivf_headers` で分岐する
  - IVF 分岐内の `DKIF` チェックは、IVF コンテナの先頭フレームに必ず付く 32 byte ファイルヘッダーの検出であり、決定的なパースであって「IVF か raw か」の推測ではない。DKIF 判定は元データの先頭に対して行う
  - フレーム単位で判定するため、reconfigure（Reset）や encoder 再生成でファイルヘッダーが再出力された場合にも追従できる（状態管理を導入しない）
- 空 payload の拒否は維持し、拒否時のエラーメッセージを stripping に依存しない文言（例: "VP9 payload is empty"）に一般化する。空判定は入力時・各除去ステップ後（ファイルヘッダー除去後・フレームヘッダー除去後）に行い、0 byte 入力でも「ヘッダー不足」ではなく空 payload 拒否になるようにする（境界入力: ちょうど 32 byte の DKIF 付き入力はファイルヘッダー除去後に空、ちょうど 44 byte の DKIF 付き入力とちょうど 12 byte の DKIF なし入力はフレームヘッダー除去後に空になり、いずれも空 payload 拒否になる）。ヘッダー不足のエラーメッセージ（現行の "VP9 IVF file header is truncated" / "VP9 IVF frame header is truncated"）は IVF 分岐内でのみ発生するため現行のまま維持する（stripping の実行自体に言及していない）

## 変更対象

- shiguredo_vpl（別リポジトリ）への API 追加・リリース（前提。設計方針参照。vpl-rs 側の issue / PR を追跡し、API 契約（フィールド名・getter 名・非 VP9 時の戻り値）が本 issue の記述と一致することを着手前に確認する）
- `Cargo.toml` / `Cargo.lock`（`shiguredo_vpl` のバージョン更新）
- `src/video_codecs/vpl.rs`
  - `encoder_codec_config`（`write_ivf_headers` の明示設定）
  - `vp9_payload_from_vpl`
  - `handle_vpl_encode_callback` の VP9 分岐
  - `VplVideoEncoder` / `encode()` / `EncoderCallbackValue`（write_ivf_headers の受け渡し）
  - VP9 payload の単体テスト（既存テストの意味論変更を含む書き換え）
- `CHANGES.md`

e2e-tests の既存 VP9 テスト（`test_vpl_sendrecv` 等）が回帰確認を兼ねるため、e2e-tests は変更しない。

## 完了条件

- `vp9_payload_from_vpl` の単体テストで、IVF 付き（DKIF 付き先頭フレーム: 32 + 12 byte 除去 / 2 フレーム目以降: 12 byte 除去 / DKIF が再出力された場合も 32 + 12 byte 除去）と raw（無加工）の両方、空 payload の拒否（0 byte 入力も「ヘッダー不足」ではなく空 payload 拒否になることを含む）、ヘッダー不足の拒否（DKIF 付き: 32 byte 未満はファイルヘッダー不足、33〜43 byte はフレームヘッダー不足 / 2 フレーム目以降: 12 byte 未満）、raw の短い入力（1 byte / 11 byte / 12 byte）の byte-for-byte 維持を検証する
  - 境界入力（ちょうど 32 byte の DKIF 付き入力・ちょうど 44 byte の DKIF 付き入力・ちょうど 12 byte の DKIF なし入力）は除去後に空になり、空 payload 拒否の文言（例: "VP9 payload is empty"）になることを検証する
  - raw 分岐で `DKIF` で始まる入力も無加工で返すことを検証する（byte 列による「IVF か raw か」の推測が残っていないことの直接検証）
- VPL 実機上の既存 VP9 E2E（`write_ivf_headers: true` の明示設定 ON 経路）が成功することを確認する（stats ベースのためフレーム単位の破損は検出できないが、VP9 経路全体の回帰を確認する。OFF 経路は単体テストで担保する）
  - 確認は VP9 の encoder / decoder 両対応の実機（例: CI の self-hosted Intel-VPL runner）で行い、`vpl_fully_supported_codecs()` に VP9 が含まれること（VP9 が実際に送受信されること）を事前に確認した上で完了とする。VP9 非対応環境ではスキップされ成功扱いになるため、「VP9 経路を実行した」ことが確認できなければ完了扱いとしない
- byte 列による「IVF か raw か」の推測が残っていない（分岐は報告された `write_ivf_headers` で行う）
- `VplVideoEncoder` の getter 取得 → `EncoderCallbackValue` への埋め込み → callback 内の分岐という配線は、`vp9_payload_from_vpl` の単体テストでは検証できないため、code review で確認する旨を PR に明記する（getter が誤って `false` を返す実装だと、OFF 経路の raw 分岐が使われて IVF 付き出力の先頭が壊れる。ON 経路の E2E は stats ベースで `framesDecoded` を要求するため、実際に VP9 が走れば検出されるが、OFF 経路の配線は検証されないためコードレビューで確認する）
- `cargo test --workspace --features vpl` と `cargo clippy --workspace --features vpl --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションへ `[UPDATE]`（shiguredo_vpl の更新）と担当者 `@voluntas` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする（書き換え・新規の VP9 テストに適用）

## 解決方法

初期方針は「shiguredo_vpl に `write_ivf_headers` フィールドを追加し、getter で読み取った値で IVF 付き / raw を分岐して DKIF パースを維持する」だったが、**`write_ivf_headers: false` で出力自体を raw に固定すれば分岐が不要になる**ため、よりシンプルなこの方針で解決した。

- shiguredo_vpl を `=2026.4.0-canary.2` に更新する（`Vp9EncoderConfig.write_ivf_headers` が追加されたバージョン）
- `encoder_codec_config` で `write_ivf_headers: false` を明示設定し、VP9 encoder の出力を raw VP9 に固定する
- これにより IVF ヘッダーが出力されなくなるため、`vp9_payload_from_vpl`（DKIF / IVF フレームヘッダー除去）とその単体テストを削除し、`handle_vpl_encode_callback` は `into_data()` の結果をそのまま `EncodedImageBuffer` へ渡す
- byte 列による「IVF か raw か」の推測は残っていない（完了条件のうち推測廃止の要件を満たす）
