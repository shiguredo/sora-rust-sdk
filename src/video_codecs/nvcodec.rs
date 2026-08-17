//! NVIDIA Video Codec SDK を使用したハードウェアエンコーダー/デコーダー実装。
//!
//! `#[cfg(feature = "nvcodec")]` でのみコンパイルされる。
use std::sync::{Arc, Mutex};

use shiguredo_nvcodec::{
    Av1EncoderConfig, BufferFormat, CodecConfig, Decoder, DecoderCodec, DecoderConfig,
    EncodeOptions, Encoder, EncoderConfig, FnDecodeHandler, FnEncodeHandler, H264EncoderConfig,
    HevcEncoderConfig, PictureType, Preset, RateControlMode, ReconfigureParams, SurfaceFormat,
    TuningInfo, supported_codecs,
};
use shiguredo_webrtc::{
    CodecSpecificInfo, EncodedImage, EncodedImageBuffer, EncodedImageRef, EnvironmentRef,
    H264PacketizationMode, NV12Buffer, SdpVideoFormat, SdpVideoFormatRef, VideoCodecRef,
    VideoCodecStatus, VideoCodecType, VideoDecoder, VideoDecoderDecodedImageCallbackPtr,
    VideoDecoderDecoderInfo, VideoDecoderHandler, VideoDecoderSettingsRef, VideoEncoder,
    VideoEncoderEncodedImageCallbackPtr, VideoEncoderEncodedImageCallbackRef,
    VideoEncoderEncodedImageCallbackResultError, VideoEncoderEncoderInfo, VideoEncoderHandler,
    VideoEncoderRateControlParametersRef, VideoEncoderSettingsRef, VideoFrame, VideoFrameRef,
    VideoFrameType, VideoFrameTypeVectorRef, i420_to_nv12, nv12_copy, rtc_log_error,
    rtc_log_warning,
};

use crate::error::{Error, Result};
use crate::video_codec::{SimulcastCapabilityHelper, codec_type_from_format};
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};
use crate::video_codecs::helpers;

// コーデック種別から NVCODEC 固有のエンコーダー設定を返す。
fn encoder_codec_config(codec_type: VideoCodecType) -> Option<CodecConfig> {
    match codec_type {
        VideoCodecType::H264 => Some(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        })),
        VideoCodecType::H265 => Some(CodecConfig::Hevc(HevcEncoderConfig {
            profile: None,
            idr_period: None,
        })),
        VideoCodecType::Av1 => Some(CodecConfig::Av1(Av1EncoderConfig {
            profile: None,
            idr_period: None,
        })),
        _ => None,
    }
}

// コーデック種別から NVCODEC 固有のデコーダー設定を返す。
fn decoder_codec(codec_type: VideoCodecType) -> Option<DecoderCodec> {
    match codec_type {
        VideoCodecType::H264 => Some(DecoderCodec::H264),
        VideoCodecType::H265 => Some(DecoderCodec::Hevc),
        VideoCodecType::Av1 => Some(DecoderCodec::Av1),
        VideoCodecType::Vp8 => Some(DecoderCodec::Vp8),
        VideoCodecType::Vp9 => Some(DecoderCodec::Vp9),
        _ => None,
    }
}

fn collect_supported_formats(device_id: i32) -> Result<(Vec<SdpVideoFormat>, Vec<SdpVideoFormat>)> {
    let codec_infos = supported_codecs(device_id)?;

    let mut encoder_supported_formats = Vec::new();
    let mut decoder_supported_formats = Vec::new();
    for info in codec_infos {
        let codec_type = match info.codec {
            shiguredo_nvcodec::VideoCodecType::H264 => VideoCodecType::H264,
            shiguredo_nvcodec::VideoCodecType::Hevc => VideoCodecType::H265,
            shiguredo_nvcodec::VideoCodecType::Av1 => VideoCodecType::Av1,
            shiguredo_nvcodec::VideoCodecType::Vp8 => VideoCodecType::Vp8,
            shiguredo_nvcodec::VideoCodecType::Vp9 => VideoCodecType::Vp9,
            shiguredo_nvcodec::VideoCodecType::Jpeg => continue,
        };

        if info.encoding.supported {
            encoder_supported_formats.extend(helpers::supported_formats_for_codec(codec_type));
        }
        if info.decoding.supported {
            decoder_supported_formats.extend(helpers::supported_formats_for_codec(codec_type));
        }
    }
    Ok((encoder_supported_formats, decoder_supported_formats))
}

fn nvcodec_reconfigure_params(target_bitrate_bps: u32, framerate: u32) -> ReconfigureParams {
    ReconfigureParams {
        framerate_num: Some(framerate.max(1)),
        framerate_den: Some(1),
        average_bitrate: Some(target_bitrate_bps.max(1)),
        ..ReconfigureParams::default()
    }
}

// NVCODEC 固有の `PictureType` を `VideoFrameType` に変換する。
fn frame_type_from_nvcodec(picture_type: PictureType) -> VideoFrameType {
    match picture_type {
        PictureType::Idr | PictureType::I => VideoFrameType::Key,
        PictureType::P | PictureType::B | PictureType::Unknown => VideoFrameType::Delta,
        _ => VideoFrameType::Delta,
    }
}

#[derive(Clone, Copy)]
struct EncoderCallbackValue {
    rtp_timestamp: u32,
    frame_width: u32,
    frame_height: u32,
}

#[derive(Default)]
struct EncoderCallbackState {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
}

fn handle_nvcodec_encode_callback(
    callback_state: &Arc<Mutex<EncoderCallbackState>>,
    result: Result<shiguredo_nvcodec::EncodedFrame<EncoderCallbackValue>>,
    codec_type: VideoCodecType,
) {
    let encoded = match result {
        Ok(encoded) => encoded,
        Err(err) => {
            rtc_log_error!(
                "NVCODEC encode callback failed for {:?}: {}",
                codec_type,
                err
            );
            return;
        }
    };

    let picture_type = encoded.picture_type();
    let output_frame_type = frame_type_from_nvcodec(picture_type);
    let value = encoded.user_data();
    let rtp_timestamp = value.rtp_timestamp;
    let frame_width = value.frame_width;
    let frame_height = value.frame_height;

    let mut encoded_image = EncodedImage::new();
    let encoded_buffer = EncodedImageBuffer::from_bytes(encoded.data());
    encoded_image.set_encoded_data(&encoded_buffer);
    encoded_image.set_rtp_timestamp(rtp_timestamp);
    encoded_image.set_encoded_width(frame_width);
    encoded_image.set_encoded_height(frame_height);
    encoded_image.set_frame_type(output_frame_type);

    let mut codec_specific_info = CodecSpecificInfo::new();
    codec_specific_info.set_codec_type(codec_type);
    if codec_type == VideoCodecType::H264 {
        codec_specific_info.set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
        codec_specific_info.set_h264_idr_frame(matches!(picture_type, PictureType::Idr));
    }

    let result = {
        let callback_state = callback_state
            .lock()
            .expect("callback_state should not be poisoned");
        let Some(callback) = callback_state.callback else {
            return;
        };
        unsafe {
            callback.on_encoded_image(encoded_image.as_ref(), Some(codec_specific_info.as_ref()))
        }
    };
    if result.error() != VideoEncoderEncodedImageCallbackResultError::Ok {
        rtc_log_warning!(
            "NVCODEC: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
        );
    }
}

#[derive(Clone, Copy)]
struct DecoderCallbackValue {
    rtp_timestamp: u32,
    render_time_ms: i64,
}

#[derive(Default)]
struct DecoderCallbackState {
    callback: Option<VideoDecoderDecodedImageCallbackPtr>,
}

fn handle_nvcodec_decode_callback(
    callback_state: &Arc<Mutex<DecoderCallbackState>>,
    result: Result<shiguredo_nvcodec::DecodedFrame<DecoderCallbackValue>>,
) {
    let frame = match result {
        Ok(frame) => frame,
        Err(err) => {
            rtc_log_error!("NVCODEC decode callback failed: {}", err);
            return;
        }
    };

    let width_i32 = match i32::try_from(frame.width()) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!(
                "NVCODEC decoded frame width exceeds i32::MAX: {}",
                frame.width()
            );
            return;
        }
    };
    let height_i32 = match i32::try_from(frame.height()) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!(
                "NVCODEC decoded frame height exceeds i32::MAX: {}",
                frame.height()
            );
            return;
        }
    };
    let pitch_i32 = match i32::try_from(frame.y_stride()) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!(
                "NVCODEC decoded frame pitch exceeds i32::MAX: {}",
                frame.y_stride()
            );
            return;
        }
    };

    let mut nv12 = NV12Buffer::new(width_i32, height_i32);
    let dst_stride_y = nv12.stride_y();
    let dst_stride_uv = nv12.stride_uv();
    {
        let (dst_y, dst_uv) = nv12.planes_mut();
        if !nv12_copy(
            frame.y_plane(),
            pitch_i32,
            frame.uv_plane(),
            pitch_i32,
            dst_y,
            dst_stride_y,
            dst_uv,
            dst_stride_uv,
            width_i32,
            height_i32,
        ) {
            rtc_log_error!("NVCODEC decode callback: nv12_copy failed");
            return;
        }
    }

    let value = frame.user_data();
    let decoded_frame = VideoFrame::builder(&nv12.cast_to_video_frame_buffer())
        .set_timestamp_us(value.render_time_ms.saturating_mul(1000))
        .set_rtp_timestamp(value.rtp_timestamp)
        .build();

    let callback_state = callback_state
        .lock()
        .expect("callback_state should not be poisoned");
    let Some(callback) = callback_state.callback else {
        return;
    };
    unsafe {
        callback.decoded(decoded_frame.as_ref());
    }
}

struct NvCodecVideoEncoder {
    encoder: Option<Encoder<FnEncodeHandler<EncoderCallbackValue, Error>>>,
    callback_state: Arc<Mutex<EncoderCallbackState>>,
    codec_type: VideoCodecType,
    device_id: i32,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    rebuild_needed: bool,
    reconfigure_needed: bool,
}

impl NvCodecVideoEncoder {
    fn new(codec_type: VideoCodecType, device_id: i32) -> Self {
        Self {
            encoder: None,
            callback_state: Arc::new(Mutex::new(EncoderCallbackState::default())),
            codec_type,
            device_id,
            width: 0,
            height: 0,
            framerate: 30,
            target_bitrate_bps: 500_000,
            rebuild_needed: false,
            reconfigure_needed: false,
        }
    }

    fn rebuild_encoder(&mut self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(Error::NvCodecMessage {
                reason: "encoder width and height must be set before building encoder".to_string(),
            });
        }
        let Some(codec_config) = encoder_codec_config(self.codec_type) else {
            return Err(Error::NvCodecMessage {
                reason: format!(
                    "unsupported codec type for encoder config: {:?}",
                    self.codec_type
                ),
            });
        };
        let config = EncoderConfig {
            codec: codec_config,
            width: self.width,
            height: self.height,
            max_encode_width: Some(self.width),
            max_encode_height: Some(self.height),
            framerate_num: self.framerate.max(1),
            framerate_den: 1,
            average_bitrate: Some(self.target_bitrate_bps.max(1)),
            preset: Preset::P4,
            tuning_info: TuningInfo::LOW_LATENCY,
            rate_control_mode: RateControlMode::Cbr,
            gop_length: None,
            frame_interval_p: 1,
            buffer_format: BufferFormat::Nv12,
            device_id: self.device_id,
        };

        let callback_state = Arc::clone(&self.callback_state);
        let callback_codec_type = self.codec_type;
        self.encoder = Some(Encoder::new(
            config,
            FnEncodeHandler::new(move |result| {
                handle_nvcodec_encode_callback(&callback_state, result, callback_codec_type);
            }),
        )?);
        self.rebuild_needed = false;
        self.reconfigure_needed = false;
        Ok(())
    }

    fn reconfigure_encoder(&mut self) -> Result<()> {
        let Some(encoder) = self.encoder.as_ref() else {
            return Err(Error::NvCodecMessage {
                reason: "encoder must be initialized before reconfigure".to_string(),
            });
        };
        encoder.reconfigure(nvcodec_reconfigure_params(
            self.target_bitrate_bps,
            self.framerate,
        ))?;
        self.reconfigure_needed = false;
        Ok(())
    }
}

impl VideoEncoderHandler for NvCodecVideoEncoder {
    #[expect(unused_variables)]
    fn init_encode(
        &mut self,
        codec: VideoCodecRef<'_>,
        settings: VideoEncoderSettingsRef<'_>,
    ) -> VideoCodecStatus {
        if codec.codec_type() != self.codec_type {
            return VideoCodecStatus::ErrParameter;
        }
        self.width = codec.width().max(0) as u32;
        self.height = codec.height().max(0) as u32;
        self.framerate = codec.max_framerate().max(1);
        self.target_bitrate_bps = codec.start_bitrate_kbps().saturating_mul(1000).max(1);
        if self.rebuild_encoder().is_err() {
            return VideoCodecStatus::Error;
        }
        VideoCodecStatus::Ok
    }

    fn encode(
        &mut self,
        frame: VideoFrameRef<'_>,
        frame_types: Option<VideoFrameTypeVectorRef<'_>>,
    ) -> VideoCodecStatus {
        let has_callback = {
            let callback_state = self
                .callback_state
                .lock()
                .expect("callback_state should not be poisoned");
            callback_state.callback.is_some()
        };
        if !has_callback {
            return VideoCodecStatus::Uninitialized;
        }

        let frame_width = frame.width().max(0) as u32;
        let frame_height = frame.height().max(0) as u32;
        if frame_width == 0 || frame_height == 0 {
            return VideoCodecStatus::ErrParameter;
        }
        let requested_frame_type = helpers::requested_frame_type(frame_types);
        if matches!(requested_frame_type, Some(VideoFrameType::Empty)) {
            return VideoCodecStatus::NoOutput;
        }
        if self.width != frame_width || self.height != frame_height {
            self.width = frame_width;
            self.height = frame_height;
            // 解像度変更は NVENC を再初期化する
            self.rebuild_needed = true;
        }
        if self.encoder.is_none() {
            self.rebuild_needed = true;
        }

        if self.rebuild_needed {
            if self.rebuild_encoder().is_err() {
                return VideoCodecStatus::Error;
            }
        } else if self.reconfigure_needed {
            // ビットレート更新は再初期化ではなく reconfigure を行う
            if self.reconfigure_encoder().is_err() {
                rtc_log_warning!(
                    "NVCODEC reconfigure failed for {:?}; falling back to rebuild",
                    self.codec_type
                );
                self.rebuild_needed = true;
                if self.rebuild_encoder().is_err() {
                    return VideoCodecStatus::Error;
                }
            }
        }

        let mut frame_buffer = frame.buffer();
        let Some(i420) = frame_buffer.to_i420() else {
            return VideoCodecStatus::Error;
        };
        let frame_width_i32 = match i32::try_from(frame_width) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let frame_height_i32 = match i32::try_from(frame_height) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let mut nv12 = NV12Buffer::new(frame_width_i32, frame_height_i32);
        let src_stride_y = i420.stride_y();
        let src_stride_u = i420.stride_u();
        let src_stride_v = i420.stride_v();
        let dst_stride_y = nv12.stride_y();
        let dst_stride_uv = nv12.stride_uv();
        {
            let (dst_y, dst_uv) = nv12.planes_mut();
            if !i420_to_nv12(
                i420.y_data(),
                src_stride_y,
                i420.u_data(),
                src_stride_u,
                i420.v_data(),
                src_stride_v,
                dst_y,
                dst_stride_y,
                dst_uv,
                dst_stride_uv,
                frame_width_i32,
                frame_height_i32,
            ) {
                return VideoCodecStatus::Error;
            }
        }

        let rtp_timestamp = frame.rtp_timestamp();
        let encoder = self.encoder.as_ref().expect("encoder should exist");
        let force_key_frame = matches!(requested_frame_type, Some(VideoFrameType::Key));
        let encode_options = EncodeOptions {
            force_intra: false,
            force_idr: force_key_frame,
            output_spspps: force_key_frame,
        };
        let callback_value = EncoderCallbackValue {
            rtp_timestamp,
            frame_width,
            frame_height,
        };
        if let Err(err) = encoder.encode(nv12.data(), &encode_options, callback_value) {
            rtc_log_error!("NVCODEC encode failed for {:?}: {}", self.codec_type, err);
            return VideoCodecStatus::Error;
        }

        VideoCodecStatus::Ok
    }

    fn register_encode_complete_callback(
        &mut self,
        callback: Option<VideoEncoderEncodedImageCallbackRef<'_>>,
    ) -> VideoCodecStatus {
        let mut callback_state = self
            .callback_state
            .lock()
            .expect("callback_state should not be poisoned");
        callback_state.callback = callback
            .map(|callback| unsafe { VideoEncoderEncodedImageCallbackPtr::from_ref(callback) });
        VideoCodecStatus::Ok
    }

    fn release(&mut self) -> VideoCodecStatus {
        let mut callback_state = self
            .callback_state
            .lock()
            .expect("callback_state should not be poisoned");
        callback_state.callback = None;
        drop(callback_state);
        self.encoder = None;
        VideoCodecStatus::Ok
    }

    fn set_rates(&mut self, parameters: VideoEncoderRateControlParametersRef<'_>) {
        self.framerate = parameters.framerate_fps().max(1.0) as u32;
        let bitrate = parameters
            .bitrate_sum_bps()
            .max(parameters.target_bitrate_sum_bps());
        self.target_bitrate_bps = bitrate.max(1);
        self.reconfigure_needed = true;
    }

    fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
        let mut info = VideoEncoderEncoderInfo::new();
        info.set_implementation_name("NvCodec");
        info.set_is_hardware_accelerated(true);
        info
    }
}

struct NvCodecVideoDecoder {
    decoder: Option<Decoder<FnDecodeHandler<DecoderCallbackValue, Error>>>,
    callback_state: Arc<Mutex<DecoderCallbackState>>,
    codec_type: VideoCodecType,
    device_id: i32,
}

impl NvCodecVideoDecoder {
    fn new(codec_type: VideoCodecType, device_id: i32) -> Self {
        Self {
            decoder: None,
            callback_state: Arc::new(Mutex::new(DecoderCallbackState::default())),
            codec_type,
            device_id,
        }
    }

    fn decoder_config(&self) -> Option<DecoderConfig> {
        let codec = decoder_codec(self.codec_type)?;
        Some(DecoderConfig {
            codec,
            device_id: self.device_id,
            max_num_decode_surfaces: 20,
            max_display_delay: 0,
            surface_format: SurfaceFormat::Nv12,
        })
    }

    fn rebuild_decoder(&mut self) -> Result<()> {
        let config = self.decoder_config().ok_or_else(|| Error::NvCodecMessage {
            reason: "decoder config is not available".to_string(),
        })?;
        let callback_state = Arc::clone(&self.callback_state);
        self.decoder = Some(Decoder::new(
            config,
            FnDecodeHandler::new(move |result| {
                handle_nvcodec_decode_callback(&callback_state, result);
            }),
        )?);
        Ok(())
    }
}

impl VideoDecoderHandler for NvCodecVideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != self.codec_type {
            return false;
        }
        match self.rebuild_decoder() {
            Ok(()) => true,
            Err(err) => {
                rtc_log_error!("NVCODEC rebuild decoder failed in configure: {}", err);
                false
            }
        }
    }

    fn decode(
        &mut self,
        input_image: EncodedImageRef<'_>,
        render_time_ms: i64,
    ) -> VideoCodecStatus {
        let has_callback = {
            let callback_state = self
                .callback_state
                .lock()
                .expect("callback_state should not be poisoned");
            callback_state.callback.is_some()
        };
        if !has_callback {
            return VideoCodecStatus::Uninitialized;
        }

        let Some(encoded_data) = input_image.encoded_data() else {
            return VideoCodecStatus::ErrParameter;
        };

        if self.decoder.is_none()
            && let Err(err) = self.rebuild_decoder()
        {
            rtc_log_error!("NVCODEC rebuild decoder failed in decode: {}", err);
            return VideoCodecStatus::Error;
        }

        let callback_value = DecoderCallbackValue {
            rtp_timestamp: input_image.rtp_timestamp(),
            render_time_ms,
        };
        let decoder = self.decoder.as_ref().expect("decoder should exist");
        if let Err(err) = decoder.decode(encoded_data.data(), callback_value) {
            rtc_log_error!("NVCODEC decode failed for {:?}: {}", self.codec_type, err);
            return VideoCodecStatus::Error;
        }
        VideoCodecStatus::Ok
    }

    fn register_decode_complete_callback(
        &mut self,
        callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    ) -> VideoCodecStatus {
        let mut callback_state = self
            .callback_state
            .lock()
            .expect("callback_state should not be poisoned");
        callback_state.callback = callback;
        VideoCodecStatus::Ok
    }

    fn release(&mut self) -> VideoCodecStatus {
        let mut callback_state = self
            .callback_state
            .lock()
            .expect("callback_state should not be poisoned");
        callback_state.callback = None;
        drop(callback_state);
        self.decoder = None;
        VideoCodecStatus::Ok
    }

    fn get_decoder_info(&mut self) -> VideoDecoderDecoderInfo {
        let mut info = VideoDecoderDecoderInfo::new();
        info.set_implementation_name("NvCodec");
        info.set_is_hardware_accelerated(true);
        info
    }
}

/// NVIDIA NVENC/NVDEC を使用する [VideoCodecCapability]。
///
/// `#[cfg(feature = "nvcodec")]` でのみ利用可能。
/// エンコードは H.264 / H.265 / AV1、
/// デコードは H.264 / H.265 / AV1 / VP8 / VP9 をサポートする。
pub struct NvCodecVideoCodecCapability {
    device_id: i32,
    encoder_supported_formats: Vec<SdpVideoFormat>,
    decoder_supported_formats: Vec<SdpVideoFormat>,
    simulcast_capability_helper: SimulcastCapabilityHelper,
}

impl NvCodecVideoCodecCapability {
    /// デフォルトデバイス (device_id=0) で新しい `NvCodecVideoCodecCapability` を生成する。
    pub fn new() -> Result<Self> {
        Self::new_with_device_id(0)
    }

    /// 指定されたデバイス ID で新しい `NvCodecVideoCodecCapability` を生成する。
    pub fn new_with_device_id(device_id: i32) -> Result<Self> {
        let (encoder_supported_formats, decoder_supported_formats) =
            collect_supported_formats(device_id)?;
        if encoder_supported_formats.is_empty() && decoder_supported_formats.is_empty() {
            return Err(Error::NvCodecMessage {
                reason: "No supported codecs".to_string(),
            });
        }
        Self::new_with_formats_and_device_id(
            encoder_supported_formats,
            decoder_supported_formats,
            device_id,
        )
    }

    fn new_with_formats_and_device_id(
        encoder_supported_formats: Vec<SdpVideoFormat>,
        decoder_supported_formats: Vec<SdpVideoFormat>,
        device_id: i32,
    ) -> Result<Self> {
        let encoder_supported_formats_for_factory = encoder_supported_formats.clone();

        let simulcast_capability_helper = SimulcastCapabilityHelper::new_with_builder(
            move || encoder_supported_formats_for_factory.clone(),
            {
                move |_env, format| {
                    let codec_type = codec_type_from_format(&format)?;
                    Some(VideoEncoder::new_with_handler(Box::new(
                        NvCodecVideoEncoder::new(codec_type, device_id),
                    )))
                }
            },
        );

        Ok(Self {
            device_id,
            encoder_supported_formats,
            decoder_supported_formats,
            simulcast_capability_helper,
        })
    }
}

impl VideoCodecCapability for NvCodecVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC")
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        match direction {
            CodecDirection::Encoder => self.encoder_supported_formats.clone(),
            CodecDirection::Decoder => self.decoder_supported_formats.clone(),
        }
    }

    fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        self.simulcast_capability_helper
            .create_video_encoder(env, format)
    }

    fn create_video_decoder(
        &self,
        _env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        let codec_type = codec_type_from_format(&format)?;
        Some(VideoDecoder::new_with_handler(Box::new(
            NvCodecVideoDecoder::new(codec_type, self.device_id),
        )))
    }
}

#[cfg(test)]
impl NvCodecVideoCodecCapability {
    fn new_for_test(
        encoder_supported_formats: Vec<SdpVideoFormat>,
        decoder_supported_formats: Vec<SdpVideoFormat>,
    ) -> Result<Self> {
        Self::new_with_formats_and_device_id(
            encoder_supported_formats,
            decoder_supported_formats,
            0,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use shiguredo_webrtc::{Environment, SdpVideoFormat};

    fn test_supported_formats(codec_types: &[VideoCodecType]) -> Vec<SdpVideoFormat> {
        let mut supported_formats = Vec::new();
        for codec_type in codec_types {
            supported_formats.extend(helpers::supported_formats_for_codec(*codec_type));
        }
        supported_formats
    }

    #[test]
    fn nvcodec_capability_has_expected_implementation_name() {
        let capability = NvCodecVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
        )
        .expect("テスト用の NvCodecVideoCodecCapability の生成に失敗しました");
        assert_eq!(capability.get_implementation().name(), "nvcodec");
    }

    #[test]
    fn nvcodec_reconfigure_params_uses_rate_values() {
        let params = nvcodec_reconfigure_params(1_234_567, 24);
        assert_eq!(params.width, None);
        assert_eq!(params.height, None);
        assert_eq!(params.framerate_num, Some(24));
        assert_eq!(params.framerate_den, Some(1));
        assert_eq!(params.average_bitrate, Some(1_234_567));
        assert_eq!(params.max_bitrate, None);
    }

    #[test]
    fn nvcodec_reconfigure_params_clamps_zero_to_one() {
        let params = nvcodec_reconfigure_params(0, 0);
        assert_eq!(params.framerate_num, Some(1));
        assert_eq!(params.framerate_den, Some(1));
        assert_eq!(params.average_bitrate, Some(1));
    }

    #[test]
    fn nvcodec_capability_accepts_device_id_configuration() {
        let capability = NvCodecVideoCodecCapability::new_with_formats_and_device_id(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
            7,
        )
        .expect("テスト用の NvCodecVideoCodecCapability の生成に失敗しました");
        assert_eq!(capability.device_id, 7);
    }

    #[test]
    fn nvcodec_capability_supports_formats_per_direction() {
        let capability = NvCodecVideoCodecCapability::new_for_test(
            test_supported_formats(&[
                VideoCodecType::H264,
                VideoCodecType::H265,
                VideoCodecType::Av1,
            ]),
            test_supported_formats(&[
                VideoCodecType::H264,
                VideoCodecType::H265,
                VideoCodecType::Av1,
                VideoCodecType::Vp8,
                VideoCodecType::Vp9,
            ]),
        )
        .expect("テスト用の NvCodecVideoCodecCapability の生成に失敗しました");

        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H265));
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::Av1));
        assert!(!capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp8));
        assert!(!capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp9));

        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H265));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::Av1));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::Vp8));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::Vp9));

        let encoder_formats = capability
            .get_supported_formats(CodecDirection::Encoder)
            .into_iter()
            .map(|format| format.name().expect("format name の取得に失敗"))
            .collect::<Vec<_>>();
        assert_eq!(encoder_formats, vec!["H264", "H265", "AV1"]);

        let decoder_formats = capability
            .get_supported_formats(CodecDirection::Decoder)
            .into_iter()
            .map(|format| format.name().expect("format name の取得に失敗"))
            .collect::<Vec<_>>();
        assert_eq!(decoder_formats, vec!["H264", "H265", "AV1", "VP8", "VP9"]);

        let resolved = capability
            .resolve_sdp_format(
                CodecDirection::Encoder,
                SdpVideoFormat::new("H264").as_ref(),
            )
            .expect("h264 フォーマットが解決されるはずです");
        let params = resolved
            .to_owned()
            .parameters_mut()
            .iter()
            .collect::<HashMap<String, String>>();
        assert_eq!(
            params.get("packetization-mode").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            params.get("level-asymmetry-allowed").map(String::as_str),
            Some("1")
        );

        let resolved_with_packetization_mode_0 = capability.resolve_sdp_format(
            CodecDirection::Encoder,
            SdpVideoFormat::new_with_parameters(
                "H264",
                &HashMap::from([(String::from("packetization-mode"), String::from("0"))]),
                &[],
            )
            .as_ref(),
        );
        assert!(resolved_with_packetization_mode_0.is_some());
    }

    #[test]
    fn nvcodec_simulcast_adapter_encoder_info_contains_adapter_name() {
        let capability = NvCodecVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
        )
        .expect("テスト用の NvCodecVideoCodecCapability の生成に失敗しました");
        let env = Environment::new();
        let format = SdpVideoFormat::new("H264");
        let encoder = capability
            .create_video_encoder(env.as_ref(), format.as_ref())
            .expect("サポートされているフォーマットでエンコーダーが生成されなければなりません");
        let info = encoder.get_encoder_info();
        let implementation_name = info
            .implementation_name()
            .expect("implementation_name の取得に失敗");
        assert!(
            implementation_name.contains("SimulcastEncoderAdapter"),
            "adapter encoder では SimulcastEncoderAdapter を含む実装名が必要: {implementation_name}",
        );
    }
}
