# `release.yml` を正式リリース対応に整備する

- Priority: High
- Created: 2026-06-23
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/update-release-yml
- Polished: {YYYY-MM-DD}

親 issue: [`0020-other-prepare-stable-release-2026-1-0.md`](./0020-other-prepare-stable-release-2026-1-0.md) の Must 派生 issue M9。

## 目的

`sora_sdk` の最初の正式版 `2026.1.0` を crates.io に公開するため、`.github/workflows/release.yml` を以下の観点で整備する。現状のままタグを push しても、`cargo publish` の verify ビルドが必須依存の不足で確実に失敗する。さらに prerelease 判定の SemVer 取りこぼし、同時実行制御の欠如、各ジョブの timeout 未設定など、公開 CI / リリースパイプラインとして本番運用に耐えない状態になっている。

これと併せて、`ci.yml` 側にも `cargo publish --dry-run` 検証ジョブを追加し、`release.yml` の整備が崩れたときに PR 段階で検知できるようにする。

## 優先度根拠

High。本 issue は親 issue `#0020` の Must (正式リリースのブロッカー) として位置づけられている M9 である。

- 最初の正式タグを push した瞬間に `cargo publish` の verify ビルドが失敗する見込みで、リリースパイプラインが機能しないまま canary 運用に戻ることになる
- 修正規模は CI 設定ファイルのみで、テスト容易な領域
- 正式リリース後は同様の不具合を踏むたびに「リリースに失敗する → 手動で対応する」の運用負債が積み上がる

## 現状

`.github/workflows/release.yml` 全文 (`L1-69`):

```yaml
name: Release

on:
  push:
    tags:
      - "*"

permissions:
  contents: write
  actions: read

jobs:
  github-release:
    name: "Create GitHub Release"
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.get_version.outputs.VERSION }}
    steps:
      - name: Checkout sources
        uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2

      - name: Get the version
        id: get_version
        run: echo "VERSION=${GITHUB_REF/refs\/tags\//}" >> $GITHUB_OUTPUT

      - name: Create Release
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          if ${{ contains(steps.get_version.outputs.VERSION, 'canary') }}; then
            gh release create ${{ steps.get_version.outputs.VERSION }} \
              --prerelease \
              --title "${{ steps.get_version.outputs.VERSION }}" \
              --notes "Release ${{ steps.get_version.outputs.VERSION }}"
          else
            gh release create ${{ steps.get_version.outputs.VERSION }} \
              --title "${{ steps.get_version.outputs.VERSION }}" \
              --notes "Release ${{ steps.get_version.outputs.VERSION }}"
          fi

  publish:
    needs: github-release
    runs-on: ubuntu-24.04
    environment: release
    permissions:
      id-token: write
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
      - uses: rust-lang/crates-io-auth-action@bbd81622f20ce9e2dd9622e3218b975523e45bbe # v1.0.4
        id: auth
      - run: cargo publish
        env:
          CARGO_REGISTRY_TOKEN: ${{ steps.auth.outputs.token }}

  slack_notify:
    needs: [github-release, publish]
    runs-on: ubuntu-slim
    if: ${{ !cancelled() }}
    ...
```

検出された問題点と各対応:

### 1. publish ジョブに Linux ビルド依存の `apt-get install` ステップが無い (致命的)

- `.github/workflows/release.yml:41-53` の publish ジョブには `apt-get install` が一切無い
- `Cargo.toml:97-98` で `shiguredo_webrtc` が標準依存になっており、`cargo publish` 既定の verify ビルドで shiguredo_webrtc を含む依存クレートの native ビルドが走る
- `ci.yml:35-45` には `build-essential` `libx11-dev` `libwayland-dev` `libasound2-dev` `libdrm-dev` `libclang-dev` 等の apt install ステップがある (これらを正式版ビルドにも適用する必要がある)
- 結果: 初回正式タグ push 時に verify ビルドが確実に失敗する

### 2. OpenH264 のダウンロードと `OPENH264_PATH` 設定が無い (致命的)

- `Cargo.toml:122` で `default = ["openh264"]` (feature が default に入っている)
- したがって `cargo publish` の verify ビルドは `shiguredo_openh264` をビルドしようとし、`OPENH264_PATH` を必要とする
- `release.yml:48-51` には OpenH264 ダウンロードと `OPENH264_PATH` 設定が無く、`ci.yml:53-61` の `shiguredo/github-actions/.github/actions/download-openh264` ステップを `release.yml` に移植する必要がある

### 3. `cargo publish` のパッケージ未指定・`--locked` 未指定

- `release.yml:51` は `cargo publish` のみで、`-p sora_sdk` 指定も `--locked` 指定も無い
- ルート `Cargo.toml` は `[package]` (sora_sdk) と `[workspace]` (`e2e-tests` / `examples/sumomo` / `pbt`) を同居させている。デフォルトで sora_sdk が publish される設計ではあるが、明示しないと将来の workspace 構成変更で挙動が変わる
- `--locked` が無いと `Cargo.lock` を無視した依存解決が走り、CI 上の依存が `Cargo.lock` と一致しない可能性が残る

### 4. prerelease 判定が `contains(VERSION, 'canary')` のみ

- `release.yml:30` は `contains(steps.get_version.outputs.VERSION, 'canary')` だけで prerelease 判定をしている
- SemVer の `-rc` / `-beta` / `-alpha` / `-pre` を取りこぼし、これらの prerelease タグを push した場合に正式版として `gh release create` してしまう
- 過去タグはすべて `*-canary.*` なので未顕在だが、将来 RC を切る運用に移行するなら即座に問題化する
- 一般化するなら「タグ名に `-` が含まれていたら prerelease」(SemVer 2.0.0 のプレリリース識別子定義)

### 5. `concurrency:` ブロックが無い

- `release.yml:1-12` に `concurrency` 設定が無い
- 通常、タグ push の重複起動リスクは低いが、同じタグを意図せず 2 回 push したケースや、 workflow_dispatch を併用するケースで二重 publish のリスクが残る
- リリースは中断しても無害ではない (中断後の再開で gh release create が失敗するなど) ので、`cancel-in-progress: false` で揃える

### 6. 各ジョブに `timeout-minutes` が無い

- `release.yml:13-53` の github-release / publish / slack_notify いずれも `timeout-minutes` 指定が無い
- GitHub Actions のデフォルトは 6 時間のため、ハングしたまま runner を専有する事故になり得る
- `ci.yml:31,87` は `timeout-minutes: 30` を付与している。最低限揃える

### 7. `cargo publish --dry-run` 検証ジョブが `ci.yml` に無い

- `ci.yml` に `cargo publish --dry-run --locked` を実行するジョブが無く、`release.yml` の整備崩れを PR 段階で検出できない
- 上記 1, 2, 3 のような不備は dry-run ジョブがあれば PR の時点で落ちる
- ビルド依存 (apt install + OpenH264) を含む構成で `cargo publish -p sora_sdk --dry-run --locked` を実行する

## 設計方針

`release.yml` の publish ジョブを `ci.yml` の Linux ジョブと同等のセットアップで揃える。具体的には:

1. publish ジョブ先頭に `ci.yml:35-45` と同一の `apt-get install` ステップを追加する (Linux のみ)
2. publish ジョブに `shiguredo/github-actions/.github/actions/download-openh264` を組み込み、`OPENH264_PATH` を `$GITHUB_ENV` に書き出す (`ci.yml:53-61` と同様)
3. `cargo publish` を `cargo publish -p sora_sdk --locked` に置き換える (パッケージ明示 + ロック厳守)
4. `release.yml:26-39` の Create Release ステップを bash 内で `[[ "$VERSION" == *-* ]]` 判定するか、`gh release create` の `--prerelease` フラグを変数化する。SemVer 2.0.0 のプレリリース識別子 (`-` 以降) を取りこぼさない形にする
5. `concurrency: { group: release-${{ github.ref }}, cancel-in-progress: false }` を `release.yml` のトップレベルに追加する
6. 各ジョブに `timeout-minutes: 30` 程度を付与する (slack_notify は短くて可)
7. `ci.yml` に `cargo publish -p sora_sdk --dry-run --locked` を実行する PR チェックジョブを追加する。ビルド依存 (apt install + OpenH264) を含む構成で実行する
8. `rust-cache` の利用と `cp .cargo/config.toml.ci .cargo/config.toml` などの `ci.yml` 側の前処理を、publish と dry-run でも適用するか判断する (適用しなくても build できるかは要検証)

## 完了条件

- `cargo publish -p sora_sdk --dry-run --locked` が新規 `ci.yml` ジョブで通る (PR チェックとして稼働している)
- 新規タグ push でテンプレリリースが作成され、`cargo publish` が verify ビルドを完走して crates.io へ正式版を公開できる状態になる
- `release.yml` に `concurrency` と `timeout-minutes` が付与されている
- prerelease 判定が SemVer 2.0.0 のプレリリース識別子 (`-` 以降) を正しく検出できる
- `cargo +nightly fmt --check` / `cargo clippy --all-targets --all-features -- -D warnings` 等の既存チェックは引き続き通る

## 解決方法

1. `.github/workflows/release.yml` の publish ジョブを以下の順で組み立てる:
   - `actions/checkout`
   - Linux ビルド依存の apt install (`ci.yml:35-45` を移植)
   - `rust-lang/crates-io-auth-action`
   - `shiguredo/github-actions/.github/actions/download-openh264`
   - `OPENH264_PATH` を `$GITHUB_ENV` に書き出す
   - `cargo publish -p sora_sdk --locked` (環境変数 `CARGO_REGISTRY_TOKEN` を引き継ぐ)
2. `.github/workflows/release.yml` の github-release ジョブの prerelease 判定を SemVer ベースに変更する
   - 例: `run` 内で `if [[ "$VERSION" == *-* ]]; then PRERELEASE_FLAG="--prerelease"; else PRERELEASE_FLAG=""; fi` のように展開してから `gh release create` を呼ぶ
3. `.github/workflows/release.yml` のトップレベルに `concurrency` ブロックを追加する
4. 各ジョブに `timeout-minutes` を追加する
5. `.github/workflows/ci.yml` に `cargo publish -p sora_sdk --dry-run --locked` を実行するジョブを追加する
   - ビルド依存 (apt install + OpenH264) を含む構成で実行する
   - PR と push 両方で起動するように既存マトリクスと同水準で設定する
6. CHANGES.md には変更履歴を `shiguredo-changelog` 規約に従って追記する

## 関連

- 親 issue: `#0020` (M9)
- 関連参考: `.github/workflows/ci.yml` (Linux 依存インストール / download-openh264 / OPENH264_PATH 設定 / timeout-minutes の参照元)
- `Cargo.toml:122` (`default = ["openh264"]`)
- `Cargo.toml:97` (`shiguredo_webrtc.workspace = true` 標準依存)
