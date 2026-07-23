# README / docs / sumomo メタデータを正式リリース向けに整備する

- Priority: Medium
- Created: 2026-07-23
- Completed: {YYYY-MM-DD}
- Model: Composer
- Branch: feature/update-readme-and-docs
- Polished: 2026-07-23

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./closed/0020-other-prepare-stable-release-2026-1-0.md) の Should 派生 issue S5。親 issue は 2026-07-23 に closed 済みで、本 issue は「正式リリース後でも段階対応可能」と位置付けられている（`issues/closed/0020-...md:82,98`）。S7 の `CODEBASE.md` 整備も本 issue に含める。

## 目的

利用者が正式版を導入するときに迷わないよう、README・付属 docs・`CODEBASE.md`・sumomo ドキュメント・sumomo Cargo.toml メタデータを現状の実装・CI・ワークスペース構成に揃える。実装 / CI が正でドキュメントを追従させる方向でズレを潰す。

## 優先度根拠

Medium。ただし内訳が 2 種類混在するため、着手時は次の 2 段構えで扱う。

- **High 相当（正式リリース blocker）**: `README.md:160-165` sendonly サンプルコードのコンパイル不能状態（詳細は「現状」）。crates.io 到着直後の利用者を導入初日にブロックする。単独で最小 PR を先行させる。SKILL.md drift（後述）も同格の blocker で、別 issue として即時起票して並行で解決する
- **Medium（正式リリース後でも段階対応可能）**: 上記以外の全項目。親 issue #0020 の Should 分類に従う

triage 時の誤選別（`Priority: Medium` で選別すると blocker が埋もれる）を避けるため、本 issue の完了条件で「sendonly / SKILL.md drift を別 issue に切り出し、番号を本 issue に記録する」ことを担保する。

## 対象ファイル

- `README.md`
- `src/lib.rs`（`//!` に埋め込まれた sendrecv 最小例が README:92-140 と公開 API 面（use 節・EventHandler・Builder 引数）を共有している。README:92-140 とは意味的に一致する独立サンプルであり完全コピーではない。README を書き換える際は公開 API 面を揃える。粒度差（README はフル `#[tokio::main]` 例、`//!` は最小スニペット）は維持する）
- `examples/sumomo/README.md`
- `examples/sumomo/Cargo.toml`
- `docs/SORA_CPP_SDK.md`
- `docs/INPUT_MP4.md`
- `CODEBASE.md`

## 対象外

- `skills/sora-rust-sdk/SKILL.md`: 本 issue では書き換えない。drift の詳細と別 issue 起票の運用は「完了条件・別 issue の起票」節を参照
- `THIRD_PARTY_LICENSES.md`: 依存更新側で扱う
- `CHANGES.md` のリリース見出し整備: 親 issue の完了条件（closed/0020:94）でメンテナーがリリース時に扱う。本 issue の変更を `CHANGES.md` に載せるかは未確定事項 8 で判定

## 現状

調査日: 2026-07-23。事実として確定した項目のみを載せる。未確定な方針判断は「未確定事項」節に分ける。

### High 相当 blocker

- `README.md:147-173` の sendonly サンプルコードが以下 3 点で破綻している
  - `use` 節（L147）に `SoraConnectionEventHandler` が含まれていない
  - `struct MyEventHandler;` と `impl SoraConnectionEventHandler for MyEventHandler {}` の定義が存在しない
  - `SoraConnection::builder(...)` が 4 引数呼び出しで、`src/connection.rs:658-664` の第 5 引数 `event_handler: impl SoraConnectionEventHandler + 'static` が抜けている
  - `sendrecv`（`README.md:122-128`）と `recvonly`（`README.md:195-201`）は 5 引数で正しい。sendonly だけ追従漏れ
- `skills/sora-rust-sdk/SKILL.md` に massive な API drift（後述の別 issue で扱うため本 issue では書き換えないが、実態は現状として記録する）
  - `SKILL.md:68` 型シグネチャ記述が 4 引数（`SoraConnection::builder(context, signaling_urls, channel_id, role)`）で `event_handler` が抜け
  - `SKILL.md:292-297` sendrecv 例、`SKILL.md:432-437` 複数クライアント例、`SKILL.md:469-471` 複数 URL 例がすべて 4 引数呼び出し
  - `SKILL.md:263` `SoraConnectionBuilder::on_message(Fn(&str, &[u8]))` として書かれているが、`on_message` は `SoraConnectionEventHandler` トレイトのメソッド（`src/connection.rs:1860` で `handler.on_message(label, &message_bytes)` として呼ばれる）で `SoraConnectionBuilder` には無い
  - `SKILL.md:300` `.on_track(|transceiver| ...)`、`SKILL.md:361` `.on_message(|label, data| ...)` も同じく Builder メソッドとして書かれているが実際はトレイトメソッド
  - `SKILL.md:70-87` の 12 個のコールバック表全体が「Builder メソッド」として書かれているが実装はすべてトレイトメソッド

### 実装・CI とドキュメントの確定した齟齬

- `README.md:420-424`「前提条件」は `Rust 1.88 以上` / `libclang` / `Python 3` の 3 項目のみで、`.github/workflows/ci.yml:41-47` が Linux で apt install している以下のパッケージ群がすべて未掲載
  - `build-essential`
  - X11 系: `libx11-dev` `libxext-dev` `libxrandr-dev` `libxi-dev` `libxfixes-dev` `libxcursor-dev` `libxss-dev` `libxtst-dev`
  - Wayland/入力: `libwayland-dev` `libxkbcommon-dev`
  - オーディオ: `libasound2-dev` `libpulse-dev` `libpipewire-0.3-dev`
  - グラフィック: `libvulkan-dev` `libdrm-dev` `libgbm-dev`
  - システム: `libdbus-1-dev` `libudev-dev`
  - Rust bindgen: `libclang-dev`（README 側の `libclang` はこれの略記）
  - CI は「github-hosted で全 feature をカバーする保守的スーパーセット」を install しており、`sora_sdk` の default features（`openh264` のみ）で使う場合の必須集合とは異なる。README では default 用と feature 別追加分を分けて書く
- `README.md:438-447`「対応プラットフォーム」と `.github/workflows/ci.yml:24-31, 76-85` の具体差分
  - README にあるが CI（github-hosted）に無い: `macOS Tahoe 26 arm64` / `macOS Sequoia 15 arm64` / `Windows 11 x86_64`
  - `Windows Server 2025 x86_64`（README:447）は CI の `windows-2025-vs2026`（Visual Studio 2026 プレビュー環境）で回している。README ではその点が読み取れない
  - `ci-self-hosted:84` に `Raspberry-Pi (arm64)` があり、README の「特徴」節（`README.md:47-48`）や「対応コーデック」表（`README.md:64`）でも Raspberry Pi サポートを謳っているにもかかわらず、「対応プラットフォーム」節（`README.md:438-447`）に Raspberry Pi の行がない
- `README.md:397-403` の構成図は `src/` / `examples/sumomo/` / `e2e-tests/` の 3 つのみで、`pbt/`（`Cargo.toml:14` に登録済みのワークスペースメンバー）と `docs/` `tests/` が抜けている。粒度は未確定事項 5 参照
- `README.md:66-71`「MP4 無変換送信」節に「音声は無視される」制約（`docs/INPUT_MP4.md:17` に記載）が反映されていない

### sumomo 側の確定した齟齬

- `examples/sumomo/README.md:10` の `sudo apt install libssl-dev pkg-config` は `libssl-dev` が不要。ワークスペースは `rustls` + `rustls-platform-verifier` + `aws-lc-rs` 構成で、`Cargo.toml` / `Cargo.lock` に `openssl-sys` / `native-tls` / `libssl-*` 系は存在しない。`.github/workflows/ci.yml:41-47` の apt install にも `libssl-dev` は含まれていない
- `pkg-config`（Debian パッケージ）は `Cargo.lock` 上で `shiguredo_audio_device` / `shiguredo_libcamera` / `shiguredo_video_device` の build.rs が pkg-config クレート経由で使うため、`media-device` / `libcamera` feature を有効化する場合は間接的に必要（github-hosted runner にはデフォルト同梱されているため CI では明示 install していない）。したがって「default features では不要、`media-device` / `libcamera` feature では必要」と分岐して案内する
- 同じ `libssl-dev` コメントが `examples/sumomo/Cargo.toml:5-6` にも残存
- `examples/sumomo/README.md:8-11` の「ビルドには」文脈に、runtime パッケージ `pipewire-pulse`（L14）と dev headers `libpulse-dev` / `libpipewire-0.3-dev` 相当が混在。build 依存と runtime 依存の分離が必要
- `examples/sumomo/README.md:76,86,100,109,122,134,168,182,197,209` に `wss://sora-test.shiguredo.co.jp/signaling` が 10 箇所残存。ルート `README.md` / `docs/INPUT_MP4.md` は `sora.example.com` に統一済みだが、sumomo は 1 箇所も置換されていない。`shiguredo-no-secrets` 規約上も「内部エンドポイントのホスト名」の除去対象になり得るので、方針は「10 箇所すべて `sora.example.com` に統一」で確定する
- `examples/sumomo/README.md:33-56` のオプション表に、`examples/sumomo/src/args.rs` に定義されている以下の 8 オプションが載っていない
  - `--video-bit-rate`（`args.rs:290-297`）
  - `--input-mp4`（`args.rs:299-302`。`docs/INPUT_MP4.md` の実行例で使われている主要機能）
  - `--insecure`（`args.rs:336-339`）
  - `--client-cert`（`args.rs:341-347`）
  - `--client-key`（`args.rs:349-355`）
  - `--ca-cert`（`args.rs:357-363`）
  - `--turn-tls-insecure`（`args.rs:370-373`）
  - `--turn-tls-ca-cert`（`args.rs:375-378`）
- `examples/sumomo/src/args.rs:240-244` は `--signaling-url` を `.split(',')` で複数 URL 対応にしているが、`examples/sumomo/README.md:35` の説明は単数 URL 前提の記述で複数 URL の書き方が案内されていない
- `examples/sumomo/Cargo.toml` のパッケージメタデータが薄い
  - `version = "0.0.0"` のまま
  - `description` / `license` / `readme` / `repository` / `homepage` / `authors` などが未指定
  - `publish = false` が **未指定**。同じワークスペースの `e2e-tests/Cargo.toml:6` と `pbt/Cargo.toml:6` は `publish = false` を持っており、sumomo だけ抜けている。誤 publish リスクあり
- `docs/INPUT_MP4.md:21`「`--video-input-device` との同時指定はできない」の実装挙動は「排他エラーではなく silent precedence」。`examples/sumomo/src/main.rs:355-399` の `create_video_capturer` は `mp4_reader` が Some のとき先頭で early return するため、`--input-mp4` と `--video-input-device` を同時指定すると `--input-mp4` が優先され `--video-input-device` は黙って無視される。**同じ early return は `--libcamera`（`main.rs:364-376`）にも先んじるため、`--input-mp4 --libcamera` の同時指定でも `--input-mp4` が優先されて libcamera が silent に無視される**。`args.rs::validate_args`（`args.rs:475-568`）には `--input-mp4 × --video-input-device` / `--input-mp4 × --libcamera` の排他チェックは無い（既存の排他チェック `--libcamera × --video-input-device`（`args.rs:515-521`、`#[cfg(feature = "media-device")]` 下）は `sumomo README:L68` の禁止事項記述と対応しており、そちらは維持する）。MP4 系はドキュメントを実装挙動に合わせて書き直す（未確定事項 7 参照）

### CODEBASE.md

- `CODEBASE.md` はタイトル 1 行（`# sora-rust-sdk`）のみで本文が空。`CLAUDE.md` は「リポジトリ固有の規約・設定がある場合は `CODEBASE.md` を参照すること」と規定しているにもかかわらず参照先が空

### docs/SORA_CPP_SDK.md

- `docs/SORA_CPP_SDK.md:5` は「安定版 `2026.1.2` を基準」と明記済みで最新の安定版と一致（コミット `1dcbb4a` で 2026-07-22 に更新）
- `docs/SORA_CPP_SDK.md:203` は「TURN-TLS クライアント証明書」だけを develop 追随で書いているが、他機能（`multistrap` 廃止・NVIDIA Pascal サポート廃止・TLS 検証のシステム CA 統一 等の C++ SDK develop 差分）は追随していない。develop 追随のスコープが一貫していない（未確定事項 4 参照）

## 未確定事項（着手前に本 issue で決めきる）

デフォルト方針でよければそのまま採用し、異なる方針を採る場合は着手前に issue に追記して確定させる。

1. **README「対応プラットフォーム」節の扱い**: `#0050`（CI 強化）が逆方向で「GitHub-hosted の macOS 行列を CI に追加する」を志向している（`0050-other-strengthen-ci-workflows.md:20-22, 38-39`）。本 issue で先に README を CI 実態まで縮めると、`#0050` 完了後に元の記述に戻す手戻りが発生する。**デフォルト方針: `#0050` の完了を待って本 issue はプラットフォーム節に触らない。ただし Raspberry Pi 行の追加だけは `#0050` 対象外なので本 issue で先行して追加する。追加書式は他の対応プラットフォーム行に合わせて `- Raspberry Pi OS <codename> arm64` の 3 要素形式（例: Bookworm なら `- Raspberry Pi OS Bookworm arm64`）。`<codename>` は self-hosted runner 上で `lsb_release -c` を実行して実測する**
2. **README「優先実装」節（L461-469）の扱い**: 現状「Windows arm64 対応」1 項目のみ。**デフォルト方針: 「維持（Windows arm64 対応の予告として残す。削除すると『対応しない』の意思表示になり混乱する）」**
3. **README「Sora 対応上限」の扱い**: 現状「Sora 2025.1.0 以降」（`README.md:434`）と下限のみ。動作確認済み Sora バージョンは `TEST_SIGNALING_URLS` secret 環境依存でリポジトリからは不可視のため、「実装/CI を正とする」設計方針の例外扱いとなる。**デフォルト方針: 「本 issue では下限のみに縮退して現状維持。動作確認済みバージョンの列挙運用は別 issue で決める（本 issue の完了時に別 issue 番号を記録する）」**
4. **`docs/SORA_CPP_SDK.md` の develop 追随スコープ**: 現状 `SORA_CPP_SDK.md:203` の 1 項目のみ develop 追随している中途半端な状態。(a) develop 追随を完全撤去して基準バージョン `2026.1.2` に対する比較に統一 (b) 「develop 追随項目リスト」節を新設して継続運用にする。**デフォルト方針: 「(a) 撤去して基準バージョン比較に統一。develop の変化を継続追随する運用コストを避ける」**
5. **構成図の粒度**: `pbt/` `docs/` `tests/` は追記確定。**デフォルト方針: 「ワークスペースメンバー（`src/` / `examples/sumomo/` / `e2e-tests/` / `pbt/`）+ 主要ドキュメント（`README.md` / `CHANGES.md` / `LICENSE` / `THIRD_PARTY_LICENSES.md` / `AGENTS.md` / `CODEBASE.md` / `docs/`）+ ビルド設定（`Cargo.toml` / `Cargo.lock` / `Makefile` / `prek.toml` / `rust-toolchain.toml`）+ トップレベル integration test（`tests/`）を載せ、`.github/` / `.cargo/` / `issues/` / `skills/` / `target/` / `canary.py` / `.markdownlint.jsonc` / `.gitignore` は省略」。CODEBASE.md で説明する項目は構成図にも載せる原則を採る**
6. **`shiguredo_webrtc` / `sora_sdk` バージョン記法**: `README.md:81-82` は両方 `"<version>"` プレースホルダで、crates.io ページの利用者がそのままコピペするとビルド失敗する。**デフォルト方針: 「実バージョンを直書きする（例: `sora_sdk = "2026.1.0"`, `shiguredo_webrtc = "~0.150"`）。リリース毎に更新する運用は `CODEBASE.md` のリリースフロー節に明記する」**
7. **`docs/INPUT_MP4.md:21` の `--video-input-device` / `--libcamera` 排他制約の扱い**: 実装は silent precedence（現状節参照）。(a) ドキュメントを「同時指定時は `--input-mp4` を優先する」に書き換える (b) `args.rs::validate_args` に排他チェックを追加する別 issue を切り、本 issue のドキュメント修正は保留にする。**デフォルト方針: 「(a) ドキュメントを実装挙動に合わせて書き換える。挙動そのものは silent precedence で問題ない（`--input-mp4` は明示的な MP4 送信指定なので、意図せず両方付ける利用者はまずいない）」**。書く場所は sumomo README のオプション表（L33-56）の `--input-mp4` 行の説明列（禁止事項の列挙である「制約」節ではなく振る舞い説明の場に置く）
8. **`CHANGES.md` へのエントリ追加要否**: 本 issue は基本的にドキュメント整備だが、Linux apt パッケージ一覧の追加など既存 README になかった情報の追記も含む。**デフォルト方針: 「`CHANGES.md` にはエントリを追加しない。理由: 実装挙動には影響しない純粋な README 補記であり、既に事実だった依存を明文化するだけで利用者側の挙動変化はない」**

## 設計方針

- 実装と CI を正とし、ドキュメントを追従させる（未確定事項 3 は secret 環境依存のため例外）
- `CODEBASE.md` はリポジトリ固有規約・ディレクトリ役割が分かる最小限の本文にする（次節で具体化）
- sumomo は「動かすための最小情報」を README / Cargo.toml に集約する
- 本 issue ではコードの機能変更をしない。実装追加が必要なら別 issue に切り出す（未確定事項 7 参照）
- 実行時サンプルの URL は `wss://sora.example.com/signaling` に統一する
- `.github/workflows/ci.yml:5-6` の `paths-ignore: "**.md"` により `.md` のみの PR では CI が走らないため、sumomo `Cargo.toml` 変更を含むブランチにまとめて CI を発火させる。純粋な `.md` のみブランチにする場合は push 前にローカルで `prek run --all-files` と `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` を実行して代替検証する

### CODEBASE.md に入れる最小セクション

書式は 1 段の inline 記述で統一する（見出しにコード span を含めない）。

1. `## ワークスペース構成`: `src/` / `examples/sumomo/` / `e2e-tests/` / `pbt/` / `docs/` / `tests/` / `skills/` の役割を各 1〜2 行で。構成図（README）と載せる粒度を一致させる
2. `## ビルド`: `cargo build` / feature 組み合わせ（`openh264` がデフォルト。`amf` / `nvcodec` / `vpl` / `v4l2` / `libcamera` の有効化条件と system libs）
3. `## テスト`: `cargo test --workspace` / `cargo test -p pbt` / `cargo test -p e2e-tests` / E2E 実行時の環境変数（`TEST_SIGNALING_URLS` / `TEST_CHANNEL_ID_PREFIX` / `TEST_SECRET_KEY` / `TEST_API_URL`）。Coverage は `Makefile:8-13`、Fuzzing は `Makefile:16-24` を参照
4. `## Lint / Format`: clippy は 4 系統の実行環境があり、それぞれ引数が異なる。(1) `prek.toml:88` = `cargo clippy --workspace --all-targets -- -D warnings`（**`--all-targets` あり。この 1 つだけ非対称で、tests / examples / benches 全ターゲットを対象にする**）、(2) `Makefile:32` = `cargo clippy --workspace -- -D warnings`（`--all-targets` なし）、(3) `.github/workflows/ci.yml:68` = 同（default features、github-hosted）、(4) `.github/workflows/ci.yml:103` = `cargo clippy --workspace ${{ matrix.features }} -- -D warnings`（`openh264` / `openh264,nvcodec` / `openh264,amf` / `openh264,vpl` / `openh264,libcamera,v4l2` の feature 別 matrix、self-hosted）。ローカルで最も広範に警告を検出するには `prek run --all-files` を使うこと、feature-gated code の警告は self-hosted CI でしか検出できないことを明記
5. `## .cargo/config.toml の運用`: `.cargo/config.toml.example` はローカル開発向け設定サンプル（`cp .cargo/config.toml.example .cargo/config.toml` で使う）、`.cargo/config.toml.ci` は CI 実行時に `.github/workflows/ci.yml:66,101` で `cp` される、`.gitignore` により `.cargo/config.toml` 本体はリポジトリ管理外
6. `## リリースフロー`: `canary.py` の役割、`Cargo.toml` バージョン運用（canary → 正式版）、`cargo publish` の流れ、`CHANGES.md` の扱い（詳細は `shiguredo-changelog` スキルへのポインタ）、README の `sora_sdk` / `shiguredo_webrtc` バージョン記法をリリース毎に手動更新する運用（未確定事項 6 で確定した方針）
7. `## 依存の方針`: `[workspace.dependencies]` 統一、時雨堂クレートは `~X.Y` tilde requirement、外部クレートは通常のバージョン範囲、TLS バックエンドは rustls + aws-lc-rs 固定（`openssl` / `native-tls` は使わない）
8. `## MSRV`: `Cargo.toml:5` `rust-version = "1.88"` と `rust-toolchain.toml`（`channel = "stable"`）の関係。両者に乖離が生じた場合の判断（`Cargo.toml` の `rust-version` を正とする）
9. `## issues / メタデータ運用`: `issues/` = open、`issues/closed/` = closed、`issues/pending/` = pending。詳細は `shiguredo-issues` スキルへのポインタ。加えてリポジトリ内では issue メタ情報を `Priority → Created → Completed → Model → Branch → Polished` の順で統一している独自運用の明記

## 完了条件

以下をすべて満たすこと。

### 別 issue の起票（本 issue 着手時に必ず実施）

- **SKILL.md drift 全面追随の別 issue を起票する**（`Priority: High`、sendonly blocker と同格）。sendonly / SKILL.md drift のうち SKILL.md 側は範囲が広いため独立させる。drift の起源は `#0044`（callback trait 化、2026-07-08 completed）で `SKILL.md` 側の追随が漏れている（`#0042` は canary バージョン表記を扱った issue で drift とは無関係。誤引用しないこと）。起票する別 issue には現状節「High 相当 blocker」で列挙した具体箇所（4 引数呼び出し 3 箇所 + Builder 誤扱いのコールバック API 12 個）を素材として引き継ぐ
- **未確定事項 3 で決めた「動作確認済み Sora バージョン列挙運用」の別 issue** を起票する
- 起票した別 issue の番号は、本 issue のマージ用 PR 本文冒頭に「起票済み: #XXXX (SKILL.md drift), #YYYY (Sora 動作確認バージョン運用)」の形式で記録する

### grep 判定（issue ファイル自身を除外するため対象パスを絞る）

以下は bash（process substitution `<(...)` を使用）で実行する前提。判定は exit code で行う。

- `git grep -q 'libssl-dev' -- 'examples/**' 'docs/**' 'README.md' 'CODEBASE.md'` の exit code が **非 0**（ヒットなし）
- `git grep -q 'sora-test.shiguredo.co.jp' -- 'examples/**' 'docs/**' 'README.md' 'CODEBASE.md'` の exit code が **非 0**（sumomo README 10 箇所が `sora.example.com` に置換されている）

### 突合スクリプト（bash 前提）

sumomo README のオプション表と `args.rs` の noargs 定義の集合が一致すること:

```sh
# args.rs 側: noargs::opt / noargs::flag の NAME を抽出
git grep -hoE 'noargs::(opt|flag)\("[a-z0-9-]+"\)' examples/sumomo/src/args.rs \
  | sed -E 's/noargs::(opt|flag)\("(.*)"\)/\2/' | sort -u > /tmp/args.txt
# sumomo README 側: `--NAME` を抽出
grep -oE '`--[a-z0-9-]+`' examples/sumomo/README.md | tr -d '`' | sed 's/^--//' \
  | sort -u > /tmp/readme.txt
# 差分ゼロ（version / help は noargs 自動提供なので README 側にのみ現れる想定で除外）
diff <(sort -u /tmp/args.txt) <(grep -vE '^(version|help)$' /tmp/readme.txt | sort -u)
```

README の Builder メソッド例と実装の突合（対象範囲を絞って false positive を減らす）:

```sh
# README 側: Builder 例セクション (L235-272) だけを対象にメソッド名を抽出
sed -n '235,272p' README.md | grep -oE '\.[a-z_]+\(' | tr -d '.(' | sort -u > /tmp/readme_builder.txt
# 実装側: SoraConnectionBuilder の impl block だけを対象に pub fn を抽出
awk '/^impl SoraConnectionBuilder/,/^impl [^S]|^}/' src/connection.rs \
  | grep -oE 'pub fn [a-z_]+' | awk '{print $3}' | sort -u > /tmp/impl_builder.txt
# README にあって実装に無いメソッド（0 件が期待値）
comm -23 /tmp/readme_builder.txt /tmp/impl_builder.txt
# 実装にあって README にないメソッド（新規追加された public API の case）
comm -13 /tmp/readme_builder.txt /tmp/impl_builder.txt
```

### 目視レビューで判定する項目

- README「### 前提条件」の Linux パッケージ列挙が「default features 用の必須集合」+「feature 別追加集合（`media-device` / `libcamera` / `v4l2` / `nvcodec` / `amf` / `vpl`）」の 2 段構成になっており、和集合が `.github/workflows/ci.yml:41-47` の apt install 一覧をサブセットとして含む
- `docs/SORA_CPP_SDK.md` が基準バージョン `2026.1.2` に対する比較として整合している（未確定事項 4 のデフォルト方針を採る場合は L203 の「develop のみ」列を削除）

### コード例のコンパイル可能性

README / `src/lib.rs //!` のコード例 3 種（sendrecv / sendonly / recvonly）を一時的にワークスペース内のプレースホルダ `.rs` ファイルに貼り付けて `cargo check` を通す（PR には残さない）。`src/lib.rs:18` の ```` ```ignore ```` を外して doctest 化するかは本 issue の対象外（外部リソース `sora.example.com` への接続を避ける書き方への置換が必要なため、別 issue で扱う）

### 構造的な整備

- `examples/sumomo/Cargo.toml` に `publish = false` / `description` / `license` が追加されている
- `README.md:397-403` の構成図が未確定事項 5 で確定した粒度どおりの内容になっている
- `CODEBASE.md` に「CODEBASE.md に入れる最小セクション」で列挙した 9 節がすべて存在する

### docs.rs 側の確認

- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` が exit 0 で終わること
- `Cargo.toml` に `[package.metadata.docs.rs]` が必要かどうかを上記コマンドで判定する。default features `openh264` が docs.rs sandbox でビルド失敗するなら `no-default-features = true` などを追加。追加が必要になった場合は Cargo.toml 変更として本 issue に含めてよい

### 副作用がないこと

- 変更は SemVer 非影響（公開 API を触らない）

## 解決方法

1. 「未確定事項」節の各項目のデフォルト方針を採用しない場合は着手前に issue に追記する
2. 完了条件「別 issue の起票」の 2 件（SKILL.md drift、Sora 動作確認バージョン運用）を先に起票する
3. `CODEBASE.md` を先に整備する（本 issue の他の判断のよりどころになるため）。上記「CODEBASE.md に入れる最小セクション」の 9 節を追加
4. `README.md` を更新する
   - `L81-82` `sora_sdk` / `shiguredo_webrtc` のバージョン記法を未確定事項 6 の方針で書き換える
   - `L147` sendonly 例の `use` 節に `SoraConnectionEventHandler` を追加
   - `L149` の周辺に `struct MyEventHandler;` と `impl SoraConnectionEventHandler for MyEventHandler {}` を追加
   - `L160-165` sendonly 例の `SoraConnection::builder(...)` に第 5 引数 `MyEventHandler` を追加
   - `L235-272` Builder 例の 28 メソッドが `src/connection.rs:160-425` の `pub fn` と一致することを完了条件の突合スクリプトで確認（2026-07-23 時点では乖離なし）
   - `L66-71` MP4 無変換送信節に「音声は無視される（映像のみ送信）」制約を追記
   - `L397-403` 構成図を未確定事項 5 の方針で書き換える
   - `L420-424` 前提条件に Linux 用 apt パッケージ一覧を追加（default features 用の必須集合 + feature 別追加集合の 2 段構成）
   - `L438-447` 対応プラットフォームは未確定事項 1 の方針で処理（デフォルトなら self-hosted runner 上で `lsb_release -c` を実行して codename を実測し、Raspberry Pi 行のみ `- Raspberry Pi OS <codename> arm64` の 3 要素形式で追加）
5. `src/lib.rs:18-42` の `//!` sendrecv 最小例を、README の書き換えに合わせて公開 API 面（use 節・EventHandler・Builder 引数）を揃える。lib.rs は最小スニペット、README はフル `#[tokio::main]` 例という粒度差は維持する
6. `examples/sumomo/README.md` を更新する
   - `L10` の `libssl-dev` を削除。`pkg-config` は default features では不要のため base install から外し、`media-device` / `libcamera` feature 節に移動
   - `L8-14` の「ビルド」文脈と「runtime 起動」の切り分けを整理（`pipewire-pulse` は runtime、`libpulse-dev` / `libpipewire-0.3-dev` は build）
   - `L33-56` オプション表に欠落 8 個を追加。種別列の書式は「必須 / 任意 (値: xxx) / 任意 (フラグ) / 任意 (フラグ、feature 有効時のみ)」の 4 パターンに統一。追加する `--input-mp4` 行の説明列には未確定事項 7 の precedence 説明（「`--video-input-device` や `--libcamera` と同時指定した場合はこちらが優先」）も同時に記載する
   - `L35` `--signaling-url` の説明にカンマ区切りで複数 URL を渡せる旨を追記
   - `L68` の既存排他記述（`--libcamera` と `--video-input-device` は同時に指定できません）は実装（`args.rs:515-521`）と一致しているため維持する
   - `L76` 以降の `sora-test.shiguredo.co.jp` 10 箇所を `sora.example.com` に置換
7. `examples/sumomo/Cargo.toml` を更新する: `publish = false` を追加、`description` / `license` を追加、`L5-6` の `libssl-dev pkg-config` コメントを削除
8. `docs/INPUT_MP4.md` を更新する: `L21` の `--video-input-device` との排他制約を「同時指定時は `--input-mp4` を優先する。`--libcamera` も同様」に書き換える
9. `docs/SORA_CPP_SDK.md` を確認する: 未確定事項 4 のデフォルト方針を採る場合、`L203` の「develop のみ」列を削除して基準バージョン `2026.1.2` に対する比較に統一
10. 完了条件の grep / 突合スクリプト / cargo check / cargo doc をすべて実行して合格を確認する
