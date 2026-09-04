# リリース時に sumomo の Linux バイナリを GitHub Release に添付する

- Created: 2026-09-04
- Completed: {YYYY-MM-DD}
- Branch: feature/add-sumomo-linux-github-release-binaries
- Polished: {YYYY-MM-DD}

## 目的

tag によるリリース時に、サンプルクライアント `sumomo` の Linux 向けリリースバイナリを GitHub Release に置き、ソースからビルドしなくても配布物を取得できるようにする。

## 現状

- `.github/workflows/release.yml` は tag push で GitHub Release を作成し、`cargo publish` で crates.io へ公開するだけである
- GitHub Release に添付される asset は無く、`examples/sumomo` は `publish = false` のため crates.io からも配布されない
- 利用者はリポジトリを clone して `cargo build -p sumomo --release` する必要がある
- CI (`.github/workflows/ci.yml`) には `ubuntu-*-arm` を含む Linux x86_64 / aarch64 行列があり、ネイティブビルドの実績はある

## 設計方針

- 対象プラットフォームは当面 Linux のみとする
  - `x86_64` (`ubuntu-24.04` 相当)
  - `aarch64` / arm64 (`ubuntu-24.04-arm` 相当)
- ビルドは `cargo build -p sumomo --release` とし、sumomo 側の追加 feature は付けない
  - `sora_sdk` の default feature (`openh264`) は依存経由で有効になる
  - HWA 向け feature (`amf` / `nvcodec` / `vpl` / `libcamera` / `v4l2`) は有効にしない
  - libwebrtc が標準で持つコーデック実装以外のハードウェアアクセラレーションは本 issue の配布物に含めない
- macOS / Windows、および上記 HWA feature 付きビルドは本 issue の対象外とする
- `.github/workflows/release.yml` に Linux 向け build job を追加し、作成済み GitHub Release へ `gh release upload` で添付する
- artifact 名はアーキテクチャが分かる形にする（例: `sumomo-<version>-x86_64-unknown-linux-gnu` / `sumomo-<version>-aarch64-unknown-linux-gnu`）
- OpenH264 は実行時に `OPENH264_PATH` で共有ライブラリを参照する実装であるため、ビルド時に CI と同様 `download-openh264` で用意する。共有ライブラリ自体を Release asset に同梱するかは実装時に判断し、同梱しない場合は取得方法を Release notes か既存ドキュメントへ短く案内する
- issue 0099（リリース公開 gate の強化）とは別目的とする。0099 が Release 作成順を変えた場合は、その順序に追従して upload する

## 完了条件

- 正式版 tag と canary tag の両方で、Linux x86_64 と aarch64 の sumomo リリースバイナリが GitHub Release の asset として取得できる
- 添付バイナリは HWA feature (`amf` / `nvcodec` / `vpl` / `libcamera` / `v4l2`) を有効にしていない
- macOS / Windows 向けバイナリは本 issue では追加しない

## 変更対象

- `.github/workflows/release.yml`
- 必要なら Release notes や `README.md` / `docs/SUMOMO.md` への入手手順の短文追記
