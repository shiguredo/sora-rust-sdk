//! ビデオコーデックの実装モジュール。
//!
//! 各プラットフォーム・ハードウェア向けの
//! [VideoCodecCapability] 実装を提供する。
#[cfg(feature = "amf")]
pub mod amf;
pub(crate) mod av1;
pub(crate) mod h264;
#[cfg(any(
    feature = "v4l2",
    feature = "vpl",
    feature = "amf",
    feature = "nvcodec",
    feature = "openh264"
))]
pub(crate) mod helpers;
pub mod internal;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub mod internal_apple;
pub mod mp4;
#[cfg(feature = "nvcodec")]
pub mod nvcodec;
#[cfg(feature = "openh264")]
pub mod openh264;
#[cfg(feature = "v4l2")]
pub mod v4l2;
// VPL は Linux 専用 (README 参照) のため、feature が有効でも他 OS ではモジュールをコンパイルしない。
// shiguredo_vpl の `supported_codecs` 等は `#[cfg(target_os = "linux")]` でしか定義されておらず、
// 他 OS で有効化するとビルドエラーになるため、モジュール単位で OS ガードする。
#[cfg(all(feature = "vpl", target_os = "linux"))]
pub mod vpl;
