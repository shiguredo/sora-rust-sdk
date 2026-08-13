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

### 実装境界とモジュール配置

- 映像の無変換パススルーは維持し、音声は「デコード → PCM → WebRTC の Opus エンコーダーで再エンコード」の方式で送信する
- `Mp4SampleReader` は SDK 側で、ビデオトラックに加えて音声トラックのサンプル（データ位置・サイズ・タイムスタンプ・duration）と音声 `SampleEntry` のチャンネル数・サンプリングレートを抽出できるよう拡張する
- 音声サンプルのデコード（PCM 化）と音声キャプチャースレッドは sumomo 側（`examples/sumomo/`）の新規モジュールに置き、PCM の注入は既存の `SumomoAdmState::on_recorded_data` 経由で行う。音声デコーダーの依存は sumomo の `Cargo.toml` に追加する（依存は最小限にし、用途をコメントで明記する）
- `Mp4SampleReader` はビデオキャプチャー (`Mp4VideoCapturer`) に move されて消費されるため、音声キャプチャーが同じ reader に依存する構造にしてはならない。音声サンプルは reader を move する前に一括でデコードして S16 PCM と音声タイムラインを構築し、デコード済み PCM とタイムラインだけを音声キャプチャーへ引き渡す

### 対応コーデックと未対応コーデックの扱い

- 対応コーデックは Opus / AAC を初期対象とし、デコーダーを用意できるものから順に対応する
- 初期完了条件は Opus とする。AAC はデコーダー選定が確定した時点で対応を進める
- 未対応コーデック（例: FLAC、対応前の AAC）を含む MP4 は、従来どおり映像のみを送信する（音声は無音スキップ）ことを既定とし、必要なら明示的なエラーへ切り替える

### 音声ペーシングとループ

- 音声用のキャプチャースレッドを追加し、音声トラックのタイムスタンプに従って PCM を `recorded_data_is_available` に供給する（映像の `Mp4VideoCapturer` と同じ絶対時刻ベースのペーシング）
- 音声も映像と同様に、MP4 末尾で先頭に戻りループ再生する
- 音声と映像の厳密な A/V 同期は対象外とする（それぞれ独立にペーシングする）

### 入力オプションとの関係

- `--audio false` は MP4 音声にも適用し、無効時は音声トラックを添付しない
- `--input-mp4` と `--audio-input-device` は排他とし、両方指定はエラーにする（映像側の `--video-input-device` との排他、`docs/INPUT_MP4.md` の記述と整合させる）
- MP4 音声を使用する場合、`--audio-input-device` 未指定でも `SumomoAdm`（外部 ADM）を生成して `AdmConfig::UseExternal` にし、`recorded_data_is_available` の受け皿を用意する

### PCM 形式

- `recorded_data_is_available` が期待する S16 PCM を供給する
- チャンネル数とサンプリングレートは音声トラックの `SampleEntry` から導出する。サンプリングレートの扱い（ネイティブのまま渡すか 48 kHz へリサンプリングするか）は、実装時に WebRTC の Opus エンコーダー経路に合わせて決める

## 完了条件

- Opus 音声トラックを含む MP4 を `--input-mp4` に指定すると、受信側で音声が聞こえる
- 映像は従来どおり無変換パススルーで送信される
- 音声トラックを含まない MP4 では従来どおり動作する
- `--audio false` 指定時は MP4 音声も送信されない
- `--input-mp4` と `--audio-input-device` の同時指定はエラーになる
- 音声デコーダーが生成する S16 PCM、および `recorded_data_is_available` への供給タイミング・データが正しいことを、実 `SumomoAdm` 経由のテストで検証する（「受信側で音声が聞こえる」は受信クライアントでの手動確認も併用する）
- README の「MP4 無変換送信」の記述が実装と一致する (映像は無変換、音声はデコード + Opus 再エンコードであることを明記する)
- `docs/INPUT_MP4.md` の制約が実装と一致する

## 変更対象

- `src/video_codecs/mp4.rs`（音声トラックのサンプル・SampleEntry 情報の抽出拡張）
- `examples/sumomo/` 配下の音声デコーダー・音声キャプチャーモジュール（新規）
- `examples/sumomo/Cargo.toml`（音声デコーダー依存）
- `examples/sumomo/src/main.rs`（MP4 音声キャプチャーの起動、外部 ADM の生成条件）
- `README.md` / `docs/INPUT_MP4.md`
- `CHANGES.md`
