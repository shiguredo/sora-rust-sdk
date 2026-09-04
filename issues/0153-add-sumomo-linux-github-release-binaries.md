# リリース時に sumomo の Linux バイナリを GitHub Release に添付する

- Created: 2026-09-04
- Completed: {YYYY-MM-DD}
- Branch: feature/add-sumomo-linux-github-release-binaries
- Polished: 2026-09-04

## 目的

tag によるリリース時に、サンプルクライアント `sumomo` の Linux 向けリリースバイナリを GitHub Release に置き、ソースからビルドしなくても配布物を取得できるようにする。

## 現状

- `.github/workflows/release.yml` は tag push で GitHub Release を作成し、`cargo publish` で crates.io へ公開するだけである
- GitHub Release に添付される asset は無く、`examples/sumomo` は `publish = false` のため crates.io からも配布されない
- 利用者はリポジトリを clone して `cargo build -p sumomo --release` する必要がある
- CI (`.github/workflows/ci.yml`) には `ubuntu-*-arm` を含む Linux x86_64 / aarch64 行列があり、ネイティブビルドの実績はある

## 設計方針

- 対象プラットフォームは当面 Linux の Ubuntu 24.04 / 26.04 のみとする（x86_64 と aarch64 の両アーキテクチャ）
  - ビルド行列（4 本）:
    - `ubuntu-24.04`（x86_64）
    - `ubuntu-24.04-arm`（aarch64）
    - `ubuntu-26.04`（x86_64）
    - `ubuntu-26.04-arm`（aarch64）
  - Ubuntu バージョンごとにネイティブビルドしたバイナリを出す。24.04 ビルドを 26.04 利用者向けに兼用しない（実行環境ごとの共有ライブラリ差を踏まえ、ビルドホストの Ubuntu バージョンごとに asset を分ける）
  - Ubuntu 22.04、macOS、Windows は本 issue の対象外とする
  - `ubuntu-slim` は使わない。`shiguredo-github-actions` は slim 優先だが、sumomo / `shiguredo_webrtc` のネイティブビルドには `ci.yml` の Linux job と同様の `apt` 依存（`build-essential`、X11 / Wayland / ALSA / DRM 等）が必要で、slim では要件を満たさない
- ビルドジョブは `ci.yml` の Linux 依存インストールに揃えたうえで `cargo build -p sumomo --release` を実行する。sumomo 側の追加 feature は付けない
  - `sora_sdk` の default feature (`openh264`) は依存経由で有効になる
  - HWA 向け feature (`amf` / `nvcodec` / `vpl` / `libcamera` / `v4l2`) は有効にしない
  - libwebrtc が標準で持つコーデック実装以外のハードウェアアクセラレーションは本 issue の配布物に含めない
- 上記 HWA feature 付きビルドは本 issue の対象外とする
- `.github/workflows/release.yml` に Linux 向け build job を追加し、作成済み GitHub Release へ `gh release upload` で添付する
- artifact 名は release tag（`release.yml` の `VERSION`）と Ubuntu バージョンとアーキテクチャが分かる形にする。`examples/sumomo` の Cargo package version は `0.0.0` のままなので用いない
  - 例: `sumomo-<tag>-ubuntu-24.04-x86_64` / `sumomo-<tag>-ubuntu-24.04-aarch64` / `sumomo-<tag>-ubuntu-26.04-x86_64` / `sumomo-<tag>-ubuntu-26.04-aarch64`
- OpenH264 は実行時にパス指定で共有ライブラリを動的ロードする（sumomo では `--openh264-path`。`OPENH264_PATH` はテスト / CI 用）
  - `cargo build -p sumomo --release` 自体には OpenH264 共有ライブラリは不要（closed issue 0028 の結論と同型）
  - Release asset には sumomo バイナリのみを添付する。OpenH264 共有ライブラリは同梱しない
  - OpenH264 を使う利用者向けに、`--openh264-path` での指定と共有ライブラリの入手先を Release notes か `docs/SUMOMO.md` / `README.md` へ短く案内する
- issue 0099（リリース公開 gate の強化）とは別目的とする。0099 が Release 作成順を変えた場合は、その順序に追従して upload する

## 完了条件

- 正式版 tag と canary tag の両方で、次の 4 本の sumomo リリースバイナリが GitHub Release の asset として取得できる
  - Ubuntu 24.04 x86_64 / Ubuntu 24.04 aarch64
  - Ubuntu 26.04 x86_64 / Ubuntu 26.04 aarch64
- asset 名の version 部分が当該 release tag と一致し、Ubuntu バージョンとアーキテクチャが名前から判別できる
- 添付物は sumomo バイナリのみで、OpenH264 共有ライブラリは同梱されていない
- 添付バイナリは HWA feature (`amf` / `nvcodec` / `vpl` / `libcamera` / `v4l2`) を有効にしていない
- OpenH264 利用時の `--openh264-path` と共有ライブラリ入手の案内が Release notes または既存ドキュメントに存在する
- Ubuntu 22.04 / macOS / Windows 向けバイナリは本 issue では追加しない

## 変更対象

- `.github/workflows/release.yml`
- 必要なら Release notes や `README.md` / `docs/SUMOMO.md` への入手手順と OpenH264 案内の短文追記
