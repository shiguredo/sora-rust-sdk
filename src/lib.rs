//! 公開 API の入口。
mod client;
mod client_context;
mod error;
mod rpc;
mod signaling_types;
mod types;
mod version;
mod video_codec;
mod video_codec_capability;
mod video_codec_preference;
mod video_codecs;
mod zlib;

pub use crate::client::{
    SoraClient, SoraClientBuilder, SoraClientCommand, SoraClientHandle, TlsConfig,
};
pub use crate::client_context::{AdmConfig, SoraClientContext, SoraClientContextConfig};
pub use crate::error::{Error, Result};
pub use crate::rpc::{RpcRequestOptions, RpcResponse};
pub use crate::types::{
    Audio, AudioCodecType, ConnectDataChannel, ForwardingFilter, ForwardingFilterRule, JsonString,
    OpusParams, ProxyInfo, Role, SignalingDirection, SignalingType, Video, VideoAV1Params,
    VideoCodecType, VideoH264Params, VideoH265Params, VideoVP9Params,
};
pub use crate::video_codec::{SoraVideoDecoderFactory, SoraVideoEncoderFactory};
pub use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};
pub use crate::video_codec_preference::{
    PreferenceCodec, VideoCodecPreference, validate_video_codec_preference,
};
pub use crate::video_codecs::internal::InternalVideoCodecCapability;
pub use crate::video_codecs::mp4::{
    Mp4EncodedSample, Mp4Error, Mp4PassthroughVideoCodecCapability, Mp4SampleReader,
    Mp4VideoCapturer,
};
#[cfg(feature = "nvcodec")]
pub use crate::video_codecs::nvcodec::NvCodecVideoCodecCapability;
