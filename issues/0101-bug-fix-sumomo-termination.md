# sumomo の終了要求と失敗結果を正しく処理する

- Priority: Medium
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-sumomo-termination
- Polished: 2026-07-30

## 目的

指定時間経過時に sumomo を確実に切断し、非同期処理や描画 thread の失敗を成功終了として扱わない。
通常表示と raw-player の両経路を、同じ「終了要求 → disconnect → `run()` 完了待ち → resource 解放 → 終了 status 決定」の契約へ揃える。

## 優先度根拠

Medium。
正式サンプルの CLI 契約と終了 status が実動作と一致せず、自動実行で hang や障害の見落としが発生する。
SDK 本体の通信処理には影響しないが、`--duration` を利用する検証と raw-player 利用時に再現するため、正式 release 前に修正する。

## 現状

`examples/sumomo/src/main.rs` の通常表示経路は、`SoraConnectionBuilder::build()` が返す handle を `_handle` として保持する。
duration 経過時は event loop を抜けるだけで disconnect command を送らず、loop 内で poll していた同じ `run` future を `run.await` で再び待つ。
server 側から終了しなければ process は完了しない。

raw-player 経路の `run_with_raw_player` は次の失敗を破棄する。

- worker thread 内の `rt.block_on(...)` が返す `Result`
- `JoinHandle::join()` が返す panic payload
- `RawPlayerRenderer::render` 内の window、texture、renderer error

renderer の window close と duration 経過は `AtomicBool` で worker loop を抜けるだけで、connection handle から disconnect しない。
renderer 初期化後の早期 error と worker thread の異常終了を含め、`raw_player::quit()` までの cleanup 所有権も明確でない。

`examples/sumomo/src/ansi_renderer.rs` は I420 変換失敗と stdout の write / flush error を破棄する。
callback から main async loop へ renderer error を通知する経路がない。

fake / MP4 capturer は `Drop` で thread を停止するが、connection 終了前後の drop 順序が終了契約として定義されていない。

## 設計方針

### connection の終了 helper

通常表示と raw-player worker が共有する async helper を `examples/sumomo/src/main.rs` に追加する。
固定した application-level timeout を `CONNECTION_SHUTDOWN_TIMEOUT` として 10 秒にする。
SDK の既定値である WebSocket close 3 秒と DataChannel close 5 秒の合計を収め、接続開始時の 30 秒 timeout より短く local 終了要求を bound する値として 10 秒を使う。

duration 経過、renderer の window close、event channel close、renderer error、raw-player main thread からの停止要求では、その終了要求を観測した時点で `tokio::time::Instant` の deadline を 10 秒後に固定する。
disconnect command の受理待ちと残る `run()` 完了待ちを分けて 10 秒ずつ与えず、終了処理全体を同じ `timeout_at(deadline, ...)` の内側で実行する。

1. `SoraConnectionHandle::disconnect()` future と既に pin 済みの `SoraConnection::run()` future を同じ `tokio::select!` で並行に poll する
2. `run()` が先に完了した場合は、その `Result` を終了結果にし、既に connection が終了したため disconnect ack を追加の成功条件にしない
3. disconnect command が受理された場合は、同じ deadline の残り時間だけ `run()` を待つ
4. connection establishment 中で command receiver が未処理の場合も、deadline で disconnect / `run()` の両 future を打ち切る
5. disconnect error、`run()` error、timeout を成功へ変換せず `AppError` にする
6. timeout 時は disconnect / `run()` future を drop し、connection 所有 resource を解放する

`disconnect().await` だけを先に実行しない。
handle の command は `run()` が poll されて初めて処理されるため、両 future を並行に駆動しない実装は current-thread runtime で停止する。

server 起因で `run()` が通常完了した場合は、追加の disconnect を送らずその結果を返す。
duration 経過は shutdown 全体が成功した場合だけ exit 0 とする。
duration timer は `connection.run()` を最初に poll する直前から開始し、connection establishment の時間も含める。
duration が connection establishment 中に満了し、disconnect command を timeout 内に受理できない場合は shutdown timeout として exit non-zero にする。

終了 trigger と結果は次へ固定する。

| 終了 trigger | disconnect | primary result |
| --- | --- | --- |
| `run()` の `Ok(())` | 不要 | exit 0 |
| `run()` の `Err` | 不要 | connection error |
| duration 満了 | 必要 | shutdown 成功時だけ exit 0 |
| 通常 event channel close | 必要 | shutdown result |
| ANSI renderer error | 必要 | renderer error |
| raw-player window close / Escape | 必要 | shutdown 成功時だけ exit 0 |
| raw-player render error | 必要 | renderer error |
| raw-player worker setup / connection error | 実行可能な段階なら必要 | worker が返した error |
| raw-player worker panic | 実行可能な段階なら必要 | `WorkerPanic` |

最初に観測した non-clean error を primary error とする。
cleanup 中に別 error または panic を観測しても終了 status は必ず non-zero とし、secondary error は機密情報を含めず英語で log に残す。
clean な local trigger と connection / renderer error が同じ `select!` 周回で ready の場合は、biased select にせず connection / renderer error を成功で上書きしない result arbitration helper を通す。

### 通常表示経路

- build 時の `SoraConnectionHandle` を保持する
- duration、event channel close、ANSI renderer error を終了理由として区別する
- duration と local 終了理由では共通 helper から disconnect と `run()` 完了待ちを行う
- `AnsiRenderer::render` と `render_frame` を `io::Result<()>` にし、I420 変換、stdout write / flush の失敗を返す
- renderer callback から専用の unbounded error channel へ最初の error を送り、main loop が受信して終了処理へ進む
- renderer error を channel 満杯や通常 event の混雑で失わない
- `TrackEntry` に sink と対応する video track を保持し、終了時に `remove_sink` を呼べるようにする
- connection 完了後に受信 track の sink、video / audio capturer、renderer の順で callback source を停止・解放する

### raw-player 経路

- `RawPlayerRenderer::new` 内で `raw_player::init()` が成功した直後、`Window::new` / `Renderer::new` より前に SDL cleanup guard を有効化する
- cleanup guard を SDL object より先に宣言し、部分初期化 error と通常 drop のどちらでも texture、renderer、window の後に guard が drop される所有順にする
- SDL の texture、renderer、window をすべて drop した後だけ `unsafe { raw_player::quit() }` を 1 回実行する
- window / texture / renderer 操作の `Result` を `.ok()` や `let _ =` で破棄しない
- `thread::spawn` ではなく、spawn error を返せる名前付き `thread::Builder` を使う
- worker を `JoinHandle<Result<()>>` とし、Tokio runtime build error、async block の error、connection の終了結果を main thread まで返す
- worker thread の panic は固定した英語 message の `AppError::WorkerPanic` に変換する
- worker scope の completion guard が正常 return、`?` による早期 return、panic unwind のすべてで Release store により stop flag を立て、main renderer loop が Acquire load で worker 完了を検出する
- main thread から worker への stop flag と worker から main thread への completion guard / `JoinHandle<Result<()>>` を双方向 protocol とする
- duration 経過時は worker が disconnect helper を実行してから stop flag を設定する
- renderer の window close / error 時は main thread が stop flag を設定し、worker が disconnect helper から終了するまで join する
- worker の setup / connection error 時は stop flag を設定して renderer loop を終了させる
- renderer loop が先に失敗しても worker を detach せず、必ず停止要求と join を完了してから error を返す
- connection / worker / renderer の複数 error が発生した場合は最初の primary error を返し、cleanup 中の secondary error は英語で log に残す

### resource 解放

全終了経路で次を満たす。

- receive track から sink を外して callback を停止する
- fake、MP4、libcamera、video device、audio device capturer を connection 完了後に drop する
- raw-player worker を join してから renderer resource と SDL を解放する
- thread panic、renderer 初期化失敗、disconnect error、shutdown timeout でも worker / capture thread を残さない

自動 recovery のために process abort や detached thread を使わない。

`Mp4VideoCapturer::Drop` の bounded shutdown は issue 0098 が所有する。
現行 feeder thread は長い sample deadline まで `thread::sleep` して stop flag を再確認しないため、0101 だけでは `--input-mp4` 利用時の process 終了時間を保証できない。
0098 の stop + unpark + join が実装済みであることを、本 issue の resource 解放契約の prerequisite とする。

### SDL resource の所有順

`RawPlayerRenderer` は SDL object を `Option` で所有し、専用 owner の `Drop` で次の順序を明示的に保証する。

1. texture を `take()` して drop する
2. renderer を `take()` して drop する
3. window を `take()` して drop する
4. 最後に cleanup guard から `unsafe { raw_player::quit() }` を 1 回呼ぶ

Rust の struct field の暗黙 drop 順だけに依存しない。
`raw_player::init()` 成功後に window または renderer の作成が失敗した場合も、既に作成済みの SDL object を逆順に drop してから cleanup guard が quit する。

## error

`examples/sumomo/src/error.rs` に少なくとも次を追加する。

- connection shutdown timeout
- raw-player worker thread panic

production error / log は英語かつ末尾に句点を付けない。
panic payload、signaling URL、credential、certificate 内容を error message に含めない。
変更対象の renderer / raw-player 経路に残る日本語の frame 受信 log も英語へ統一する。

## test

mock / stub、fake subprocess runner、test 専用の分岐、実装を差し替える trait は使わない。

### unit test

- 実 `std::thread::Builder` で panic する thread を起動し、completion guard の Release store、renderer loop 相当の Acquire load、join、`WorkerPanic` 変換までを一連で検証する
- result arbitration の pure function が、`run()` 先行の `Ok(())`、`run()` error、disconnect ack 後の `run()` 成功、disconnect error、shutdown timeout を定義した結果へ分類する
- 実 Tokio channel の未完了 receiver を application deadline で待ち、shutdown 全体が timeout する
- read-only の実 OS file へ ANSI output helper から書き込み、write error が伝播する
- 実 `I420Frame` を `AnsiTrackSinkHandler` へ渡して write error を発生させ、専用 channel の最初の error を main の result arbitration helper が renderer error として分類する
- `raw_player_renderer.rs` 内の test で実 `I420Frame` に invalid size を設定し、SDL dummy driver の実 `RawPlayerRenderer::render` が error を返す
- SDL の実 event queue へ Quit / WindowClose event を投入し、renderer loop の停止要求、worker completion、join までを検証する
- SDL owner の drop が texture → renderer → window → quit の順で完了し、部分初期化 error 後にも再初期化できる

### CLI integration test

`examples/sumomo/tests/termination.rs` から Cargo が提供する実 `CARGO_BIN_EXE_sumomo` を child process として起動する。
sumomo には metadata option がなく既存 `TEST_SECRET_KEY` を渡せないため、credential や secret query を含まない公開 test Sora を指す `TEST_SUMOMO_SIGNALING_URLS` と random channel ID を使って実 Sora へ接続する。
CI repository variable が未設定なら skip せず設定不足で失敗させる。
公開 endpoint 値は sumomo の正式な `--signaling-url` 入力として child argv にだけ渡し、test の assertion / panic message へ埋め込まない。
実行ごとに 30 秒の process timeout を設け、timeout 時は child を kill / wait して test を失敗させる。
application の duration 1 秒 + shutdown 最大 10 秒と、test harness の 30 秒強制 kill を別の deadline として検証する。

最低限、次を検証する。

- 通常表示で `--duration 1` を指定すると、disconnect log を出して 30 秒以内に exit 0 になる
- 構文不正な signaling URL による即時 connection error は exit non-zero になり、hang しない
- raw-player feature と `SDL_VIDEODRIVER=dummy` で `--duration 1` を指定すると、30 秒以内に exit 0 になる
- repository にある実 MP4 fixture と `--input-mp4 --duration 1` を指定すると、connection shutdown、capturer drop、process exit が 30 秒以内に完了する
- 存在しない MP4 path による raw-player worker async setup error が exit non-zero になる
- 無効な SDL video driver による renderer setup error が exit non-zero になる

同一 test process で SDL global state を使う raw-player unit test だけを直列実行する。
別 child process で実行する CLI integration test には不要な直列化を加えない。
assertion message と test comment は日本語にする。
credential、secret query、certificate を test output に表示しない。
child output を assertion failure に添付する場合は signaling URL を除去する。

## CI

既存の `cargo fmt --all --check`、`cargo clippy --workspace -- -D warnings`、`cargo test --workspace` に加え、raw-player 対応対象である Ubuntu 24.04 の独立 job で次を実行する。
独立 job は既存 `ci` job と同じ checkout、Linux build dependency、Rust stable、cache、`.cargo/config.toml.ci` の設定手順を再利用する。

```console
cargo clippy -p sumomo --features raw-player --all-targets -- -D warnings
SDL_VIDEODRIVER=dummy cargo test -p sumomo --features raw-player
```

workflow top-level `env` に `TEST_SUMOMO_SIGNALING_URLS: ${{ vars.TEST_SUMOMO_SIGNALING_URLS }}` を追加し、通常 matrix、self-hosted、raw-player 独立 job の全 `cargo test` から利用できるようにする。
workflow の command text へ値を展開せず、test process が環境変数から読み取って child argv を構築する。
値を assertion / panic message に出さない。
job timeout を 15 分、各 CLI child timeout を 30 秒にする。

## 変更対象

- `examples/sumomo/src/main.rs`
- `examples/sumomo/src/error.rs`
- `examples/sumomo/src/ansi_renderer.rs`
- `examples/sumomo/src/raw_player_renderer.rs`
- `examples/sumomo/src/tests.rs`
- `examples/sumomo/tests/termination.rs`
- `examples/sumomo/Cargo.toml`
- `.github/workflows/ci.yml`
- `CHANGES.md`

`src/video_codecs/mp4.rs` は本 issue の変更対象に含めず、prerequisite の issue 0098 だけで変更する。

## 完了条件

- issue 0098 の `Mp4VideoCapturer` bounded shutdown が実装・検証済みである
- 通常表示と raw-player が共通の disconnect + `run()` timeout 付き終了契約を使う
- `disconnect()` と `run()` を並行に poll し、current-thread runtime で deadlock しない
- `--duration` 指定時間後に shutdown が完了して process が exit 0 になる
- disconnect error、shutdown timeout、connection error が exit non-zero になる
- async block、worker spawn、worker panic、renderer setup / render error が exit non-zero になる
- raw-player の window close が connection を disconnect して worker を join する
- ANSI renderer error が main loop へ通知され、成功終了として隠れない
- 全終了経路で sink、capturer、worker、renderer、SDL resource が順序どおり解放される
- 通常表示と raw-player の実 CLI integration test が 30 秒以内に完了する
- mock / stub なしで、決定的に発生できる worker panic、connection error、worker setup error、renderer setup / render error、ANSI callback error 通知と result arbitration を検証する
- 実 MP4 fixture を使う CLI integration test が、0098 の stop + unpark + join を sumomo の connection shutdown から実行し、30 秒以内に終了する
- `cargo fmt --all --check` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `cargo test --workspace` が成功する
- `cargo clippy -p sumomo --features raw-player --all-targets -- -D warnings` が成功する
- `SDL_VIDEODRIVER=dummy cargo test -p sumomo --features raw-player` が成功する
- `CHANGES.md` の `develop` セクションへ `[FIX]` と担当者 `@voluntas` を追記する
- comment と test assertion message は日本語、production error / log は英語にする

## 参考

- `examples/sumomo/src/main.rs` の `main`
- `examples/sumomo/src/main.rs` の `run_with_raw_player`
- `examples/sumomo/src/ansi_renderer.rs` の `AnsiRenderer::render`
- `examples/sumomo/src/raw_player_renderer.rs` の `RawPlayerRenderer::render`
- `src/connection.rs` の `SoraConnectionHandle::disconnect`
- `src/connection.rs` の `SoraConnection::run`
