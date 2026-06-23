# MP4 リーダーが `length_size_minus_one` を無視して 4 バイト固定で NAL 長を読んでいる

- Priority: Medium
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-mp4-length-size-minus-one
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S2 (video codec 層の致命的バグ修正) のうち「`mp4.rs` の `lengthSizeMinusOne` 無視」分。

## 目的

`src/video_codecs/mp4.rs:406-425` の `length_prefixed_nalu_to_annex_b` は AVCC / HVCC 形式の NAL 長プレフィックスを常に 4 バイトとして読んでいる。実際には MP4 (ISO/IEC 14496-15) では NAL 長プレフィックスのバイト数は `AvccBox::length_size_minus_one` / `HvccBox::length_size_minus_one` で 1 / 2 / 4 バイトのいずれかが指定される。値 0 (= 1 バイト) や値 1 (= 2 バイト) が指定された MP4 を入力すると、4 バイト読みが実際の NAL 境界を踏み外し、Annex B 変換結果が完全に壊れる。WebRTC 受信側はデコード不能となる (panic ではないがデコードは破綻する)。

`shiguredo_mp4::boxes::AvccBox` (および `HvccBox`) は `length_size_minus_one: Uint<u8, 2>` を持っており、SDK 側で参照すれば正しく扱える。

## 優先度根拠

Medium。

- 大多数の MP4 (FFmpeg / GPAC / 多くのカメラ / 録画機器が出力するもの) は `length_size_minus_one = 3` (4 バイト) を採用しており、現状のコードでも問題なく動作するケースが圧倒的多数
- ただし規格上 1 バイト・2 バイトも有効であり、変換系ツール・組み込み機器・特殊なエンコーダーが出力した MP4 を入力に取った瞬間に黙ってフレームが壊れる
- panic は起こさないため SDK プロセスは生き続けるが、再生がデコードエラーで止まる・キーフレームが取れない・以降のフレームが破綻するなど、再現困難な不具合として現れる
- 修正規模は中程度 (構造体拡張 + 引数追加 + 単体テスト)
- High でない理由: 発生条件が限定的で、現状の `--video-file-mp4` 系の利用パターンでは顕在化していない可能性が高い

## 現状

`src/video_codecs/mp4.rs:115-126` の `Mp4VideoTrackInfo`:

```rust
struct Mp4VideoTrackInfo {
    codec_type: VideoCodecType,
    width: u16,
    height: u16,
    timescale: u32,
    parameter_sets: Option<Vec<u8>>,
    // NAL 長プレフィックスのバイト数 (length_size_minus_one + 1) を保持していない
}
```

`src/video_codecs/mp4.rs:406-425` の `length_prefixed_nalu_to_annex_b`:

```rust
fn length_prefixed_nalu_to_annex_b(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len());
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let nal_size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;
        if offset + nal_size > data.len() {
            break;
        }
        result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        result.extend_from_slice(&data[offset..offset + nal_size]);
        offset += nal_size;
    }
    result
}
```

問題:

- 常に 4 バイトの大端整数として NAL 長を読む
- `length_size_minus_one` が 0 (= 1 バイト) や 1 (= 2 バイト) の MP4 では、最初の NAL 長 1〜2 バイトの後ろに NAL ペイロードが続いているのに、4 バイト分を「長さ」と解釈してしまう
- 結果として nal_size が極端に大きな値になり、`offset + nal_size > data.len()` で break されて以降の NAL を全てスキップする (または、たまたま条件をすり抜けると変な範囲のバイト列を NAL データとして出力する)

`extract_track_info` (`src/video_codecs/mp4.rs:245-323`) でも `length_size_minus_one` を参照していない。`shiguredo_mp4::boxes::AvccBox::length_size_minus_one: Uint<u8, 2>` および `HvccBox` 側の同等フィールドが利用可能なはず (要 shiguredo_mp4 のドキュメント / ソース確認)。

なお `extract_hevc_parameter_sets` (`src/video_codecs/mp4.rs:330-339`) は HvccBox の `nalu_arrays` 内 `nalus` を直接 `extend_from_slice` で結合しているため、長さプレフィックスを読む必要が無く、本問題の影響を受けない。本件は `get_sample` 経路 (`src/video_codecs/mp4.rs:365-388`) で `length_prefixed_nalu_to_annex_b` を呼び出す H.264 / H.265 サンプルデータの取り出しに限定される。

## 設計方針

1. `Mp4VideoTrackInfo` に `nal_length_size: u8` を追加する (H.264 / H.265 トラックでのみ有意。VP8/VP9/AV1 では未使用なので `Option<u8>` でも可だが、H264/H265 でしか参照されないので `u8` のデフォルト 4 で十分)
2. `extract_track_info` の `Avc1` ケースで `avc1.avcc_box.length_size_minus_one.get() + 1` を取り、Mp4VideoTrackInfo に保存する
3. `Hev1` / `Hvc1` ケースも同様に `hev1.hvcc_box.length_size_minus_one.get() + 1` / `hvc1.hvcc_box.length_size_minus_one.get() + 1` を保存する
4. `length_prefixed_nalu_to_annex_b` を `length_prefixed_nalu_to_annex_b(data: &[u8], nal_length_size: u8) -> Vec<u8>` のシグネチャに拡張する。`nal_length_size` は 1 / 2 / 4 を受け入れる (3 は MP4 では未使用だが念のため将来拡張を含めて 1〜4 を許容してもよい)
5. `get_sample` から `track_info.nal_length_size` を渡す
6. `length_prefixed_nalu_to_annex_b` の単体テスト (`src/video_codecs/mp4.rs:761` 付近) に 1 バイト・2 バイト・4 バイトのケースを追加する

`shiguredo_mp4::Uint::get()` の戻り値型と `length_size_minus_one` の正確な格納方法 (`Uint<u8, 2>` で 2 ビット幅) は実装時に shiguredo_mp4 の API を確認する。

## 完了条件

- `Mp4VideoTrackInfo` が NAL 長プレフィックスバイト数を保持している
- `length_prefixed_nalu_to_annex_b` が NAL 長プレフィックスバイト数を引数で受け取り、1 / 2 / 4 バイトのいずれにも対応している
- H.264 と H.265 のいずれも、`length_size_minus_one` の値に応じた正しい変換が行われる
- 1 バイト・2 バイト・4 バイトの NAL 長プレフィックスを持つテストデータでの単体テストが追加され通る
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `Mp4VideoTrackInfo` 構造体に `nal_length_size: u8` フィールドを追加する (`src/video_codecs/mp4.rs:115` 付近)
2. `extract_track_info` の各分岐 (`Avc1` / `Hev1` / `Hvc1`) で `length_size_minus_one + 1` を読んで保存する。VP8/VP9/AV1 は使われないのでデフォルト値 (例えば 0 もしくは 4) を入れる
3. `length_prefixed_nalu_to_annex_b` を引数付きに変更し、`u32::from_be_bytes` の代わりに `nal_length_size` バイトを大端で読むようにする (1 / 2 / 4 を switch、もしくは汎用に `nal_length_size` バイトをループで読む)
4. `get_sample` から `self.track_info.nal_length_size` を渡す
5. 単体テスト (`length_prefixed_nalu_to_annex_b` の既存テスト群、`src/video_codecs/mp4.rs:761` および `:773` 付近) を拡張し、1 バイト・2 バイト・4 バイトの各ケースをカバーする。テストコメントは日本語、テストログは日本語 (AGENTS.md)
6. 既存の呼び出し箇所 (`src/video_codecs/mp4.rs:375` `extract_hevc_parameter_sets` 内ではない、`get_sample` 内) の API を新シグネチャに合わせる

## 関連

- ISO/IEC 14496-15 (Carriage of NAL unit structured video in the ISO Base Media File Format)
- `shiguredo_mp4::boxes::AvccBox::length_size_minus_one`
- `shiguredo_mp4::boxes::HvccBox::length_size_minus_one`
