# VPL VP9 payload の誤ったヘッダー除去を直す

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-vpl-vp9-payload-normalization
- Polished: 2026-07-29

## 目的

VPL が返す raw VP9 payload の先頭データを誤って削除せず、正しい VP9 bitstream を WebRTC へ渡す。

## 優先度根拠

High。VPL の VP9 encoder を利用するだけで payload が破損し、映像を正常に送信できない。

## 現状

`vp9_payload_from_vpl` は、`DKIF` file header の有無だけを確認した後、常に 12 byte を IVF frame header とみなして削除する。
そのため、現在の VPL VP9 encoder が返す raw VP9 bitstream は、長さが 12 byte 以下なら破棄され、13 byte 以上でも先頭 12 byte が欠落する。

現在利用している `shiguredo_vpl` 2026.3.0 は、oneVPL の `mfxBitstream` が示す `DataOffset..DataOffset + DataLength` を範囲検証し、その全 byte を `EncodedFrame::data` として返す。
同 wrapper は `mfxExtVP9Param::WriteIVFHeaders` を有効化していないため、このデータは IVF container ではなく raw VP9 elementary stream である。
oneVPL の `mfxExtVP9Param::WriteIVFHeaders` も、IVF container header を出力するには同フィールドを明示的に ON にするよう定めている。

## 設計方針

- 現在の VPL wrapper が返す VP9 データを raw VP9 elementary stream として扱う
- `vp9_payload_from_vpl` による `DKIF` 判定と、IVF file header および frame header の除去を廃止する
- VP9 でも `EncodedFrame::into_data()` が返した全 byte を、変更せず `EncodedImageBuffer` へ渡す
- 現在も拒否している空 payload は、IVF header の検証とは分離して引き続き callback へ渡さない
- raw VP9 は opaque な byte 列として扱い、長さや `DKIF` prefix から IVF を推測しない
- IVF は入力として受け入れない
  - 将来 wrapper で `WriteIVFHeaders` を有効化する場合は、wrapper の API 契約変更として SDK 側と同時に対応する
- 誤った IVF 契約を固定している `vp9_payload_from_vpl_strips_ivf_headers` と `vp9_payload_from_vpl_rejects_truncated_frame_header` は削除する
- 空 payload の拒否と、それ以外の byte 列を無加工で返す処理は、実処理から利用する小さな private helper として単体テストする

## 変更対象

- `src/video_codecs/vpl.rs`
  - `vp9_payload_from_vpl`
  - `handle_vpl_encode_callback`
  - VP9 payload の単体テスト
- `e2e-tests/tests/vpl_video_codec.rs`
  - VPL VP9 専用の実機 E2E

`shiguredo_vpl` の設定変更や IVF parser の追加は対象外とする。

## 完了条件

- 1 byte および 11 byte の短い入力を含む、空でない raw VP9 payload が byte-for-byte で維持される
- `DKIF` で始まる raw byte 列も加工されず、IVF として推測されない
- 空 payload は callback へ渡されない
- 誤った IVF header 除去処理と、その挙動を前提にした単体テストが削除されている
- Intel VPL 実機上の E2E が VP9 encoder と decoder の両対応を前提条件として検証し、未対応なら成功扱いで終了せず失敗する
- 同 E2E が `VideoCodecType::Vp9` を明示して送受信し、次の全項目を確認する
  - outbound-rtp の MIME type が `video/VP9` で、`packetsSent` が 0 より大きい
  - inbound-rtp の MIME type が `video/VP9` で、`packetsReceived` と `framesDecoded` が 0 より大きい
- `cargo test --workspace --features vpl` が成功する
