# VPL の VP9 出力の IVF ヘッダー有無で payload 処理を分岐させる

- Priority: Medium
- Created: 2026-08-01
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/add-vpl-vp9-ivf-header-branching
- Polished: {YYYY-MM-DD}

## 目的

VPL の VP9 encoder の出力が IVF ヘッダー付きか raw VP9 かを、byte 列の推測ではなく shiguredo_vpl が報告する情報に基づいて判定し、どちらの設定でも正しく payload を処理できるようにする。

## 優先度根拠

Medium。現状の設定では正しく動作しており実害はないが、byte 列の推測による判定を排除して将来の設定変更や wrapper の変更に堅牢にするための対応。

## 現状

VPL の VP9 encoder は `mfxExtVP9Param::WriteIVFHeaders` の設定で出力が変わり、現在利用している shiguredo_vpl 2026.3.0 は同フィールドを設定しないため oneVPL 実装のデフォルト (ON) が適用される。

- ON (デフォルト): 先頭フレームは IVF ファイルヘッダー (DKIF, 32 byte) + フレームヘッダー (12 byte)、2 フレーム目以降はフレームヘッダー (12 byte) だけが付く
- OFF: IVF ヘッダー無しの raw VP9 elementary stream

sora-rust-sdk の `vp9_payload_from_vpl` は、`DKIF` マジックとフレームヘッダー構造 (フレームサイズ一致・未使用フィールド・直後のフレームマーカー) の推測で IVF ヘッダーを判定して除去する。

これは現状の出力に対しては正しく動くが、以下に弱い。

- raw VP9 がたまたまフレームヘッダーの構造に一致した場合の偽陽性を原理的に排除できない (確率は無視できるが証明できない)
- 設定変更や wrapper の変更で出力形式が変わった場合に追従できない

## 設計方針

- shiguredo_vpl に VP9 encoder の IVF ヘッダー出力の有無を報告する API を追加する
  - 例: 有効な `WriteIVFHeaders` の値を返すメソッド、または `Vp9EncoderConfig` に `WriteIVFHeaders` を明示設定できるフィールド
- sora-rust-sdk は shiguredo_vpl を更新し、報告された情報に基づいて `vp9_payload_from_vpl` を分岐させる
  - IVF ヘッダー付き: ファイルヘッダー (32 byte) + フレームヘッダー (12 byte) を除去する
  - raw VP9: 無加工で返す
- byte 列からの推測 (`DKIF` マジックやフレームヘッダー構造の判定) を廃止する
- 空 payload の拒否は維持する

## 変更対象

- shiguredo_vpl (別リポジトリ) への API 追加とバージョン更新
- `src/video_codecs/vpl.rs`
  - `vp9_payload_from_vpl`
  - `handle_vpl_encode_callback` の VP9 分岐
  - VP9 payload の単体テスト

## 完了条件

- shiguredo_vpl から IVF ヘッダーの有無を取得して分岐できる
- IVF ヘッダー付き出力 (WriteIVFHeaders ON) で正しくヘッダーが除去される
- raw VP9 出力 (WriteIVFHeaders OFF) が無加工で維持される
- byte 列の推測による判定が残っていない
- `cargo test --workspace --features vpl` が成功する
