# SoraConnectionContextConfig に libwebrtc の Environment を指定できるようにする

- Created: 2026-09-19
- Completed: {YYYY-MM-DD}
- Branch: feature/add-webrtc-environment
- Polished: {YYYY-MM-DD}

## 目的

`SoraConnectionContextConfig` に libwebrtc の `Environment` を指定できるようにし、利用側が
フィールドトライアル (`WebRTC-Video-PerSsrcKeyframes` など) を有効にした `Environment` を
ADM と `PeerConnectionFactory` で共通に使えるようにする。

`WebRTC-Video-PerSsrcKeyframes` を有効にすると、libwebrtc の `EncoderRtcpFeedback` が PLI/FIR を
SSRC 単位で処理し、PLI を受けた SSRC に対応するレイヤーだけキーフレームを生成する。
キーフレームの最小送出間隔も SSRC ごとに独立する。Sora 本体ではサイマルキャストの PLI を
rid 単位で制御する対応が進行中であり、配信側の SDK でこのフィールドトライアルを有効にすると、
Sora から rid 単位で届く PLI に対して要求されたレイヤーのキーフレームだけを生成できる。

## 現状

- `SoraConnectionContextConfig` (`src/connection_context.rs`) には libwebrtc の `Environment` を
  指定する項目がない
- `SoraConnectionContext::new_with_config` は `Environment::new()` で生成した `Environment` を
  ADM の生成にだけ使い、`PeerConnectionFactoryDependencies::set_env` を呼んでいない。
  このため `PeerConnectionFactory` 側は C ラッパーの `PeerConnectionFactoryWithContext::Create` が
  生成する別の `Environment` を使い、ADM と `PeerConnectionFactory` で `Environment` が共有されない
- `AdmConfig::UseExternal` を使う場合、ADM は `SoraConnectionContext` の生成前に作る必要がある。
  SDK が内部で `Environment` を生成する構成では、利用側が ADM と `PeerConnectionFactory` で
  同じ `Environment` を共有する手段がない
- `shiguredo_webrtc` 0.154.1-canary.0 は以下を提供している
  - `Environment` / `EnvironmentFactory` / `FieldTrials`。`FieldTrials::new` は不正な文字列に対して
    `shiguredo_webrtc::Error::InvalidFieldTrials` を返す
  - `PeerConnectionFactoryDependencies::set_env(Option<Environment>)`
  - C ラッパーの `PeerConnectionFactoryWithContext::Create` は
    `dependencies.env.value_or(webrtc::CreateEnvironment())` で `dependencies.env` を尊重し、
    同じ `Environment` を `webrtc::ConnectionContext::Create` と `webrtc::PeerConnectionFactory` に渡す
  - エンコーダー/デコーダーの生成時に libwebrtc から渡される `Environment` は、
    `VideoCodecCapability::create_video_encoder` と `AudioCodecCapability::create_audio_encoder` の
    `EnvironmentRef` として既に届いている

## 設計方針

- `SoraConnectionContextConfig` に `pub environment: Option<Environment>` を追加する。
  `Default` は `None`
  - `None` の場合は `Environment::new()` で生成した既定の `Environment` を使う
  - フィールドトライアル文字列のパースは SDK では行わない。利用側が `FieldTrials` と
    `EnvironmentFactory` で `Environment` を生成して指定し、不正な文字列は利用側が
    `FieldTrials::new` のエラーとして受け取る
- `SoraConnectionContext::new_with_config` で、指定された `Environment` を ADM と
  `PeerConnectionFactoryDependencies::env` の両方に渡す
  - `AudioDeviceModule::new(&environment, layer)` に同じ `Environment` を渡す
  - ADM の生成後に `deps.set_env(Some(environment.clone()))` を呼び、
    `PeerConnectionFactory::create_modular_with_context` の前に設定する
  - C ラッパー経由で `webrtc::ConnectionContext` にも同じ `Environment` が渡る
  - `AdmConfig::UseExternal` の場合、利用側が同じ `Environment` で ADM を生成できる
- 生成に使った `Environment` は `SoraConnectionContext` が保持する
- `environment` はシグナリングメッセージには含めないクライアント側の設定とする
- C++ SDK の `SoraClientContextConfig::field_trials` は文字列を受け取るが、Rust SDK は
  `Environment` を受け取る。C++ SDK と違い `AdmConfig::UseExternal` で利用側が ADM を生成する
  経路があるため、同じ `Environment` を共有できることを優先する
- `examples/sumomo` などのサンプルへの CLI オプション追加は行わない

## 完了条件

- `SoraConnectionContextConfig` に `environment` を追加し、指定された `Environment` を ADM と
  `PeerConnectionFactoryDependencies::env` の両方に渡すこと
- `environment` が `None` の場合は `Environment::new()` を使い、既存の動作から変わらないこと
- `AdmConfig::UseExternal` で利用側が指定した `Environment` と同じ `Environment` を
  `PeerConnectionFactory` にも渡すこと
- フィールドトライアル付きの `Environment` を指定した場合、エンコーダー生成時に
  `VideoCodecCapability::create_video_encoder` へ渡される `EnvironmentRef` でそのフィールド
  トライアルが有効になっていること
- `SoraConnectionContextConfig::default()` を使う既存の利用側に回帰がないこと
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が成功すること
- コメントは日本語、ログメッセージは英語、テストの assertion message は日本語で書くこと
- モックやスタブは使用しないこと
- `CHANGES.md` の `## develop` に `[ADD]` を追記すること

## 変更対象

- `src/connection_context.rs`（`environment` の追加、`Environment` の ADM /
  `PeerConnectionFactoryDependencies` への受け渡し）
- `skills/sora-rust-sdk/SKILL.md`（`SoraConnectionContextConfig` のフィールド一覧と
  フィールドトライアルの設定例）
- `e2e-tests/src/` / `e2e-tests/tests/`（フィールドトライアルの E2E テスト）
- `CHANGES.md`

## テスト方針

- 単体テスト (`src/connection_context.rs`)
  - フィールドトライアル付きの `Environment` を指定して `new_with_config` を呼び、
    `SoraConnectionContext` が保持する `Environment` の
    `field_trials().is_enabled("WebRTC-Video-PerSsrcKeyframes")` が true になることを確認する
  - `environment` が `None` の場合にフィールドトライアルが有効にならないことを確認する
- E2E テスト (`e2e-tests/tests/`)
  - `InternalVideoCodecCapability` に委譲しつつ `create_video_encoder` に渡された
    `EnvironmentRef` を記録する `VideoCodecCapability` を実装する
  - フィールドトライアル付きの `Environment` を指定したコンテキストで Sora に接続し、
    エンコーダー生成時に記録した `EnvironmentRef` でフィールドトライアルが有効になっている
    ことを確認する
  - `create_video_encoder` は libwebrtc のスレッドから呼ばれるため、その場で assert せず記録して
    テスト本体で検証する
- per-SSRC のキーフレーム生成というフィールドトライアル自体の効果は libwebrtc 内部のため
  本 issue のテスト対象外とする
