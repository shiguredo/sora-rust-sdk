# canary release commit と tag を検証後に atomic push する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-canary-release
- Polished: 2026-07-30

## 目的

`canary.py` が不正な branch、dirty worktree、remote と不一致な commit から release commit を作らないようにする。
新しい version の package を一時 worktree で検証してから、`develop` と同じ commit を指す canary tag を `origin` へ atomic push する。
中間失敗後も同じ target version で安全に再開または中止でき、意図せず次の canary version へ進まない状態機械を設ける。

## 優先度根拠

High。
現行 script は `Cargo.toml` と `Cargo.lock` を primary worktree で変更し、local commit と tag を作った後、暗黙の upstream への branch push と `origin` への tag push を別々に実行する。
branch push 後に tag push が失敗すると remote が部分更新され、同じ操作の再実行は次の canary number を生成する。

tag push は `.github/workflows/release.yml` を即時起動する。
現行 workflow には tag / package version の検証と公開再開の状態機械がなく、GitHub Release を crates.io publish より先に作る。
local refs の原子性だけでは end-to-end の公開安全性を保証できないため、issue 0099 を実運用の prerequisite とする。

## 現状

`canary.py` は次の順序で処理する。

1. 正規表現で `Cargo.toml` の package version を書き換える
2. `cargo update sora_sdk` で `Cargo.lock` 内の workspace root package version を同期する
3. `Cargo.toml` と `Cargo.lock` を commit する
4. lightweight tag を作る
5. remote と branch を省略した `git push` を実行する
6. `git push origin <version>` で tag を公開する

branch、detached HEAD、upstream、staged / unstaged / untracked file、remote 同期、既存 local / remote tag、Git 操作中状態を検査しない。
fmt、clippy、test、package、publish dry-run を candidate version に対して実行しない。
失敗時の cleanup、prepared state の識別、resume / abort がない。

dry-run は予定 command の一部を表示するだけで、preflight、candidate package、atomic push capability を検証しない。
確認 prompt は `(Y/n)` と表示する一方、Enter を cancel として扱う。

Python 用の `pyproject.toml`、`uv.lock`、ruff、ty、pytest、CI / prek 経路は存在しない。

## issue 0099 との境界

### issue 0100 が所有する範囲

- local repository の preflight
- target canary version の計画と検証
- temporary worktree 内での candidate commit 作成
- candidate commit に対する Python / Rust / package 検証
- local prepared refs、resume、abort
- `origin` の `develop` と canary tag の atomic push
- atomic push 成功後の local `develop` fast-forward と prepared ref cleanup

### issue 0099 が所有する範囲

- 任意 tag push を拒否できない GitHub Actions 側の tag / version / default branch / CI / package gate
- crates.io publish の結果照会と再実行
- publish 済み version の no-op 判定
- crates.io publish 成功後の GitHub Release 作成
- GitHub Release の属性検証と `gh release create --verify-tag`
- release workflow の permissions、trusted publisher、environment、crate 単位の concurrency

GitHub と crates.io をまたぐ厳密な transaction は作れない。
0099 は「検証 → 未公開なら publish → registry で公開確認 → GitHub Release 作成」の再開可能な状態機械を実装する。
closed issue 0028 では release workflow 強化を実用上不要としたが、正式版公開前の再レビューで、任意 tag、GitHub Release 先行作成、publish 結果不明時の再実行という新しい具体的な失敗条件が確認されたため判断を見直す。

0099 の実装と検証が完了するまで、本 issue の新しい script で tag を実際に公開しない。
0100 は単独で end-to-end release が安全になったとは扱わない。

## CLI と version 遷移

CLI は target version を required option で受け取る。

```console
uv run python canary.py --version 2026.1.0-canary.17
uv run python canary.py --version 2026.1.0-canary.17 --dry-run
uv run python canary.py --version 2026.1.0-canary.17 --yes
uv run python canary.py --version 2026.1.0-canary.17 --abort
uv run python canary.py --version 2026.1.0-canary.17 --resolve-unpublished
```

同じ引数が同じ release operation を表すよう、実行ごとに target version を暗黙算出しない。
version は ASCII decimal の `YYYY.M.0` または `YYYY.M.0-canary.N` だけを受理し、leading zero、別 prerelease、build metadata、patch が 0 でない値を拒否する。

- current が `YYYY.M.0-canary.N` なら、target は同じ core version の `canary.(N + 1)` だけを受理する
- current が `YYYY.M.0` なら、target は `YYYY.(M + 1).0-canary.0` だけを受理する
- current と target の `N` / `M` 加算は checked arithmetic とする
- `Cargo.toml`、`Cargo.lock` の root `sora_sdk` package、planned tag の version を完全一致させる

`--dry-run` は確認入力を要求しない。
通常実行は `Proceed? [y/N]:` と表示し、`y` / `yes` だけを受理する。
Enter とそれ以外は file / index / ref を変更せず exit 0 で cancel する。
`--yes` は確認だけを省略する。
`--abort` は後述の local prepared state だけを中止し、`--dry-run` / `--yes` / `--resolve-unpublished` と同時指定できない。
`--resolve-unpublished` は `RemoteUnknown` の明示的な回復専用で、他の mode と同時指定できない。

生成する commit message は git 規約に従い、prefix のない日本語命令形へ固定する。

```text
canary バージョンを <version> に更新する
```

tag は既存運用と同じ lightweight tag とする。
検証済み version 以外を refspec に展開せず、常に完全な `refs/...` 名を使用する。

## preflight

file、index、local branch / tag を変更する前に、`git ls-remote` で target version に対応する local / remote ref を取得して state を分類する。
common preflight と state 分類は、Fresh の次 version 検証より先に行う。
これにより、同じ target version の Prepared / Published 再実行を current version だけで誤って拒否しない。

すべての state で次を検証する。

- Python は 3.12、3.13、3.14 のいずれかである
- current directory が repository root である
- repository が unborn branch、detached HEAD、merge、rebase、cherry-pick、revert、bisect の途中ではない
- current branch が正確に `develop` である
- upstream が正確に `origin/develop` である
- `git status --porcelain=v2 --untracked-files=all` が空で、staged、unstaged、untracked file が 0 件である
- `Cargo.toml` と `Cargo.lock` の current version が一致する
- `git`、`cargo`、`uv` の required command が利用できる

remote `develop` は `git ls-remote origin refs/heads/develop`、remote tag は `git ls-remote --tags origin refs/tags/<version> refs/tags/<version>^{}` で照合する。
state 分類では fetch せず、remote-tracking branch と local tag を暗黙に更新しない。
remote URL の文字列や credential を log に出さない。

Fresh だけは、さらに次を検証する。

- `HEAD == git ls-remote で得た remote develop object ID` であり、ahead、behind、diverged ではない
- target version が current version から定義した次 version と一致する
- target の local tag、remote tag、`refs/canary/<version>` が存在しない
- `git push --dry-run --atomic origin HEAD:refs/heads/develop` が成功し、remote が atomic push を受理する

Prepared / RemoteUnknown / PotentialPublished / Published / Conflict は後述の object ID と candidate metadata から分類する。
分類済み state に Fresh の「target ref が存在しない」と「次 version」の条件を適用しない。
Prepared では candidate の branch / tag refspec を使う atomic dry-run、Published では remote ref の照合によって atomic push の事前検証を置き換える。

preflight 後に remote `develop` または tag が変化しても、最終 atomic push の non-fast-forward / ref conflict で branch と tag の両方を未更新にする。
force push と tag の上書きは行わない。

## candidate commit の作成と検証

primary worktree を直接書き換えない。
`tempfile.TemporaryDirectory` 相当で temporary root を作り、その配下のまだ存在しない `<temporary-root>/candidate` に `git worktree add --detach <candidate-path> HEAD` する。
script が作成した一時 worktree 内だけで次を行う。

1. `tomllib` で確認済みの `[package].version` だけを target へ更新する
2. `cargo update -p sora_sdk` で `Cargo.lock` の root package version を同期する
3. `Cargo.toml` と `Cargo.lock` の version が target と一致することを再検証する
4. diff が両 file の `sora_sdk` version 変更だけで、dependency version や checksum を変更していないことを semantic に検証する
5. 両 file だけを stage し、固定した commit message で detached candidate commit を作る
6. candidate commit の tree と parent を記録する
7. clean な candidate commit に対して検証 command を順番に実行する

検証 command は次へ固定する。

```console
uv sync --frozen
uv run --frozen ruff format --check canary.py tests/test_canary.py
uv run --frozen ruff check canary.py tests/test_canary.py
uv run --frozen ty check canary.py tests/test_canary.py
uv run --frozen pytest tests/test_canary.py -m "not release_orchestration"
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo package -p sora_sdk --locked
cargo publish -p sora_sdk --dry-run --locked
```

全検証後に candidate worktree の tracked diff と staged diff が空で、検証した `HEAD` が記録済み candidate commit のままであることを確認する。
一つでも失敗した場合は `git worktree remove --force <validated-candidate-path>`、対象 worktree 管理 entry の不在確認、temporary root cleanup の順で一時 worktree を削除し、primary HEAD、index、worktree、local refs、remote refs を開始時のままにする。
対象 entry の削除に失敗した場合は無関係な stale entry を `git worktree prune` せず、cleanup error と対象 path だけを表示する。
削除対象は script 自身が作成し、repository root 外にある検証済み temporary path だけとする。

SIGINT / SIGTERM は処理 phase ごとに次の状態へ収束させる。

- signal handler 自体は中断要求だけを記録し、cleanup、ref 操作、remote 照会を行わない
- subprocess は `shell=False` かつ専用 process group / session で起動し、signal、terminate、kill を group 全体へ伝播する
- bounded wait 後も group 内 process が残る場合は group 全体を kill し、direct child の wait / reap と process group の消滅を確認する
- validation process group の終了を確認してから一時 worktree を削除する
- prepared refs 作成前は一時 worktree を cleanup して `Fresh` に戻す
- prepared refs 作成後から push 開始前までは原子的に作成済みの両 local ref を保持して `Prepared` にする
- push 開始後は signal の有無にかかわらず、追跡している push process group の終了を確認してから remote の両 ref を再照会する
- signal、forced termination、transport failure、結果不明のいずれかを経た場合は、即時照会で両 remote ref が未更新でも attempt ref を保持して `RemoteUnknown` にする
- remote tag が candidate を指す場合は後述の `PotentialPublished` 検証へ進み、明確な remote conflict は `Conflict` にする
- Published 後の local fast-forward / cleanup 中は remote ref を戻さず、同じ target の再実行で local cleanup を完了できる状態を保持する

`--dry-run` も同じ candidate commit と全検証 command を実行する。
最後に一時 worktree の candidate `HEAD` を source として、次の atomic dry-run を実行する。

```console
git push --dry-run --atomic origin \
  HEAD:refs/heads/develop \
  HEAD:refs/tags/<version>
```

planned version、version diff、commit message、candidate commit、tag 種別、検証結果、最終 refspec を表示するが、primary file / index、local ref、remote ref は変更しない。
一時 detached commit の object は参照を作らず、worktree cleanup 後に回収可能な状態とする。

## local / remote state machine

target version ごとに、local `refs/canary/<version>`、`refs/canary-attempt/<version>`、local tag、primary `develop`、`origin/develop`、remote tag を object ID で照合する。

### Fresh

- primary `HEAD == origin/develop`
- current version は target の直前
- local prepared ref と local / remote target tag は存在しない

一時 worktree の全検証成功後、candidate commit を `refs/canary/<version>` に保存し、同じ commit を指す local lightweight tag を作る。
`git update-ref --stdin` の transaction で、両 ref の old object が zero であることを検証してから同時作成する。
process の同時実行、検証後の race、signal により片方だけを作成しない。
既に片方だけが存在する場合は `Conflict` とし、自動 cleanup しない。

### Prepared

- `refs/canary/<version>` と local tag が同じ candidate commit を指す
- `refs/canary-attempt/<version>` は存在しない
- candidate parent は primary `develop` と `git ls-remote` で得た remote develop object ID の両方に一致する
- candidate tree の `Cargo.toml` / `Cargo.lock` version と commit message が target に一致する
- remote target tag は存在しない

同じ `--version` による再実行では既存 candidate commit を作り直さない。
candidate parent との差分が `Cargo.toml` / `Cargo.lock` の root package version 変更だけであることを semantic に再検証し、candidate の一時 worktree で固定した Python / Rust / package validation をすべて再実行する。
再検証成功後、`git update-ref --stdin` transaction で prepared ref と local tag が candidate のままであることを検証し、`refs/canary-attempt/<version>` を candidate に作成してから直ちに次の 1 command を実行する。
attempt ref は marker 作成直後の process crash を含む「push が開始された可能性あり」の永続 marker とする。

```console
git push --atomic origin \
  refs/canary/<version>:refs/heads/develop \
  refs/tags/<version>:refs/tags/<version>
```

remote が atomic push 非対応、pre-receive hook が一方を拒否、remote branch が進んだ、同名 tag が作られた、通信に失敗した場合は error にする。
push 結果が不明な場合は remote の両 ref を再照会する。
remote 照会も失敗した場合は local prepared refs と attempt ref を保持する `RemoteUnknown` として、local / remote を変更せず exit non-zero にする。
次回の同じ `--version` 実行は最初に remote を再照会し、状態を確定するまで再 push しない。
remote が atomic push を明示的に拒否し、push process が正常に応答を返し、両 ref が未更新であることを確認できた場合だけ attempt ref を削除して `Prepared` に戻す。
remote tag が candidate なら `PotentialPublished`、結果不明のまま両方未更新なら `RemoteUnknown`、片方だけまたは別 commit なら `Conflict` とする。

### RemoteUnknown

- attempt ref の作成後、process crash、signal、forced termination、network / authentication / remote service failure により push の未実行または結果不明を script から区別できない
- local target tag と `refs/canary/<version>` は同じ検証済み candidate commit のまま保持する
- `refs/canary-attempt/<version>` が同じ candidate commit を指し、push が開始された可能性を永続的に示す

remote state が不明な間は再 push、abort、local ref cleanup、local branch 更新を行わない。
同じ target version の再実行は remote の branch / tag だけを再照会する。
remote tag が candidate なら `PotentialPublished`、remote に片側更新や別 object があれば `Conflict` に遷移する。
両方未更新でも先行する remote receive 処理の終了を script から証明できないため `RemoteUnknown` を維持し、Prepared へ自動遷移しない。

`--resolve-unpublished` は hosting provider 側を含む外部確認により先行 receive 処理が終了し、candidate が公開されていないと operator が確認した場合だけ使用する。
script は candidate metadata、3 個の local ref、remote tag が存在せず、取得した remote `develop` に candidate が含まれないことを再検証し、target version を含む確認文の完全入力を要求する。
確認後は `git update-ref --stdin` transaction で attempt ref だけを期待する candidate object から削除し、prepared ref と local tag を保持して push-attempt state を解消する。
remote `develop` が candidate parent のままなら Prepared として再開できる。
remote `develop` が candidate を含まない別 commit へ進んでいれば attempt ref 解消後の `Conflict` とし、`--abort` で script 所有の prepared ref / tag を安全に削除できる。
この操作は remote 未公開を自動証明せず、operator の外部確認を明示的に記録する回復境界である。

### PotentialPublished

- remote target tag が candidate commit を直接指す
- remote `develop` の object ID は `ls-remote` で得たが、local object database に存在しない場合がある

remote tag が candidate と一致した場合だけ、`git fetch --no-tags origin +refs/heads/develop:refs/remotes/origin/develop` で remote branch object を取得する。
取得後に candidate が `origin/develop` と同じか ancestor なら `Published`、そうでなければ `Conflict` とする。
RemoteUnknown の再開と `--abort` 前に remote branch が candidate を含むか確認する場合も、同じ取得と ancestry 検証を行う。

### Published

- remote target tag が candidate commit を指す
- candidate commit が remote `develop` と同じか ancestor である
- candidate tree と commit message が target に一致する
- local target tag は存在しないか candidate commit を指す
- `refs/canary/<version>` と `refs/canary-attempt/<version>` は cleanup 前の candidate commit を指すか、cleanup 済みで存在しない

`PotentialPublished` で取得した `origin/develop` に candidate が含まれ、remote tag が引き続き candidate を指すことを再照合する。
primary `develop` が取得した `origin/develop` の ancestor なら最新 remote `develop` へ `git merge --ff-only` する。
local tag の作成が必要な場合と prepared ref / attempt ref の削除は、期待する old object を検証する `git update-ref --stdin` transaction で処理する。
primary worktree が clean で `HEAD == origin/develop`、target tag が `HEAD` と同じか ancestor になったことを確認して exit 0 にする。
同じ `--version` の再実行は新しい version を作らず、Published の整合性確認だけで exit 0 にする。
state 分類では remote が Published を証明するこの条件を、local tag / prepared ref の片方だけが存在する Conflict 条件より先に評価する。
正常 cleanup 後の「local tag は candidate、prepared ref は不存在」を Published の定常状態とする。

### Conflict

次のいずれかは自動変更せず、local / remote の ref 名と短縮 object ID だけを表示して exit non-zero にする。

- remote branch と tag の片方だけが candidate commit を指す
- remote target tag が存在せず、candidate が remote branch に含まれる
- local / remote tag が異なる commit を指す
- remote branch が candidate と無関係な commit を指すか、candidate の ancestor に戻っている
- candidate tree、parent、commit message、version が一致しない
- remote が Published を証明していない状態で、local target tag と prepared ref の片方だけが存在する

credential を含み得る remote URL と command output の全文は表示しない。
force push、tag delete、reset、rebase を自動実行しない。

### abort

`--abort` は local target tag と `refs/canary/<version>` が同じ検証済み candidate commit を指し、attempt ref が存在せず、remote target tag が存在せず、candidate が remote `develop` と同じでも ancestor でもない場合だけ受理する。
remote `develop` が candidate parent のままか、別 process により無関係な commit へ進んだかは問わない。
object ID と candidate metadata を再検証してから、`git update-ref --stdin` の transaction で両 ref が期待する candidate object のままであることを検証し、local target tag と `refs/canary/<version>` を同時削除する。
primary branch、file、index、remote ref、candidate 以外の tag / ref は変更しない。
削除後に target の script 所有 local ref が存在せず、primary worktree が clean であることを確認する。
remote が candidate commit を branch または tag のどちらかに保持する場合は公開結果が不明なため拒否し、remote ref を自動削除しない。

## test

mock / stub、`monkeypatch`、fake subprocess runner、network service、実 `origin` を使わない。
`tmp_path` に実 Git repository、local bare remote、最小の実 Cargo package を作り、実 `git` / `cargo` process で検証する。
remote hook も local bare repository の実 `pre-receive` hook として実行し、subprocess の戻り値を差し替えない。
CLI を起動する end-to-end test には `release_orchestration` marker を付ける。
candidate worktree 内の validation は同じ CLI を再帰的に起動しないよう、この marker 以外の parser、version、candidate metadata test だけを実行する。
CI の通常 test は marker を除外せず全 test を実行する。
prek の pre-commit / pre-push pytest hook は `-m "not release_orchestration"` を固定し、candidate commit の hook から end-to-end test を再帰起動しない。
candidate commit で `--no-verify` は使用せず、marker 以外の hook を通常どおり実行する。

最低限、次を決定的に検証する。

- wrong directory、unborn branch、detached HEAD、wrong branch / upstream
- staged、unstaged、untracked の各 dirty state
- local ahead、remote ahead、diverged
- invalid current / target version と全 version 遷移境界
- `Cargo.toml` / `Cargo.lock` version 不一致
- local tag、remote lightweight tag、remote annotated tag の重複
- candidate diff に dependency update が混入した場合の拒否
- 実 Cargo package の fmt / clippy / test / package failure ごとに、primary file / index / refs が不変である
- dry-run の全検証成功後も primary file / index / refs と remote refs が不変である
- local bare remote への atomic push 成功後、remote develop と tag が同じ commit を指す
- `pre-receive` hook が tag ref だけを拒否しても、remote develop / tag が両方未更新である
- preflight 後に別 clone が remote develop を進めても、atomic push で remote develop / tag が両方未更新である
- Prepared から同じ target を再実行して candidate object ID と version が変わらない
- Prepared の再実行が candidate diff と全 validation を再検証する
- atomic push の結果照会で Prepared / Published / Conflict / RemoteUnknown を正しく分類する
- push 開始前に attempt ref を永続化し、結果不明の両 ref 未更新では再実行後も RemoteUnknown を保持する
- `--resolve-unpublished` が operator の外部確認後に attempt ref だけを削除し、remote develop に応じて Prepared または Conflict へ遷移する
- RemoteUnknown 中に別 clone が remote develop を candidate を含まない commit へ進めても、外部確認後に attempt ref を解消して Conflict / abort へ遷移できる
- push 後に remote develop が進んでも、candidate tag が最新 develop に含まれれば Published とする
- Published の再実行が no-op になる
- local prepared refs の作成と abort が `git update-ref --stdin` の transaction で原子的に行われる
- abort が script 所有の local refs だけを削除する
- 各 phase の SIGINT / SIGTERM で定義した再開可能 state に収束する
- signal を Python parent だけが受け、remote hook が push process group の消滅後まで更新を遅延させても、未更新の即時照会から Prepared と誤判定しない
- signal 後に validation の descendant process が一時 worktree を使用したまま残らない
- candidate cleanup 後に filesystem と `git worktree list --porcelain` の両方から一時 worktree が消える

pytest は test ごとの timeout を最大 10 秒にし、parameterized test はすべて意味のある `ids` を付ける。
テスト assertion message と comment は日本語、production script の error / log は英語かつ末尾に句点を付けない。

## Python tooling

Python 3.12、3.13、3.14 を対象とする。
`shiguredo-python` に従い、uv で dependency と lockfile を管理し、`from __future__ import annotations`、`__all__`、module 変数・属性を含む型、`Any` 不使用を満たす。

`pyproject.toml` に `requires-python = ">=3.12,<3.15"`、ruff、ty、pytest、pytest-timeout、`release_orchestration` marker と各設定を追加し、`uv.lock` を commit する。
tool dependency は実装時の最新 minor 系に下限を置き、次 minor 未満の上限と用途 comment を付ける。
`prek.toml` から `uv run --frozen` で ruff format / check、ty、`release_orchestration` marker を除外した pytest を実行する。
`.github/workflows/ci.yml` に Python 3.12 / 3.13 / 3.14 matrix の独立 job を追加し、次の SHA 固定 action を使用する。

- `actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`
- `actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0`
- `astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0`

CI と local で次が成功する。

```console
uv sync --frozen
uv run --frozen ruff format --check canary.py tests/test_canary.py
uv run --frozen ruff check canary.py tests/test_canary.py
uv run --frozen ty check canary.py tests/test_canary.py
uv run --frozen pytest tests/test_canary.py
```

## 変更対象

- `canary.py`
- `tests/test_canary.py`
- `pyproject.toml`
- `uv.lock`
- `prek.toml`
- `.github/workflows/ci.yml`
- `CHANGES.md`

`.github/workflows/release.yml` は issue 0099 の変更対象とし、本 issue では変更しない。

## pending 理由

2026-07-30 時点では、end-to-end の公開安全性を所有する issue 0099 が未実装である。
現行 release workflow のまま新しい script が tag を push すると、安全でない workflow を即時起動する。
0100 の local state machine は独立して実装可能だが、安全な canary release 操作として実運用・完了するには 0099 が必要なため pending にする。

issue 0099 の実装、workflow test、release environment / trusted publisher の確認が完了した後に reopened にする。
reopened 後は 0100 の local state machine を実装し、local bare remote test までは実際の GitHub / crates.io へ公開せずに完了させる。

## 完了条件

- issue 0099 の release workflow gate と再開可能な公開 state machine が実装・検証済みである
- preflight が repository、Git state、`develop` / `origin/develop`、clean、同期、version、local / remote tag を変更前に検証する
- required `--version` と current version から一意な canary 遷移だけを受理する
- candidate version の変更と commit は一時 worktree 内だけで行い、全 validation が成功するまで永続 local ref を作らない
- candidate commit の diff が `Cargo.toml` / `Cargo.lock` の root package version だけである
- Python / Rust / package の全 validation が同じ candidate commit に対して成功する
- validation failure、cancel、dry-run では primary HEAD、index、worktree、永続 local refs、remote refs が変化しない
- SIGINT / SIGTERM では subprocess の process group を停止・回収し、処理 phase に応じて Fresh / Prepared / Published / Conflict / RemoteUnknown の再開可能な状態へ収束する
- actual run は `refs/canary/<version>` と同じ commit を指す lightweight tag を作る
- branch と tag を完全な refspec の 1 回の `git push --atomic origin ...` で公開する
- push 前に attempt ref を永続化し、atomic 非対応、hook rejection、remote race、通信結果不明で Prepared / RemoteUnknown / PotentialPublished / Published / Conflict を正しく分類する
- RemoteUnknown は両 remote ref が未更新でも自動解除せず、外部確認を伴う `--resolve-unpublished` だけが attempt ref を削除して Prepared または Conflict へ遷移する
- remote tag が candidate commit を指し、candidate が remote develop と同じか ancestor の場合だけ local develop を最新 remote develop へ fast-forward して成功にする
- 同じ `--version` の再実行は candidate commit を再利用し、version を増やさない
- `--abort` は remote 未公開の script 所有 local refs だけを削除する
- mock / stub なしの temporary repository / bare remote test が全状態と atomicity を検証する
- Python 3.12 / 3.13 / 3.14 の CI が成功する
- `uv sync --frozen` が成功する
- `uv run --frozen ruff format --check canary.py tests/test_canary.py` が成功する
- `uv run --frozen ruff check canary.py tests/test_canary.py` が成功する
- `uv run --frozen ty check canary.py tests/test_canary.py` が成功する
- `uv run --frozen pytest tests/test_canary.py` が成功する
- `cargo fmt --all --check` が成功する
- `cargo clippy --workspace --all-targets -- -D warnings` が成功する
- `cargo test --workspace` が成功する
- `cargo package -p sora_sdk --locked` が成功する
- `cargo publish -p sora_sdk --dry-run --locked` が成功する
- release tooling の変更として `CHANGES.md` の `develop` に `### misc` を設け、`[FIX]` と担当者 `@voluntas` を追記する
- Python comment と test assertion message は日本語、production error / log は英語にする

## 参考

- `canary.py`
- `.github/workflows/release.yml`
- `.github/workflows/ci.yml`
- `issues/0099-other-strengthen-release-publication-gates.md`
- `issues/closed/0028-other-update-release-yml.md`
- Git `git-push` documentation の `--atomic`
- Cargo Book の Publishing on crates.io
- GitHub Actions workflow syntax の `permissions`
- GitHub CLI manual の `gh release create --verify-tag`
