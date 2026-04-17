//! 公開 API の入口。
mod connection;
mod connection_context;
mod error;
#[cfg(feature = "libcamera")]
mod libcamera;
mod rpc;
mod signaling_types;
mod types;
mod version;
mod video_codec;
mod video_codec_capability;
mod video_codec_preference;
mod video_codecs;
mod zlib;

pub use crate::connection::{
    SoraConnection, SoraConnectionBuilder, SoraConnectionCommand, SoraConnectionHandle, TlsConfig,
};
pub use crate::connection_context::{
    AdmConfig, SoraConnectionContext, SoraConnectionContextConfig,
};
pub use crate::error::{Error, Result};
#[cfg(feature = "libcamera")]
pub use crate::libcamera::{LibcameraVideoCapturer, LibcameraVideoCapturerBuilder};
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
    Mp4EncodedSample, Mp4Error, Mp4PassthroughVideoCodecCapability, Mp4SampleReader,
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
