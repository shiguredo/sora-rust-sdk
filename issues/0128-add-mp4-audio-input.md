# MP4 入力の音声対応 (--input-mp4 で音声も送信する)

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/add-mp4-audio-input
- Polished: {YYYY-MM-DD}

## 目的

`sumomo --input-mp4` で、MP4 ファイルに含まれる音声も送信できるようにする。README の「MP4 ファイルから無変換での音声・映像送信対応」という表記は映像に関しては正しいが、音声は未実装のため誤表記であり、「音声対応の実装」によって表記と実装を一致させる。

## 現状

- `src/video_codecs/mp4.rs` の `Mp4SampleReader::new_inner` は、`shiguredo_mp4` が返すトラックから最初のビデオトラックのみを選択し、サンプル読み出しループでもビデオトラック以外のサンプルをスキップする。音声トラックは一切読み出されない
- sumomo の `--input-mp4` 指定時、`examples/sumomo/src/main.rs` の `attach_sender_tracks` はデフォルトで空の `AudioTrackSource` を音声トラックとして添付する。PCM を供給するキャプチャーは `--audio-input-device` 指定時のみ起動するため、音声トラックは無音になる
- `docs/INPUT_MP4.md` は「映像のみ送信する (音声は無視される)」と実態を正しく記載しており、README の「音声・映像送信」表記と矛盾している
- `shiguredo_mp4 2026.4.0` は音声トラックのデマルチプレックスに対応しており、`SampleEntry` の `Opus` / `Mp4a` (AAC) / `Flac` からチャンネル数・サンプリングレート等を取得できる
- `shiguredo_webrtc` の音声パイプラインは映像と異なり、エンコード済みデータを直接注入する公開 API が存在しない。音声は PCM を `AudioTransport::recorded_data_is_available` で供給し、WebRTC 内蔵の Opus エンコーダーが再エンコードする。sumomo はこの経路を `SumomoAdm` (`examples/sumomo/src/adm.rs`) で既に使っている

## 設計方針

- 映像の無変換パススルーは維持し、音声は「デコード → PCM → WebRTC の Opus エンコーダーで再エンコード」の方式で送信する
- `Mp4SampleReader` を拡張し、ビデオトラックに加えて音声トラックのサンプル (データ位置・サイズ・タイムスタンプ・duration) も抽出する
- MP4 の音声サンプルをデコードして PCM に変換する音声デコーダーを追加する (対応コーデックは Opus / AAC を初期対象とし、デコーダーが用意できるものから順に対応する)
- 音声用のキャプチャースレッドを追加し、音声トラックのタイムスタンプに従って PCM を `recorded_data_is_available` に供給する (映像の `Mp4VideoCapturer` と同じ絶対時刻ベースのペーシング)
- 音声と映像の厳密な A/V 同期は対象外とする (それぞれ独立にペーシングする)
- 音声デコーダーは Cargo.toml に依存を追加する (依存は最小限にし、用途をコメントで明記する)

## 完了条件

- 音声トラックを含む MP4 を `--input-mp4` に指定すると、受信側で音声が聞こえる
- 映像は従来どおり無変換パススルーで送信される
- 音声トラックを含まない MP4 では従来どおり動作する
- README の「MP4 無変換送信」の記述が実装と一致する (映像は無変換、音声はデコード + Opus 再エンコードであることを明記する)
- `docs/INPUT_MP4.md` の制約が実装と一致する

## 変更対象

- `src/video_codecs/mp4.rs` (または新規の音声トラック抽出モジュール)
- 音声デコーダーを提供する新規モジュール
- `Cargo.toml` (音声デコーダー依存)
- `examples/sumomo/src/main.rs` (MP4 音声キャプチャーの起動)
- `README.md` / `docs/INPUT_MP4.md`
