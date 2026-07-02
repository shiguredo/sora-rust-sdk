# SKILL.md のバージョン追従ルールと iOS 対応の確定

- Priority: High
- Created: 2026-07-03
- Completed: {YYYY-MM-DD}
- Model: DeepSeek V4 Pro
- Branch: feature/doc-fix-skill-version-and-ios-gate
- Polished: {YYYY-MM-DD}

## 目的

`skills/sora-rust-sdk/SKILL.md` にハードコードされた canary バージョン番号が `Cargo.toml` の実バージョンと乖離する問題を解消し、`InternalAppleVideoCodecCapability` の iOS gate を確定する。

## 優先度根拠

- 正式リリース前に必須（親 issue: #0020 の Must 項目 M11）
- SKILL.md のバージョン表記が実態と一致していないと、AI エージェントが誤った API 情報を参照する
- iOS ビルド要件（「iOS はビルド可能である必要があります」）を満たすための設計判断が必要

## 現状

### SKILL.md のバージョン乖離

`skills/sora-rust-sdk/SKILL.md:26` に `2026.1.0-canary.10` とハードコードされているが、`Cargo.toml` の実際のバージョンは `2026.1.0-canary.11` で一致していない。`canary.py` は `Cargo.toml` と `Cargo.lock` のみを更新し、SKILL.md の更新ロジックは含まれていない。

### InternalAppleVideoCodecCapability の iOS gate

`InternalAppleVideoCodecCapability` は以下の 4 箇所で `#[cfg(any(target_os = "macos", target_os = "ios"))]` でゲートされている:

- `src/video_codecs/mod.rs:4` - モジュール宣言
- `src/lib.rs:47` - `pub use`
- `src/connection_context.rs:16` - `use`
- `src/connection_context.rs:49` - `default()` 内の初期化

一方、iOS のビルドターゲット構成（`.cargo/config.toml`）、OS 検出（`get_os_info()`）、オーディオキャプチャ（`shiguredo_audio_device`）は iOS 未対応であり、正式サポートは macOS のみとなっている。

## 設計方針

### SKILL.md のバージョン表記

canary のたびに SKILL.md のバージョン番号を手動更新する運用を廃止し、バージョン表記を粗くする。具体的には `2026.1.0-canary.10` を `2026.1.0-canary` のように canary のマイナーバージョンを含まない表記に変更する。正式リリース後は `2026.1.0` になる。

### iOS gate

iOS はビルド可能である必要があるため、`#[cfg(any(target_os = "macos", target_os = "ios"))]` を維持する。`InternalAppleVideoCodecCapability` の内部実装（`VideoEncoderFactory::from_objc_default()`）は ObjC ランタイムを使用しており、iOS ターゲットでもコンパイルは通る。

## 完了条件

- SKILL.md のバージョン表記が `2026.1.0-canary` または `2026.1.0` のような canary 番号を含まない表記になっている
- `InternalAppleVideoCodecCapability` の `#[cfg]` ゲートが `target_os = "ios"` を含む状態で維持されている
- iOS ターゲットでのビルドが成功することを確認できている

## 解決方法

1. `skills/sora-rust-sdk/SKILL.md:26` の `2026.1.0-canary.10` を `2026.1.0-canary` に変更する
2. `InternalAppleVideoCodecCapability` の `#[cfg]` ゲートは変更しない（維持する）
3. iOS ターゲットでのビルドを確認する

## 親 issue

- #0020
