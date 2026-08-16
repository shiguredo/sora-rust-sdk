//! ビデオコーデックの実装モジュール。
//!
//! 各プラットフォーム・ハードウェア向けの
//! [VideoCodecCapability] 実装を提供する。
#[cfg(feature = "amf")]
pub mod amf;
#[cfg(any(
    feature = "v4l2",
    feature = "vpl",
    feature = "amf",
    feature = "nvcodec",
    feature = "openh264"
))]
pub mod helpers;
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
#[cfg(feature = "vpl")]
pub mod vpl;
