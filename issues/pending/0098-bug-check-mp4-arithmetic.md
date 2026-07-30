# MP4 demux の位置・時刻算術と capturer 停止を安全化する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-mp4-arithmetic
- Polished: 2026-07-30

## 目的

`shiguredo_mp4` と `Mp4SampleReader` が MP4 内の位置、サイズ、sample count、sample duration から metadata と入力範囲を計算する際に、整数の切り捨て、overflow、範囲外 slice で panic または wraparound しないようにする。
構造上は parse できても表現可能な位置・時刻範囲を超える入力は、demuxer または reader の初期化 error として拒否する。
また、長い frame 間隔の待機中でも `Mp4VideoCapturer` を速やかに停止できるようにする。

## 優先度根拠

High。
攻撃者が直接送るネットワーク入力ではないが、公開 API へ渡す小さなローカル MP4 ファイルの box large size、`stts`、`ctts`、`stco` / `co64` を書き換えるだけで到達できる。
SDK が metadata を受け取る前の依存 crate 内でも発生し、debug build の panic と release build の wraparound で挙動が変わる。
さらに、overflow しない巨大な frame duration でも `thread::sleep` 中は stop flag を確認できず、`Drop::join` が長時間停止する。

## 現状

### `shiguredo_mp4 2026.3.0`

SDK が `Mp4FileDemuxer::tracks()` / `next_sample()` から値を受け取る前に、固定中の `shiguredo_mp4 2026.3.0` には次の未検査演算がある。

- `demux_mp4_file.rs` が `BoxHeader::box_size` の `u64` を `usize` へ `as` 変換する
- `SampleTableAccessor::new` が `stts` の sample count と累積 duration を未検査で加算する
- `SampleTableAccessor::new` が `ctts` の sample count を未検査で加算する
- `SampleTableAccessor::new` が `stco` / `co64` の chunk offset に sample size を未検査で加算する
- `SampleAccessor::timestamp` が基準 timestamp に sample index 分の duration を未検査で加算する
- `get_sample_by_timestamp` が sample timestamp に duration を未検査で加算する

SDK 側だけを checked arithmetic 化しても、その検査へ到達する前に依存 crate が panic または wraparound し得る。

### `sora-rust-sdk`

`src/video_codecs/mp4.rs` の `Mp4SampleReader::new_inner` には次の未検査演算が残る。

- `required.position as usize` は、`u64` が `usize` に収まらない target で上位 bit を切り捨てる
- `start + size` は、`RequiredInput::size` が大きいと `usize` overflow を起こす
- sample の `data_offset` と `data_size` は `u64` / `usize` のまま保持され、検証後の `get_sample` で `as` 変換と加算をやり直す

issue 0061 は `required.position > file_data.len()`、issue 0062 は sample range がファイル末尾を超える場合を既に修正した。
本 issue はその初期化時検証方針を維持し、残っている型変換と算術 overflow を同じ検証へ統合する。

feeder thread は deadline まで `thread::sleep` し、`Drop` は stop flag の設定後に thread を `join` する。
closed issue 0048 は通常の 30 fps なら停止待ちが約 1 frame 分であるため対応不要としたが、malformed input 由来の巨大な duration では同じ前提が成立しない。
本 issue では arithmetic error の防御をすり抜けた場合にも破棄を停止させない多層防御として、0048 の判断を見直す。

## 実装順と repository 境界

### prerequisite 1: issue 0096

issue 0096 を本 issue より先に実装する。
0096 は SDK 内の decode-order duration 累積、microseconds 変換、presentation span / stride、`Instant` deadline、loop epoch、RTP timestamp の checked arithmetic と feeder 開始前検証を所有する。

本 issue は 0096 の timeline helper と error を再利用し、同じ duration 上限や deadline helper を再設計しない。
0096 実装後のコードを監査し、SDK 内の sample duration と `Instant` 算術に unchecked な `+`、`*`、`as` が残っていないことだけを完了条件にする。
0096 が未実装なら本 issue の SDK branch を開始しない。

### prerequisite 2: `shiguredo_mp4 2026.4.0`

upstream `shiguredo_mp4` repository に専用 issue を作成し、別 branch / PR で後述の安全化と test を実装してから `2026.4.0` として release する。
2026-07-30 時点の crates.io 最新版は `2026.3.0` であり、現行の `~2026.3` requirement は安全でない `2026.3.0` を許容する。
公開 error API と demuxer の安全性契約を変更するため、patch requirement で下限だけをずらさず、新しい minor 系を使用する。

本 repository の branch は upstream release 後に `Cargo.toml` を `shiguredo_mp4 = "~2026.4"` へ更新し、`Cargo.lock` が `2026.4.0` 以上かつ `2026.5.0` 未満を解決することを確認する。
upstream issue / PR / release が未完了の間は、本 issue を pending として完了扱いにしない。
upstream の source や test を sora-rust-sdk の 1 issue / 1 branch へ混在させない。

## 設計方針

### `shiguredo_mp4` の安全化

upstream issue は、少なくとも次の演算を checked arithmetic と検査付き変換へ変更する。
失敗は sample index と演算対象を含む `SampleTableAccessorError` または `DemuxError` の具体的な variant とし、panic、wraparound、saturating arithmetic で処理を継続しない。

- `BoxHeader::box_size.get()` の `u64` から `usize` への変換
- `stts` entry の累積 sample count
- `sample_delta * sample_count` と累積 duration
- `ctts` entry の累積 sample count
- chunk 内の `data_offset + sample_size`
- sample timestamp の基準 timestamp、duration、sample index による計算
- sample timestamp と duration から求める比較終端

同じ `SampleTableAccessor` から MP4 file demuxer の `tracks()` / `next_sample()` 経路で到達する count、index、offset、timestamp 演算を全数監査し、同型の未検査演算を残さない。

`stts` は run-length 形式であり、小さな box でも大きな sample count を表現できる。
`MAX_SAMPLE_COUNT_PER_TRACK = 10_368_000` を設け、上限値ちょうどを受理し、1 sample 超過を `SampleCountLimitExceeded` error で拒否する。
これは 120 fps を 24 時間保持できる件数であり、現在の全 sample metadata を memory に展開する file demuxer の上限として使用する。
`stts` の累積 sample count を checked arithmetic で求めてこの上限を検証し、`ctts`、`stsz`、`stsc` の count 一貫性も確認してから、sample 数に比例する allocation と loop を開始する。
既存の sample-scaled `Vec` は `try_reserve_exact` で必要容量を先に確保し、allocation failure も `SampleTableAccessorError` として返す。

全 sample の timestamp と比較終端を展開した新しい `Vec` は追加しない。
`SampleTableAccessor::new` は `stts` entry ごとに、run の基準 timestamp、`sample_delta * sample_count`、最終 sample timestamp、最終比較終端を checked arithmetic で検証し、既存の圧縮 table を維持する。
総 sample count が `u32` 以下なら、各 duration も `u32` であるため、総 duration の数学的上限は `(u32::MAX)^2 < u64::MAX` となる。
`SampleAccessor::timestamp() -> u64` と `get_sample_by_timestamp() -> Option<_>` は infallible のまま維持し、この invariant の範囲内で既存の圧縮 table から計算する。
`checked_*().expect("validated")`、`unwrap`、sample ごとの timestamp table は追加しない。
上限検証と run 単位の証明を production comment に記録する。

upstream test は巨大な allocation や巨大な実ファイルを作らず、synthetic な `StblBox` と小さな box header で次を検証する。

- `stts` / `ctts` sample count の上限ちょうどと 1 超過
- `MAX_SAMPLE_COUNT_PER_TRACK` ちょうどの受理と 1 sample 超過の拒否を、sample 数分の allocation や反復を行う前に確認する
- synthetic な count を受け取る allocation helper が容量確保失敗を具体的な error にする
- 数個の `stts` entry だけで、総 sample count が `u32` に収まる場合の総 duration、run の最終 timestamp、比較終端が `u64` に収まる数学的最大境界を確認する
- `co64` offset と sample size の加算 overflow
- large-size box の `u64` / `usize` 境界
  - 64 bit target では `u64::MAX` の変換成功後に後続の range 加算が error になる
  - 32 bit target の conditional test では `u32::MAX + 1` の変換を error にする
- debug / release profile のどちらでも同じ error になること

### required input range

`RequiredInput` から slice range を作る pure helper を追加し、`Mp4SampleReader::new_inner` はその helper の結果だけで `file_data` を slice する。

1. `usize::try_from(required.position)` で開始位置を変換する
2. 変換できない場合、または開始位置が `file_data.len()` を超える場合は、既存の `Mp4Error::InputPositionOutOfRange` を返す
3. `required.size == Some(size)` なら `start.checked_add(size)` で要求終端を計算する
4. 加算が overflow する場合は、`position` と `size` を保持する新しい input range overflow error を返す
5. 加算できるが終端がファイル末尾を超える場合は、現在と同じく終端を `file_data.len()` に丸め、truncated input の最終判定は demuxer に委ねる
6. `required.size == None` ならファイル末尾を終端とする
7. `file_data.get(start..end)` を維持し、計算済み range に対する最後の防御とする

`checked_add` の失敗を `file_data.len()` への丸めとして受理してはならない。
算術的に表現できない要求と、表現できるが入力が不足している要求を区別する。

### sample range

demuxer から video sample を受け取る時点で `data_offset` を `usize::try_from` し、`offset.checked_add(data_size)` と `end <= file_data.len()` を検証する。
変換、加算、ファイル範囲のいずれかに失敗した場合は、sample index、元の `u64` offset、size、file size を保持する既存の `Mp4Error::InconsistentSampleTable` を返す。

検証後の `samples` には `usize` の開始位置と終端位置を保持する。
`get_sample` は検証済みの `Range<usize>` で slice し、`data_offset as usize` と `data_offset as usize + data_size` を hot path に残さない。
issue 0062 で定めた「reader 初期化時に全 sample を検証し、`get_sample` を `Result` 化しない」という API 方針は維持する。

### 停止可能な deadline 待機

`thread::sleep(target - now)` を、stop signal で中断できる deadline wait helper へ置き換える。
標準 library だけで実装する場合は feeder thread で `thread::park_timeout(remaining)` を使い、`Drop` は stop flag を `Release` で保存した後、`JoinHandle::thread().unpark()` してから `join()` する。

spurious wakeup または過去の unpark token で frame を早く送らないよう、wait helper は次を loop する。

1. stop flag を `Acquire` で読み、設定済みなら停止結果を返す
2. 0096 の checked deadline と `Instant::now()` から残り時間を求め、deadline 到達済みなら送信継続結果を返す
3. 残り時間を `park_timeout` する
4. wakeup 後に 1 へ戻り、deadline と stop flag を再評価する

`Drop` の応答性を sample duration の値に依存させない。
pure helper の table test に加え、実 thread と channel barrier を使う integration test を追加する。
test thread が `park_timeout` 直前へ到達したことを barrier で確認してから stop flag の設定と `unpark` を行い、終了通知を `recv_timeout` で受け取る。
`sleep`、長い実 deadline、mock / stub は使わず、`Drop` と同じ stop / unpark / join 順序で park 中の thread が終了することを確認する。

### error

required input の `start + size` overflow には、`position` と `size` を保持する専用の `Mp4Error` variant を追加する。
`Display` と `std::error::Error::source` を更新し、error message は日本語とする。
position 変換または開始位置の範囲外は既存の `InputPositionOutOfRange`、sample range の変換・加算・EOF 超過は既存の `InconsistentSampleTable` を使う。

upstream arithmetic error は `Mp4Error::Demux` から source chain を保持して公開 `crate::error::Error` へ伝播させる。
SDK 側で文字列へ変換して原因を失わない。

## test

mock / stub、sleep、外部 command、ネットワークを使わず、pure helper と実 `Mp4SampleReader` をテストする。

- required input range helper で、開始位置 0、EOF ちょうど、EOF 1 byte 超過、`start + size` overflow、`size == None` を確認する
- `required.position` の変換は target pointer width ごとに期待値を分け、64 bit では `u64::MAX` の変換成功後に file size error、32 bit の conditional test では `u32::MAX + 1` を変換 error にする
- 既存 H.264 fixture の `moov` header を 64 bit `largesize` 形式の `u64::MAX` へ書き換え、`position > 0` かつ `size == usize::MAX` となる入力で reader が panic せず input range overflow error を返すことを確認する
- sample range helper で、空 sample、EOF ちょうど、EOF 1 byte 超過、`usize::MAX` 近傍の offset / size と加算 overflow を確認する
- malformed MP4 は `catch_unwind` で panic の不在だけを確認せず、具体的な error variant、sample index、position、size を検証する
- 0096 の既知 fixture について、sample payload、送信順序、全 deadline が変わらないことを確認する
- stop 設定済みの wait helper が park せず停止し、spurious wakeup 相当の再評価で deadline 前に送信継続を返さないことを確認する
- 実 thread の barrier test で、park 中の wait が stop + unpark により終了し、`recv_timeout` まで `join` を停止させないことを確認する

fixture を byte patch する場合は、書き換え前の box type、box size、対象 field を `assert_eq!` で確認してから変更し、偶然別の byte 列を書き換えたテストを成功させない。

## 変更対象

- `src/video_codecs/mp4.rs`
- `Cargo.toml`
- `Cargo.lock`
- `CHANGES.md`

upstream `shiguredo_mp4` の source と test は prerequisite の別 issue / branch / PR で変更し、本 issue の commit には含めない。

## pending 理由

2026-07-30 時点では、安全化を含む upstream `shiguredo_mp4 2026.4.0` と、その issue / PR が存在しない。
また、SDK timeline の prerequisite である issue 0096 も未実装である。
この状態で open にすると実装を完了できず、自動対応も必ず外部 release 待ちで停止するため pending にする。

upstream の issue / PR を作成して `2026.4.0` を公開し、issue 0096 の実装が完了した後に reopened にする。
reopened 時に upstream issue / PR / release の URL と確定 version を本文へ記録し、`Cargo.toml` / `Cargo.lock` の更新から着手する。

## 完了条件

- upstream の専用 issue / PR が、対象算術の checked arithmetic、具体的な error、境界 test を実装している
- `shiguredo_mp4 2026.4.0` が公開済みである
- `Cargo.toml` が `shiguredo_mp4 = "~2026.4"`、`Cargo.lock` が `2026.4.0` 以上かつ `2026.5.0` 未満を解決している
- required input の `position` を `usize::try_from` し、`start + size` を `checked_add` してから slice range を構築する
- required input range の変換または加算に失敗すると、panic や clamp ではなく対応する `Mp4Error` を返す
- sample の offset を reader 初期化時に `usize` へ検査付き変換し、`checked_add` 済みの range を `samples` に保持する
- `get_sample` に `as usize` と未検査の range 加算が残っていない
- sample range が変換不能、加算 overflow、またはファイル範囲外の場合に `InconsistentSampleTable` を返す
- 0096 が所有する SDK 内の duration、microseconds、deadline、loop epoch 算術に未検査の `+`、`*`、`as` が残っていない
- feeder thread の待機を stop signal で中断でき、`Drop` が sample duration の残り時間だけ停止しない
- large-size box、sample range、stop / unpark の境界 test が具体的な error と終了結果を検証する
- 通常 fixture の sample payload、送信順序、deadline が従来どおりである
- debug / release profile に依存せず、同じ不正入力が同じ error になる
- `cargo test --workspace` が成功する
- `cargo test --workspace --release` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする

## 参考

- `src/video_codecs/mp4.rs`
- `issues/0096-bug-preserve-mp4-presentation-timestamps.md`
- `issues/closed/0048-bug-fix-mp4-capturer-stop-latency.md`
- `issues/closed/0061-bug-fix-mp4-demuxer-required-input-oob.md`
- `issues/closed/0062-bug-fix-mp4-get-sample-oob-panic.md`
- `shiguredo_mp4 2026.3.0` の `src/auxiliary.rs`
- `shiguredo_mp4 2026.3.0` の `src/demux_mp4_file.rs`
- Rust standard library `std::primitive::usize::checked_add`
- Rust standard library `std::time::Instant::checked_add`
