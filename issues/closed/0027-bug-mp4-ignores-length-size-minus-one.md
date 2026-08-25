# MP4 リーダーが `length_size_minus_one` を無視して 4 バイト固定で NAL 長を読んでいる

- Priority: Medium
- Created: 2026-06-23
- Completed: 2026-06-25
- Model: Opus 4.7
- Branch: feature/fix-mp4-length-size-minus-one
- Polished: 2026-06-25

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

- 常に 4 バイトの大端整数として NAL 長を読むため、`length_size_minus_one` が 0 (= 1 バイト) や 1 (= 2 バイト) の MP4 では NAL 境界を誤読する
- 結果として nal_size が極端に大きな値になり、`offset + nal_size > data.len()` で break されて以降の NAL を全てスキップする

`extract_track_info` (`src/video_codecs/mp4.rs:245-323`) でも `length_size_minus_one` を参照していない。`shiguredo_mp4::boxes::AvccBox::length_size_minus_one: Uint<u8, 2>` および `HvccBox::length_size_minus_one: Uint<u8, 2, 0>` が利用可能であり、`.get()` で `u8` 値（下位 2 ビットをマスクした値）を取得できる。`.get() + 1` により NAL 長プレフィックスのバイト数 (1/2/3/4) が得られる。

なお `extract_hevc_parameter_sets` (`src/video_codecs/mp4.rs:330-339`) は HvccBox の `nalu_arrays` 内 `nalus` を直接 `extend_from_slice` で結合しているため、長さプレフィックスを読む必要が無く、本問題の影響を受けない。

## 設計方針

1. `Mp4VideoTrackInfo` に `nal_length_size: u8` を追加する (H.264 / H.265 トラックでのみ有意。VP8/VP9/AV1 では使用しないがデフォルト値 4 を入れておく)
2. `extract_track_info` の `Avc1` ケースで `avc1.avcc_box.length_size_minus_one.get() + 1` を取り、Mp4VideoTrackInfo に保存する
3. `Hev1` / `Hvc1` ケースも同様に `hev1.hvcc_box.length_size_minus_one.get() + 1` / `hvc1.hvcc_box.length_size_minus_one.get() + 1` を保存する
4. `Vp08` / `Vp09` / `Av01` ケースでは `nal_length_size: 4` を入れる（`get_sample` で使用されないが、誤使用時の防御）
5. `length_prefixed_nalu_to_annex_b` を `length_prefixed_nalu_to_annex_b(data: &[u8], nal_length_size: u8) -> Vec<u8>` のシグネチャに拡張する。`nal_length_size` バイトを大端で読むループを実装し、1 / 2 / 4 のいずれにも対応する。値 3 (= 3 バイト NAL 長) は ISO/IEC 14496-15 上 `lengthSizeMinusOne` の取りうる値 (0〜3) には含まれるが実際の利用例が知られていないため、`nal_length_size` が 1/2/4 以外の場合は `panic!` させる (実装の健全性を優先)
6. `get_sample` から `track_info.nal_length_size` を渡す
7. `length_prefixed_nalu_to_annex_b` のドキュメントコメント (L397-405) の「4 バイトの長さプレフィックス」記述を `nal_length_size` バイトに対応した説明に更新する
8. `get_sample` のドキュメントコメント (L358-364) の「4 バイト長さプレフィックス」記述も更新する

### nal_length_size フィールドの型について

`u8` とする。`Option<u8>` にして VP8/VP9/AV1 で「NAL 長プレフィックスの概念がない」ことを型で表現する選択肢もあるが、`get_sample` 内で毎回 `unwrap_or(4)` が必要になり、H.264/H.265 での誤った `None` の混入をコンパイル時に防げないため、`u8` で十分。

## 完了条件

- `Mp4VideoTrackInfo` が `nal_length_size: u8` フィールドを保持している
- `length_prefixed_nalu_to_annex_b` が `nal_length_size` を引数で受け取り、1 / 2 / 4 バイトのいずれにも対応している（1/2/4 以外の値では `panic!` する）
- H.264 (`Avc1`) と H.265 (`Hev1` / `Hvc1`) のいずれも、`length_size_minus_one` の値に応じた正しい変換が行われる
- `length_prefixed_nalu_to_annex_b` および `get_sample` のドキュメントコメントが `nal_length_size` に対応した説明に更新されている
- 単体テストが以下の全ケースをカバーし通過すること:
  - 1 バイト NAL 長プレフィックス: 単一 NAL / 複数 NAL / truncated NAL
  - 2 バイト NAL 長プレフィックス: 単一 NAL / 複数 NAL / truncated NAL
  - 4 バイト NAL 長プレフィックス: 単一 NAL / 複数 NAL / truncated NAL（既存テストを拡張して継続確認）
- 既存の fixture テスト (`sample_reader_reads_fixture_h264_mp4`) で `get_sample(0)` の出力データの先頭 NAL が正しく Annex B 変換されていることを検証する
- VP8/VP9/AV1 のパススルー経路 (`_ => raw_data.to_vec()`) に変更がないこと
- `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `Mp4VideoTrackInfo` に `nal_length_size: u8` フィールドを追加する
2. `extract_track_info` の `Avc1` / `Hev1` / `Hvc1` 各分岐で `length_size_minus_one.get() + 1` を `nal_length_size` に保存する
3. `Vp08` / `Vp09` / `Av01` の各分岐で `nal_length_size: 4` を設定する
4. `length_prefixed_nalu_to_annex_b` のシグネチャを `(data: &[u8], nal_length_size: u8)` に変更し、`nal_length_size` バイトをループで大端読みする実装に書き換える。`nal_length_size` が 1/2/4 以外の場合は `panic!` させる
5. `get_sample` の H.264/H.265 分岐で `length_prefixed_nalu_to_annex_b(raw_data, self.track_info.nal_length_size)` を呼ぶように変更する
6. ドキュメントコメントを更新する:
   - `length_prefixed_nalu_to_annex_b` のコメント: 「4 バイトの長さプレフィックス」→ `nal_length_size` バイトに対応
   - `get_sample` のコメント: 「4 バイト長さプレフィックス」→ `nal_length_size` バイトに対応
7. 単体テストを拡張する:
   - 1 バイト NAL 長: 単一 NAL / 複数 NAL / truncated NAL
   - 2 バイト NAL 長: 単一 NAL / 複数 NAL / truncated NAL
   - 4 バイト NAL 長: 既存テストを維持しつつ、明示的に `nal_length_size: 4` を渡す
8. fixture テスト (`sample_reader_reads_fixture_h264_mp4`) を拡張し、`reader.get_sample(0).data` の先頭に Annex B スタートコード `[0x00, 0x00, 0x00, 0x01]` が存在することを検証する
9. `CHANGES.md` に `[FIX] MP4 リーダーが length_size_minus_one を無視して 4 バイト固定で NAL 長を読む問題を修正する` エントリを追記する
10. `cargo +nightly fmt` / `cargo clippy --all-targets --all-features -- -D warnings` が通ることを確認する

## 関連

- ISO/IEC 14496-15 (Carriage of NAL unit structured video in the ISO Base Media File Format)
- `shiguredo_mp4::boxes::AvccBox::length_size_minus_one`
- `shiguredo_mp4::boxes::HvccBox::length_size_minus_one`

## 解決方法

`Mp4VideoTrackInfo` に `nal_length_size: u8` フィールドを追加。
`extract_track_info` で各 codec の `length_size_minus_one` から計算。
`length_prefixed_nalu_to_annex_b` のシグネチャを `(data, nal_length_size)` に変更し 1/2/4 バイト対応。
テスト追加: 1/2/4 バイト NAL 長。
フィクスチャテストに Annex B スタートコード検証追加。

### 修正ファイル
- `src/video_codecs/mp4.rs`
- `CHANGES.md`
