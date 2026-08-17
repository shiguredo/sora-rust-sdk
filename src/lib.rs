//! Sora Rust SDK ― WebRTC SFU Sora の Rust クライアントライブラリ。
//!
//! このクレートは、[Sora] サーバーに接続して WebRTC による音声・映像の送受信を
//! 行うためのクライアント機能を提供する。
//!
//! # 主な機能
//!
//! - WebSocket 経由のシグナリング
//! - WebRTC による音声・映像の送受信
//! - DataChannel 経由のメッセージングと JSON-RPC 2.0
//! - 複数のハードウェア/ソフトウェアビデオコーデック対応
//! - MP4 ファイルのパススルー送信
//! - HTTP プロキシ経由の接続
//! - libcamera による映像キャプチャ (Linux)
//!
//! # 基本的な使い方
//!
//! ```no_run
//! use sora_sdk::{Result, SoraConnection, SoraConnectionContext, SoraConnectionEventHandler, Role};
//!
//! struct MyEventHandler;
//!
//! impl SoraConnectionEventHandler for MyEventHandler {
//!     fn on_notify(&mut self, text: &str) {
//!         println!("notify: {text}");
//!     }
//! }
//!
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() -> Result<()> {
//!     let context = SoraConnectionContext::new()?;
//!     let (connection, handle) = SoraConnection::builder(
//!         context,
//!         vec!["wss://sora.example.com/signaling".to_string()],
//!         "your-channel-id".to_string(),
//!         Role::SendRecv,
//!         MyEventHandler,
//!     )
//!     .build()?;
//!
//!     // run() は接続が終了するまでブロックするため、別タスクで実行する。
//!     // 以降、connection への操作は handle を利用する
//!     let run_task = tokio::spawn(async move { connection.run().await });
//!
//!     // 任意のタイミングで切断し、run() の完了を待つ。
//!     handle.disconnect().await?;
//!     let run_result = run_task
//!         .await
//!         .map_err(|_| std::io::Error::other("run task panicked"))?;
//!     run_result?;
//!     Ok(())
//! }
//! ```
//!
//! [Sora]: https://sora.shiguredo.jp/
#![warn(missing_docs)]
mod connection;
mod connection_context;
mod connection_event_handler;
mod error;
#[cfg(feature = "libcamera")]
mod libcamera;
mod rpc;
mod signaling_types;
#[cfg(test)]
mod testing;
mod types;
mod version;
mod video_codec;
mod video_codec_capability;
mod video_codec_preference;
mod video_codecs;
mod zlib;

pub use crate::connection::{
    ParsedProxyInfo, SoraConnection, SoraConnectionBuilder, SoraConnectionHandle,
};
pub use crate::connection_context::{
    AdmConfig, SoraConnectionContext, SoraConnectionContextConfig,
};
pub use crate::connection_event_handler::SoraConnectionEventHandler;
pub use crate::error::{Error, Result};
#[cfg(feature = "libcamera")]
pub use crate::libcamera::{
    LibcameraNativeFrameBuffer, LibcameraVideoCapturer, LibcameraVideoCapturerBuilder,
};
pub use crate::rpc::{RpcRequestOptions, RpcResponse};
pub use crate::types::{
    Audio, AudioCodecType, AudioOpusParams, ConnectDataChannel, ForwardingFilter,
    ForwardingFilterRule, JsonString, ProxyInfo, Role, SignalingDirection, SignalingType, Video,
    VideoAV1Params, VideoCodecType, VideoH264Params, VideoH265Params, VideoVP9Params,
};
pub use crate::video_codec::{
    AlignmentEncoderAdapter, SimulcastCapabilityHelper, SoraVideoDecoderFactory,
    SoraVideoEncoderFactory, codec_type_from_format,
};
pub use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};
pub use crate::video_codec_preference::{
    PreferenceCodec, VideoCodecPreference, validate_video_codec_preference,
};
#[cfg(feature = "amf")]
pub use crate::video_codecs::amf::AmfVideoCodecCapability;
pub use crate::video_codecs::internal::InternalVideoCodecCapability;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use crate::video_codecs::internal_apple::InternalAppleVideoCodecCapability;
pub use crate::video_codecs::mp4::{
    Mp4BitstreamMetadata, Mp4Error, Mp4PassthroughVideoCodecCapability, Mp4SampleReader,
    Mp4VideoCapturer,
};
#[cfg(feature = "nvcodec")]
pub use crate::video_codecs::nvcodec::NvCodecVideoCodecCapability;
#[cfg(feature = "openh264")]
pub use crate::video_codecs::openh264::Openh264VideoCodecCapability;
#[cfg(feature = "v4l2")]
pub use crate::video_codecs::v4l2::V4l2VideoCodecCapability;
#[cfg(feature = "vpl")]
pub use crate::video_codecs::vpl::VplVideoCodecCapability;
