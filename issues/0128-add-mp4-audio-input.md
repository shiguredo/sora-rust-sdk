# MP4 入力の音声対応 (--input-mp4 で音声も送信する)

- Priority: Medium
- Created: 2026-08-10
- Completed: {YYYY-MM-DD}
- Model: deepseek-v4-flash
- Branch: feature/add-mp4-audio-input
- Polished: {YYYY-MM-DD}

## 目的

`sumomo --input-mp4` で、MP4 ファイルに含まれる Opus 音声も送信できるようにする。

README の「MP4 ファイルから無変換での音声・映像送信対応」という表記は、映像に関しては正しいが、音声は未実装のため誤表記である。
映像の無変換パススルーは維持し、音声は PCM 経由または Opus packet のパススルーで送信する。
音声の送信方式は本 issue の実装前に決定し、実装結果を README と `docs/INPUT_MP4.md` に明記して表記と実装を一致させる。

## 現状

- `src/video_codecs/mp4.rs` の `Mp4SampleReader::new_inner` は、`shiguredo_mp4` が返すトラックから最初のビデオトラックのみを選択し、サンプル読み出しループでもビデオトラック以外のサンプルをスキップする。音声トラックは一切読み出されない
- sumomo の `--input-mp4` 指定時、`examples/sumomo/src/main.rs` の `attach_sender_tracks` はデフォルトで空の `AudioTrackSource` を音声トラックとして添付する。PCM を供給するキャプチャーは `--audio-input-device` 指定時のみ起動するため、音声トラックは無音になる
- `docs/INPUT_MP4.md` は「映像のみ送信する (音声は無視される)」と実態を正しく記載しており、README の「音声・映像送信」表記と矛盾している
- `shiguredo_mp4 2026.4.0` は音声トラックのデマルチプレックスに対応しており、`SampleEntry` の `Opus` / `Mp4a` (AAC) / `Flac` からチャンネル数・サンプリングレート等を取得できる
- `shiguredo_webrtc` の `AudioTransport::recorded_data_is_available` は PCM 入力であり、sumomo はこの経路を `SumomoAdm` (`examples/sumomo/src/adm.rs`) で既に使っている
- sora-rust-sdk は現在 `shiguredo_webrtc ~0.150` を使用している。調査対象の `../webrtc-rs` 0.151 でも `AudioEncoderFactory` は `builtin` のみを公開しており、`RtpSender` は encoded audio の注入、frame transformer の設定、RTP packet の直接送信を公開していない
- `../webrtc-rs` 0.151 が使用する libwebrtc m151 では、`RtpSenderInterface::SetEncoderToPacketizerFrameTransformer` と `TransformableFrameInterface::SetData` を利用すると、内蔵エンコーダーが生成した payload を RTP packetize 前に MP4 の Opus packet へ差し替えられる。ただし、この API は `shiguredo_webrtc` の C / Rust ラッパーに公開されていない
- libwebrtc の公開 `RtpSenderInterface` に encoded audio や RTP packet を直接送信する API はない
- `SumomoAdm` と `examples/sumomo/src/adm.rs` は現在 `media-device` feature で条件付きコンパイルされるが、MP4 音声入力は実オーディオデバイスに依存しない

## 前提

- 方式 2-b（カスタム AudioEncoder）は、「AudioEncoder / AudioDecoder をユーザー側でカスタム可能にする」issue を前提とする
- 前提 issue が提供する `AudioCodecCapability` / `SoraAudioEncoderFactory` 上に、MP4 パススルー用の capability を sumomo 側で実装して登録する
- 方式 1 または 2-a を採用する場合は、この前提は不要になる

## 設計方針

### 音声送信方式の選択肢

| 案 | 方式 | `shiguredo_webrtc` | 主な制約 |
|---|---|---|---|
| 1 | Opus → PCM → Opus | 変更不要 | 再エンコード |
| 2-a | FrameTransformer で差し替え | API 公開 | 固定 ptime、audio level |
| 2-b | カスタム AudioEncoder | API 追加 | 変更範囲、audio level |

#### 1. PCM 経由

- sumomo で MP4 の Opus を 48 kHz / S16 PCM へデコードし、`SumomoAdmState::on_recorded_data` から libwebrtc へ入力する
- 現在の `shiguredo_webrtc` の公開 API だけで実装でき、`dOps.output_gain`、sample 単位の末尾調整、実際の PCM に基づく RTP audio level を扱える
- MP4 音声のデコードと Opus 再エンコードによる品質劣化、処理負荷、デコーダー依存の追加が発生する

#### 2-a. FrameTransformer

- `SumomoAdm` から 10 ms 単位のダミー PCM を入力し、libwebrtc 内蔵 Opus エンコーダーが生成した payload を frame transformer で MP4 の Opus packet へ差し替える
- MP4 の Opus packet 自体はデコードも再エンコードもしないが、送信タイミングを作るためのダミー PCM の Opus エンコードは実行される
- `shiguredo_webrtc` に `FrameTransformerInterface`、`TransformableFrameInterface`、`RtpSenderInterface::SetEncoderToPacketizerFrameTransformer` の C / Rust ラッパーを追加する必要がある
- MP4 の Opus packet duration と libwebrtc の送信 ptime を一致させ、初期対応では全 packet の duration を一定にする。DTX と adaptive ptime は無効にする
- RTP audio level は変換前のダミー PCM から計算されるため、実際の音量を表さない

#### 2-b. カスタム AudioEncoder

- 前提 issue が提供する `AudioCodecCapability` フレームワークでカスタム `AudioEncoder` を登録し、エンコーダーから MP4 の Opus packet を直接返す
- MP4 の Opus packet をデコードも再エンコードもせず、10 ms の整数倍であれば 2-a より自然に packet duration を扱える
- libwebrtc の音声エンコード処理を駆動する 10 ms 単位の PCM 入力は引き続き必要だが、カスタムエンコーダーは PCM の内容を使用しない
- 2-a より `shiguredo_webrtc` の変更範囲が大きく、無音区間、ネットワーク適応要求、可変 packet duration の扱いを定義する必要がある
- RTP audio level は入力したダミー PCM から計算されるため、実際の音量を表さない

Opus packet を無変換で送信できる 2-a と 2-b が有力だが、現時点では方式を決定しない。
実装前に 1、2-a、2-b の制約と検証方法を比較し、採用方式を本 issue に反映する。

次の方式は採用しない。

| 方式 | 採用しない理由 |
|---|---|
| `ChannelSend` へ直接注入 | libwebrtc の内部 API に依存する |
| RTP / SRTP を直接生成 | WebRTC の送信処理を迂回する |
| 外部プロセスで送信 | sumomo の対応ではなくなる |

### 共通の実装境界とモジュール配置

- SDK 側は 1 回の MP4 解析で映像と音声のトラック情報を抽出する。音声についてはサンプルのデータ位置・サイズ・タイムスタンプ・duration と `SampleEntry::Opus` の `dOps` 情報を保持する
- 映像キャプチャーと音声キャプチャーは独立した reader とファイルハンドルを所有する。`Mp4SampleReader` を `Mp4VideoCapturer` へ move した後も、音声側が独立してサンプルを読み出せる公開 API を追加する
- MP4 ファイルは解析時に映像用と音声用のファイルハンドルを個別に開く。`File::try_clone` がプラットフォームによってファイルオフセットを共有し得ることに依存しない
- 映像入力と音声入力は別々のスレッドで扱う。各スレッドは独立した reader と可変状態を持ち、共通の基準時刻、ループ周期、停止通知、エラー通知だけを共有する
- 音声入力スレッドは MP4 の Opus sample を順次読み出し、採用方式に応じて PCM または encoded packet の有界キューへ渡す。ファイル全体の PCM や encoded packet は保持しない
- 音声のデコード、frame transformer、カスタム AudioEncoder との連携は sumomo 側 (`examples/sumomo/`) に置き、SDK 本体の利用者へ方式固有の依存を強制しない
- `SumomoAdm` と `examples/sumomo/src/adm.rs` は `media-device` feature から分離し、sumomo の通常ビルドで利用できるようにする。`AudioDeviceCapturer` と実デバイス依存だけは従来どおり `media-device` feature に残す

### 対応コーデックと未対応コーデックの扱い

- 初期対応は Opus のみとする
- channel mapping family 0 のモノラルまたはステレオを対象とする
- AAC と FLAC はこの issue の対象外とし、必要に応じて別 issue で対応する
- 音声トラックが存在しない MP4 はエラーにせず、映像のみを送信する
- 音声トラックが 1 本だけ存在し、それが AAC、FLAC、未知の音声コーデックなどの未対応コーデックである場合は、英語の warning ログを残し、音声トラックを添付せずに映像のみを送信する
- コーデックを問わず音声トラックが 2 本以上存在する場合は、任意の 1 本を暗黙に選択せず、接続開始前に未対応エラーを返す
- Opus トラックとして認識した後の不正な `dOps`、破損 packet、採用方式の制約を満たさない duration、ファイル I/O 失敗、音声送信経路の失敗は映像のみの送信へ縮退せず、アプリケーションエラーとして接続を終了する
- 再生途中で Opus の `SampleEntry` にあるチャンネル数などの重要な設定が変化した場合は、未対応エラーを返す

### Opus-in-ISOBMFF の扱い

- Edit List は解釈せず無視し、この制限を `docs/INPUT_MP4.md` に明記する
- `dOps.pre_skip` は完全なファイル再生における先頭 PCM の破棄量として使用せず、Edit List を無視する方針に合わせて PCM の破棄も行わない
- `dOps.input_sample_rate` はエンコード前の参考値であり、再生クロックとして使用しない
- `dOps.output_channel_count` と channel mapping family をチャンネル構成の検証に使用する
- 方式 1 では `dOps.output_gain` の Q7.8 dB を復号結果へ適用できる。方式 2-a / 2-b では RTP の Opus payload に output gain を引き継げないため、非ゼロ値を未対応エラーにするか無視するかを方式決定時に定める
- 方式 1 では MP4 sample duration に合わせて復号 PCM の末尾を調整できる。方式 2-a / 2-b では Opus packet の途中を切断できないため、MP4 sample duration と packet 自体の duration の一致を検証し、不一致時の扱いを方式決定時に定める
- 方式 2-a / 2-b では Opus packet の構造と duration をデコードせずに検証する。必要な packet utility は sumomo の依存として追加し、SDK 本体へ強制しない

### 音声ペーシングとループ

- 映像と音声で共有する再生時計を導入する。双方のキャプチャーは同じ基準時刻とループ周期を使い、各スレッドがローカルのループ番号と MP4 内のタイムスタンプから絶対送信時刻を求める
- Opus 音声を送信する場合は、`SumomoAdm` が音声コールバックを登録し、recording 状態になるまで映像と音声の再生時計を開始しない。開始前の音声サンプルを破棄して再生位置がずれることを防ぐ
- ループ周期は現行の映像ループとの互換性を優先し、映像トラックの総 duration とする
- 各ループのペーシングは `Instant` ベースの絶対時刻で行い、処理時間やループ回数による累積ドリフトを防ぐ
- MP4 sample duration はその sample が音声タイムライン上で占める長さの決定に使用する
- 方式 1 では復号結果を PCM FIFO に入れ、`recorded_data_is_available` へ 10 ms ごとに 48 kHz の 480 samples/channel を供給する。たとえば duration が 20 ms の sample は、同時に 2 回供給してから 20 ms 待つのではなく、10 ms 間隔の 2 回に分けて供給する
- 方式 2-a では MP4 の Opus packet duration と内蔵 Opus エンコーダーの ptime を一致させ、内蔵エンコーダーが payload を出力する時刻に対応する MP4 packet を差し替える
- 方式 2-b では `AudioEncoder::Num10MsFramesInNextPacket` に対応する単位で MP4 packet を出力する
- 音声が映像より短い場合の無音区間は、方式 1 では PCM の無音、方式 2-a では差し替え前の内蔵 Opus エンコーダーが生成した無音 packet を使用する。方式 2-b の無音区間の扱いは方式決定時に定める
- 音声が映像より長い場合は映像のループ境界で打ち切る。ただし、方式 2-a / 2-b では Opus packet の途中を切断できないため、ループ境界との整合条件を方式決定時に定める
- 方式 1 では 10 ms buffer がループ境界をまたぐ場合、前ループ末尾と次ループ先頭の PCM を時系列に連結して格納してよい。これは加算によるミキシングではない
- 外部オーディオデバイスとのクロック同期や、音声のリサンプリングによる長時間のドリフト補正は対象外とする

### 入力オプションとの関係

- `--audio false` は MP4 音声にも適用し、無効時は音声トラックを添付しない
- `--input-mp4` と `--audio-input-device` は排他とし、両方指定はエラーにする
- `--input-mp4` と `--video-input-device` も排他とし、MP4 が優先されて実デバイス指定が黙って無視される現状を解消する
- MP4 音声を使用する場合、`--audio-input-device` 未指定でも `SumomoAdm`（外部 ADM）を生成して `AdmConfig::UseExternal` にし、採用方式で必要となる実 PCM またはダミー PCM の入力経路を用意する
- MP4 に対応する Opus トラックがない場合は、MP4 用の `SumomoAdm` と音声送信トラックを生成しない
- 初期対応は「対応映像トラックを含む MP4」に含まれる Opus 音声を対象とする。映像トラックを含まない音声のみの MP4 は対象外とする

### エラーとスレッド終了

- 音声キャプチャーの失敗は英語の error ログに残すだけでなく、sumomo の実行結果へ伝播する
- 音声または映像の一方のキャプチャーが致命的なエラーで終了した場合は、もう一方と接続も終了させ、sumomo を non-zero で終了する
- 最初に発生した致命的なエラーを接続タスクへ通知し、共通の停止通知で両方のキャプチャーを終了する
- キャプチャーの Drop では停止要求を送り、ペーシング待機や recording 開始待機を有界な間隔で解除してスレッドを join する

## 完了条件

- Opus 音声トラックを含む MP4 を `--input-mp4` に指定すると、受信側で音声が聞こえる
- 映像は従来どおり無変換パススルーで送信される
- 音声トラックを含まない MP4 では音声送信トラックを添付せず、映像のみを送信する
- 未対応音声コーデックを含む MP4 では英語の warning ログを出力し、音声送信トラックを添付せずに映像のみを送信する
- 音声トラックを 2 本以上含む MP4 は接続開始前に未対応エラーになる
- `--audio false` 指定時は MP4 音声も送信されない
- `--input-mp4` と `--audio-input-device` の同時指定はエラーになる
- `--input-mp4` と `--video-input-device` の同時指定もエラーになる
- `media-device` feature を有効にしていない sumomo でも MP4 の Opus 音声を送信できる
- 採用方式と、採用しなかった方式に対する判断理由が本 issue に記録されている
- Edit List と `dOps.pre_skip` を再生時の trimming に使用しない制限が `docs/INPUT_MP4.md` に記載されている
- 実際の Opus 音声トラックを含む MP4 fixture を使い、音声 sample の抽出、packet 検証、timestamp、duration を検証する
- 方式 1 を採用した場合は、実際の Opus デコーダーと `SumomoAdm` を使い、48 kHz / S16 / 10 ms 単位の PCM、`output_gain`、sample duration による末尾調整を検証する
- 方式 2-a / 2-b を採用した場合は、MP4 の各 Opus packet がデコードや再エンコードを経ずに送信 payload へ渡ることと、採用方式で定めた duration 制約を検証する
- 映像と音声の再生開始時刻とループ境界が一致し、音声トラックの長さが映像と異なってもループごとにずれが累積しないことを検証する
- 破損 packet、ファイル I/O 失敗、音声送信経路の失敗が sumomo の non-zero 終了へ伝播することを検証する
- モックやスタブは使用しない
- 「受信側で音声が聞こえる」ことは受信クライアントで手動確認する
- README の「MP4 無変換送信」の記述が採用した音声送信方式と一致する
- `docs/INPUT_MP4.md` の制約が実装と一致する

## 次回の方針確認で決めること

- 1、2-a、2-b のどの音声送信方式を採用するか
- 2-a を採用する場合、`shiguredo_webrtc` の frame transformer API、固定 ptime、RTP audio level をどう扱うか
- 2-b を採用する場合、前提 issue のカスタム AudioEncoder API 上での無音区間、ネットワーク適応要求、RTP audio level をどう扱うか
- 方式 1 を採用する場合、全対象 OS で利用する Opus デコーダークレートとビルド方法
- 方式 2-a / 2-b で `dOps.output_gain` が非ゼロのファイルと、sample duration が packet duration と一致しないファイルをどう扱うか
- 方式 2-a / 2-b で Opus packet が映像のループ境界をまたぐ場合を未対応エラーにするか、ループ周期を調整するか
- SDK から映像 reader と音声 reader を分離して取得する公開 API の型名と所有権設計
- `SumomoAdm` の recording 開始を再生時計へ通知する同期手段
- 音声キャプチャーの致命的なエラーを接続タスクと sumomo の終了コードへ伝播する経路

## 変更対象

- `src/video_codecs/mp4.rs`（音声トラックのサンプル・SampleEntry 情報の抽出拡張）
- `examples/sumomo/` 配下の MP4 音声入力モジュール（新規、方式 2-b では `AudioCodecCapability` の実装を含む）
- `examples/sumomo/Cargo.toml`（採用方式で必要な依存）
- `examples/sumomo/src/adm.rs`（`media-device` feature からの分離、recording 開始通知、供給エラーの伝播）
- `examples/sumomo/src/args.rs`（入力オプションの排他検証）
- `examples/sumomo/src/main.rs`（MP4 音声キャプチャーの起動、外部 ADM の生成条件、方式 2-b では `AudioCodecCapability` の登録、エラー伝播）
- `src/connection.rs` / `src/connection_context.rs`（方式 2-a の frame transformer 対応。方式 2-b は前提 issue のフレームワークを使うため本 issue では変更しない）
- `Cargo.toml` / `Cargo.lock`（方式 2-a で必要となる `shiguredo_webrtc` の更新。方式 2-b の更新は前提 issue 側）
- `shiguredo_webrtc`（方式 2-a の frame transformer API。方式 2-b のカスタム AudioEncoder API は前提 issue 側。別リポジトリ）
- `testdata/`（Opus 音声を含む実 MP4 fixture）
- `README.md` / `docs/INPUT_MP4.md`
- `CHANGES.md`
