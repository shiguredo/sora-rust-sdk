# 公開 API の rustdoc 拡充

- Priority: High
- Created: 2026-07-03
- Completed: 2026-07-03
- Model: DeepSeek V4 Pro
- Branch: feature/update-add-rustdoc-to-public-api
- Polished: 2026-07-03

## 目的

公開 API に rustdoc を追加し、`cargo doc` で完全な API リファレンスを生成できるようにする。

## 優先度根拠

- 正式リリース前に必須（親 issue: #0020 の Must 項目 M6）
- rustdoc がないと利用者が公開 API の使い方を理解できない
- 公開後はドキュメントの不備がそのまま利用者への不利益になる

## 現状

各ファイルの公開型（ `pub struct` / `pub enum` / `pub trait` / `pub type` / `pub fn`（トレイト実装を除く））に対する `///` と、モジュールへの `//!` の付与状況を以下に示す。
型定義に `///` が付与されていても個別のメソッドに `///` がない場合は「一部メソッド未記載」と注記している。

| ファイル | `//!` | `///` 未付与 | `///` 付与済 |
|---|---|---|---|
| `src/lib.rs` | 不十分（ 1 行のみ） | 再エクスポート 30 件 | — |
| `src/types.rs` | あり | 全公開型・全公開メソッド（ 17 型 + 多数のメソッド） | — |
| `src/error.rs` | あり | `Error` enum と全バリアント（ 50 件超） | `Result`（ #0034 で追加済） |
| `src/video_codec.rs` | なし | `SoraVideoEncoderFactory`, `SoraVideoDecoderFactory`, `SimulcastCapabilityHelper`, `codec_type_from_format` | `AlignmentEncoderAdapter` |
| `src/video_codec_capability.rs` | なし | `VideoCodecImplementation`, `CodecDirection` | `VideoCodecCapability`（ trait 本体） |
| `src/video_codec_preference.rs` | なし | `PreferenceCodec`, `VideoCodecPreference`, `validate_video_codec_preference` | — |
| `src/connection.rs` | あり | `SoraConnection`（ `run()` 等の一部メソッド）, `SoraConnectionBuilder`（全ビルダーメソッド未記載, 約 35 件）, `SoraConnectionHandle::get_stats()` | `TlsConfig`, `SoraConnectionHandle`（ 型定義と主要メソッド）, `ParsedProxyInfo` |
| `src/connection_context.rs` | あり | `AdmConfig`, `SoraConnectionContext::new()` | `SoraConnectionContextConfig`, `SoraConnectionContext`（ 型定義と `new_with_config()` / `create_audio_source()` / `create_audio_track()` / `create_video_track()`） |
| `src/rpc.rs` | あり | — | `RpcRequestOptions`, `RpcResponse`（ 型定義・フィールド・バリアントすべて `///` 付与済） |
| `src/libcamera.rs` (`#[cfg(feature = "libcamera")]`) | なし | `LibcameraVideoCapturerBuilder`, `LibcameraVideoCapturer`, `LibcameraNativeFrameBuffer`（ 3 型すべて完全未記載） | — |
| `src/video_codecs/mod.rs` | なし | — | — |
| `src/video_codecs/internal.rs` | なし | `InternalVideoCodecCapability` | — |
| `src/video_codecs/internal_apple.rs` | なし | `InternalAppleVideoCodecCapability` | — |
| `src/video_codecs/amf.rs` | なし | `AmfVideoCodecCapability` | — |
| `src/video_codecs/nvcodec.rs` | なし | `NvCodecVideoCodecCapability` | — |
| `src/video_codecs/openh264.rs` | なし | `Openh264VideoCodecCapability` | — |
| `src/video_codecs/v4l2.rs` | なし | `V4l2VideoCodecCapability` | — |
| `src/video_codecs/vpl.rs` | なし | `VplVideoCodecCapability` | — |
| `src/video_codecs/mp4.rs` | あり | `Mp4VideoCapturer`（一部メソッド） | `Mp4PassthroughVideoCodecCapability`, `Mp4SampleReader`, `Mp4EncodedSample` |

## 設計方針

- 全公開型（ `pub struct` / `pub enum` / `pub trait` / `pub type`）に `///` ドキュメントコメントを追加する
- `pub struct` の全公開フィールド、enum の全バリアントとそのフィールドに `///` を追加する
- 全 `pub fn`（Rust 標準ライブラリで定義されたトレイトおよび `DisplayJson` の実装に伴う `fn` を除く）に `///` を追加する
- セッターメソッドは「XXX を設定する」の形式で書き、デフォルト値がある場合は併記する
- ショートカットメソッド（例: `new()` → `new_with_config(Default::default())` のラッパー）では委譲先を `[Type::method]` で参照する
- 全モジュールに `//!` モジュールドキュメントを追加する
- `src/lib.rs` にクレート全体の説明を `//!` で追加する
- ドキュメントは日本語で書く（ CLAUDE.md の規約に従う）
- `SKILL.md` は LLM エージェント向けの参照資料であり、人間向けの rustdoc とは目的が異なる。人間向けの rustdoc は自己完結した説明を書く。`SKILL.md` への参照案内は不要
- 型の説明・メソッドの説明・パニック条件・エラー条件を明記する。ただし通常の利用でパニックしないものは省略可能
- `#[cfg]` で conditionally compiled なアイテムにはその旨を記載する
- re-export の rustdoc は定義元に付与する。`lib.rs` の `pub use` 行自体への `///` 追加は不要（ re-export の存廃自体は本 issue では扱わない。親 issue #0020 の M5 参照）
- 既に rustdoc が付与されているアイテム（ `Result`, `AlignmentEncoderAdapter`, `VideoCodecCapability` 等）の内容は維持する
- 公開型への参照には `[TypeName]` 記法（ intra-doc link ）を使い、`cargo doc` で解決可能であること
- 新規の doc-test は追加しない（既存の `Result` の doc-test のみ維持する）

## 完了条件

- 上記「現状」テーブルの全ファイルで、公開型・enum バリアント・全 `pub fn`（トレイト実装を除く）に `///` が付与されている
- 全ファイルにモジュールレベルの `//!` が付与されている
- `src/lib.rs` にクレートレベルの `//!` が付与されている
- `cargo doc --no-deps --all-features` を `RUSTDOCFLAGS="-D warnings -D rustdoc::broken_intra_doc_links"` で実行し警告なく完了する
- `cargo test --doc --all-features` を同一の `RUSTDOCFLAGS` で実行し既存の doc-test が通過する
- 全ドキュメント追加完了後、`src/lib.rs` に `#![warn(missing_docs)]` を追加する（`--all-features` で警告ゼロ）
- `CHANGES.md` の `## develop` に `[ADD]` エントリを追記する

## 解決方法

1. `src/lib.rs` にクレート全体の説明を `//!` で追加する
2. `src/types.rs` の全公開型・全 `pub fn`（トレイト実装を除く）・enum バリアント・公開フィールドに `///` を追加する
3. `src/error.rs` の `Error` enum と全バリアント（ 50 件超）に `///` を追加する
4. `src/video_codec.rs` に `//!` を追加し、`SoraVideoEncoderFactory`, `SoraVideoDecoderFactory`, `SimulcastCapabilityHelper`, `codec_type_from_format` の型定義・全 `pub fn` に `///` を追加する
5. `src/video_codec_capability.rs` に `//!` を追加し、`VideoCodecImplementation`, `CodecDirection` の型定義・全 `pub fn` に `///` を追加する
6. `src/video_codec_preference.rs` に `//!` を追加し、`PreferenceCodec`, `VideoCodecPreference`, `validate_video_codec_preference` の型定義・全 `pub fn` に `///` を追加する
7. `src/connection.rs` の `SoraConnection::run()`, `SoraConnectionBuilder` の全ビルダーメソッド（ 約 35 件）, `SoraConnectionHandle::get_stats()` に `///` を追加する
8. `src/connection_context.rs` の `AdmConfig` の型定義・バリアント, `SoraConnectionContext::new()` に `///` を追加する（ `new()` は `[SoraConnectionContext::new_with_config]` を参照する）
9. `src/libcamera.rs` に `//!` を追加し、`LibcameraVideoCapturerBuilder`, `LibcameraVideoCapturer`, `LibcameraNativeFrameBuffer` の型定義・全 `pub fn` に `///` を追加する（ `#[cfg(feature = "libcamera")]` 条件の記載を含む）
10. `src/video_codecs/mod.rs` に `//!` を追加する
11. `src/video_codecs/internal.rs` に `//!` を追加し、`InternalVideoCodecCapability` に `///` を追加する
12. `src/video_codecs/internal_apple.rs` に `//!` を追加し、`InternalAppleVideoCodecCapability` に `///` を追加する（ `#[cfg(any(target_os = "macos", target_os = "ios"))]` 条件の記載を含む）
13. `src/video_codecs/amf.rs` に `//!` を追加し、`AmfVideoCodecCapability` に `///` を追加する（ `#[cfg(feature = "amf")]` 条件の記載を含む）
14. `src/video_codecs/nvcodec.rs` に `//!` を追加し、`NvCodecVideoCodecCapability` に `///` を追加する（ `#[cfg(feature = "nvcodec")]` 条件の記載を含む）
15. `src/video_codecs/openh264.rs` に `//!` を追加し、`Openh264VideoCodecCapability` に `///` を追加する（ `#[cfg(feature = "openh264")]` 条件の記載を含む）
16. `src/video_codecs/v4l2.rs` に `//!` を追加し、`V4l2VideoCodecCapability` に `///` を追加する（ `#[cfg(feature = "v4l2")]` 条件の記載を含む）
17. `src/video_codecs/vpl.rs` に `//!` を追加し、`VplVideoCodecCapability` に `///` を追加する（ `#[cfg(feature = "vpl")]` 条件の記載を含む）
18. `src/video_codecs/mp4.rs` の `Mp4VideoCapturer` の未ドキュメントメソッドに `///` を追加する
19. 完了条件の `cargo doc --no-deps --all-features` および `cargo test --doc --all-features` を `RUSTDOCFLAGS` 付きで実行し、警告・エラーがないことを確認する
20. `src/lib.rs` に `#![warn(missing_docs)]` を追加し、`--all-features` で警告ゼロを確認する
21. `CHANGES.md` の `## develop` に `[ADD]` エントリを追記する

## 親 issue

- #0020
