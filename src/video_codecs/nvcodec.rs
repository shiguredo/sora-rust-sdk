use std::collections::HashMap;

use shiguredo_nvcodec::{
    Av1EncoderConfig, BufferFormat, CodecConfig, Decoder, DecoderCodec, DecoderConfig,
    EncodeOptions, Encoder, EncoderConfig, H264EncoderConfig, HevcEncoderConfig, PictureType,
    Preset, RateControlMode, SurfaceFormat, TuningInfo, VideoCodecType as NvCodecType,
    supported_codecs,
};
use shiguredo_webrtc::{
    CodecSpecificInfo, EncodedImage, EncodedImageBuffer, EncodedImageRef, EnvironmentRef,
    H264PacketizationMode, I420Buffer, NV12Buffer, ScalabilityMode, SdpVideoFormat,
    SdpVideoFormatRef, VideoCodecRef, VideoCodecStatus, VideoCodecType, VideoDecoder,
    VideoDecoderDecodedImageCallbackPtr, VideoDecoderDecoderInfo, VideoDecoderHandler,
    VideoDecoderSettingsRef, VideoEncoder, VideoEncoderEncodedImageCallbackPtr,
    VideoEncoderEncodedImageCallbackRef, VideoEncoderEncodedImageCallbackResultError,
    VideoEncoderEncoderInfo, VideoEncoderHandler, VideoEncoderRateControlParametersRef,
    VideoEncoderSettingsRef, VideoFrame, VideoFrameRef, VideoFrameType, VideoFrameTypeVectorRef,
    i420_to_nv12, nv12_to_i420,
};

use crate::video_codec::SimulcastCapabilityHelper;
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CodecAvailability {
    codec_type: VideoCodecType,
    encoder_supported: bool,
    decoder_supported: bool,
}

impl CodecAvailability {
    fn is_supported(&self, direction: CodecDirection) -> bool {
        match direction {
            CodecDirection::Encoder => self.encoder_supported,
            CodecDirection::Decoder => self.decoder_supported,
        }
    }
}

fn codec_sort_key(codec_type: VideoCodecType) -> u8 {
    match codec_type {
        VideoCodecType::H264 => 0,
        VideoCodecType::H265 => 1,
        VideoCodecType::Av1 => 2,
        VideoCodecType::Vp8 => 3,
        VideoCodecType::Vp9 => 4,
        _ => u8::MAX,
    }
}

fn codec_type_from_format(format: &SdpVideoFormatRef<'_>) -> Option<VideoCodecType> {
    let format_name = format.name().ok()?;
    VideoCodecType::try_from(format_name.as_str()).ok()
}

fn supported_formats_for_codec(codec_type: VideoCodecType) -> Vec<SdpVideoFormat> {
    match codec_type {
        VideoCodecType::H264 => vec![SdpVideoFormat::new_with_parameters(
            "H264",
            &HashMap::from([
                (String::from("level-asymmetry-allowed"), String::from("1")),
                (String::from("packetization-mode"), String::from("1")),
            ]),
            &[ScalabilityMode::L1T1],
        )],
        VideoCodecType::H265 => vec![SdpVideoFormat::new("H265")],
        VideoCodecType::Av1 => vec![SdpVideoFormat::new("AV1")],
        VideoCodecType::Vp8 => vec![SdpVideoFormat::new("VP8")],
        VideoCodecType::Vp9 => vec![SdpVideoFormat::new("VP9")],
        _ => Vec::new(),
    }
}

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

fn collect_codec_availability(device_id: i32) -> Vec<CodecAvailability> {
    let Ok(codec_infos) = supported_codecs(device_id) else {
        return Vec::new();
    };

    let mut codecs = Vec::new();
    for info in codec_infos {
        let codec_type = match info.codec {
            NvCodecType::H264 => VideoCodecType::H264,
            NvCodecType::Hevc => VideoCodecType::H265,
            NvCodecType::Av1 => VideoCodecType::Av1,
            NvCodecType::Vp8 => VideoCodecType::Vp8,
            NvCodecType::Vp9 => VideoCodecType::Vp9,
            NvCodecType::Jpeg => continue,
        };
        let encoder_supported = info.encoding.supported;
        let decoder_supported = info.decoding.supported;
        if encoder_supported || decoder_supported {
            codecs.push(CodecAvailability {
                codec_type,
                encoder_supported,
                decoder_supported,
            });
        }
    }
    codecs.sort_by_key(|codec| codec_sort_key(codec.codec_type));
    codecs
}

struct NvCodecVideoEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
    encoder: Option<Encoder>,
    codec_type: VideoCodecType,
    device_id: i32,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    reconfigure_needed: bool,
}

impl NvCodecVideoEncoder {
    fn new(codec_type: VideoCodecType, device_id: i32) -> Self {
        Self {
            callback: None,
            encoder: None,
            codec_type,
            device_id,
            width: 0,
            height: 0,
            framerate: 30,
            target_bitrate_bps: 500_000,
            reconfigure_needed: false,
        }
    }

    fn rebuild_encoder(&mut self) -> Result<(), ()> {
        if self.width == 0 || self.height == 0 {
            return Err(());
        }
        let Some(codec_config) = encoder_codec_config(self.codec_type) else {
            return Err(());
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
        self.encoder = Encoder::new(config).ok();
        self.reconfigure_needed = false;
        self.encoder.as_ref().map(|_| ()).ok_or(())
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

    #[expect(unused_variables)]
    fn encode(
        &mut self,
        frame: VideoFrameRef<'_>,
        frame_types: Option<VideoFrameTypeVectorRef<'_>>,
    ) -> VideoCodecStatus {
        let callback = match self.callback {
            Some(callback) => callback,
            None => return VideoCodecStatus::Uninitialized,
        };

        let frame_width = frame.width().max(0) as u32;
        let frame_height = frame.height().max(0) as u32;
        if frame_width == 0 || frame_height == 0 {
            return VideoCodecStatus::ErrParameter;
        }
        if self.width != frame_width || self.height != frame_height {
            self.width = frame_width;
            self.height = frame_height;
            self.reconfigure_needed = true;
        }
        if (self.reconfigure_needed || self.encoder.is_none()) && self.rebuild_encoder().is_err() {
            return VideoCodecStatus::Error;
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
        let encoder = self.encoder.as_mut().expect("encoder should exist");
        let encode_options = EncodeOptions {
            force_intra: false,
            force_idr: false,
            output_spspps: false,
        };
        if encoder.encode(nv12.data(), &encode_options).is_err() {
            return VideoCodecStatus::Error;
        }

        let mut has_output = false;
        while let Some(encoded_frame) = encoder.next_frame() {
            has_output = true;
            let mut encoded_image = EncodedImage::new();
            let encoded_buffer = EncodedImageBuffer::from_bytes(encoded_frame.data());
            encoded_image.set_encoded_data(&encoded_buffer);
            encoded_image.set_rtp_timestamp(rtp_timestamp);
            encoded_image.set_encoded_width(frame_width);
            encoded_image.set_encoded_height(frame_height);
            encoded_image.set_frame_type(match encoded_frame.picture_type() {
                PictureType::Idr | PictureType::I => VideoFrameType::Key,
                _ => VideoFrameType::Delta,
            });

            let mut codec_specific_info = CodecSpecificInfo::new();
            codec_specific_info.set_codec_type(self.codec_type);
            if self.codec_type == VideoCodecType::H264 {
                codec_specific_info
                    .set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
                codec_specific_info
                    .set_h264_idr_frame(matches!(encoded_frame.picture_type(), PictureType::Idr));
            }

            let result = unsafe {
                callback
                    .on_encoded_image(encoded_image.as_ref(), Some(codec_specific_info.as_ref()))
            };
            if result.error() != VideoEncoderEncodedImageCallbackResultError::Ok {
                return VideoCodecStatus::Error;
            }
        }

        if has_output {
            VideoCodecStatus::Ok
        } else {
            VideoCodecStatus::NoOutput
        }
    }

    fn register_encode_complete_callback(
        &mut self,
        callback: Option<VideoEncoderEncodedImageCallbackRef<'_>>,
    ) -> VideoCodecStatus {
        self.callback = callback
            .map(|callback| unsafe { VideoEncoderEncodedImageCallbackPtr::from_ref(callback) });
        VideoCodecStatus::Ok
    }

    fn release(&mut self) -> VideoCodecStatus {
        self.encoder = None;
        self.callback = None;
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
    callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    decoder: Option<Decoder>,
    codec_type: VideoCodecType,
    device_id: i32,
}

impl NvCodecVideoDecoder {
    fn new(codec_type: VideoCodecType, device_id: i32) -> Self {
        Self {
            callback: None,
            decoder: None,
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

    fn ensure_decoder(&mut self) -> Result<(), ()> {
        if self.decoder.is_none() {
            let Some(config) = self.decoder_config() else {
                return Err(());
            };
            self.decoder = Decoder::new(config).ok();
        }
        self.decoder.as_ref().map(|_| ()).ok_or(())
    }
}

impl VideoDecoderHandler for NvCodecVideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != self.codec_type {
            return false;
        }
        let Some(config) = self.decoder_config() else {
            return false;
        };
        self.decoder = Decoder::new(config).ok();
        self.decoder.is_some()
    }

    fn decode(
        &mut self,
        input_image: EncodedImageRef<'_>,
        render_time_ms: i64,
    ) -> VideoCodecStatus {
        if self.ensure_decoder().is_err() {
            return VideoCodecStatus::Error;
        }
        let Some(encoded_data) = input_image.encoded_data() else {
            return VideoCodecStatus::ErrParameter;
        };

        let rtp_timestamp = input_image.rtp_timestamp();
        let mut decoded_images = Vec::new();
        let decoder = self.decoder.as_mut().expect("decoder should exist");
        if decoder.decode(encoded_data.data()).is_err() {
            return VideoCodecStatus::Error;
        }

        loop {
            let frame = match decoder.next_frame() {
                Ok(v) => v,
                Err(_) => return VideoCodecStatus::Error,
            };
            let Some(frame) = frame else {
                break;
            };

            let mut i420 = I420Buffer::new(frame.width() as i32, frame.height() as i32);
            let dst_stride_y = i420.stride_y();
            let dst_stride_u = i420.stride_u();
            let dst_stride_v = i420.stride_v();
            let (dst_y, dst_u, dst_v) = i420.planes_mut();
            if !nv12_to_i420(
                frame.y_plane(),
                frame.y_stride() as i32,
                frame.uv_plane(),
                frame.uv_stride() as i32,
                dst_y,
                dst_stride_y,
                dst_u,
                dst_stride_u,
                dst_v,
                dst_stride_v,
                frame.width() as i32,
                frame.height() as i32,
            ) {
                return VideoCodecStatus::Error;
            }

            decoded_images.push(
                VideoFrame::builder(&i420.cast_to_video_frame_buffer())
                    .set_timestamp_us(render_time_ms.saturating_mul(1000))
                    .set_rtp_timestamp(rtp_timestamp)
                    .build(),
            );
        }
        let Some(callback) = self.callback.as_ref() else {
            return VideoCodecStatus::Uninitialized;
        };
        for decoded_image in &decoded_images {
            unsafe {
                callback.decoded(decoded_image.as_ref());
            }
        }

        if !decoded_images.is_empty() {
            VideoCodecStatus::Ok
        } else {
            VideoCodecStatus::NoOutput
        }
    }

    fn register_decode_complete_callback(
        &mut self,
        callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    ) -> VideoCodecStatus {
        self.callback = callback;
        VideoCodecStatus::Ok
    }

    fn release(&mut self) -> VideoCodecStatus {
        self.decoder = None;
        self.callback = None;
        VideoCodecStatus::Ok
    }

    fn get_decoder_info(&mut self) -> VideoDecoderDecoderInfo {
        let mut info = VideoDecoderDecoderInfo::new();
        info.set_implementation_name("NvCodec");
        info.set_is_hardware_accelerated(true);
        info
    }
}

pub struct NvCodecVideoCodecCapability {
    device_id: i32,
    codecs: Vec<CodecAvailability>,
    simulcast_capability_helper: SimulcastCapabilityHelper,
}

impl NvCodecVideoCodecCapability {
    pub fn new() -> Self {
        Self::new_with_device_id(0)
    }

    pub fn new_with_device_id(device_id: i32) -> Self {
        Self::new_with_codecs_and_device_id(collect_codec_availability(device_id), device_id)
    }

    fn new_with_codecs_and_device_id(codecs: Vec<CodecAvailability>, device_id: i32) -> Self {
        let mut codecs = codecs;
        codecs.sort_by_key(|codec| codec_sort_key(codec.codec_type));

        let mut encoder_codec_types = codecs
            .iter()
            .filter(|codec| codec.encoder_supported)
            .map(|codec| codec.codec_type)
            .collect::<Vec<_>>();
        encoder_codec_types.sort_by_key(|codec_type| codec_sort_key(*codec_type));

        let simulcast_capability_helper = SimulcastCapabilityHelper::new_with_builder(
            {
                let encoder_codec_types = encoder_codec_types.clone();
                move || {
                    let mut formats = Vec::new();
                    for codec_type in &encoder_codec_types {
                        formats.extend(supported_formats_for_codec(*codec_type));
                    }
                    formats
                }
            },
            {
                let encoder_codec_types = encoder_codec_types.clone();
                move |_env, format| {
                    let codec_type = codec_type_from_format(&format)?;
                    if !encoder_codec_types.contains(&codec_type) {
                        return None;
                    }
                    Some(VideoEncoder::new_with_handler(Box::new(
                        NvCodecVideoEncoder::new(codec_type, device_id),
                    )))
                }
            },
        );

        Self {
            device_id,
            codecs,
            simulcast_capability_helper,
        }
    }

    fn is_codec_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
        self.codecs
            .iter()
            .any(|codec| codec.codec_type == codec_type && codec.is_supported(direction))
    }
}

impl Default for NvCodecVideoCodecCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoCodecCapability for NvCodecVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("nvcodec", "NVIDIA NVENC/NVDEC")
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        let mut formats = Vec::new();
        for codec in &self.codecs {
            if !codec.is_supported(direction) {
                continue;
            }
            formats.extend(supported_formats_for_codec(codec.codec_type));
        }
        formats
    }

    fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        let codec_type = codec_type_from_format(&format)?;
        if !self.is_codec_supported(CodecDirection::Encoder, codec_type) {
            return None;
        }
        self.simulcast_capability_helper
            .create_video_encoder(env, format)
    }

    fn create_video_decoder(
        &self,
        _env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        let codec_type = codec_type_from_format(&format)?;
        if !self.is_codec_supported(CodecDirection::Decoder, codec_type) {
            return None;
        }
        Some(VideoDecoder::new_with_handler(Box::new(
            NvCodecVideoDecoder::new(codec_type, self.device_id),
        )))
    }
}

#[cfg(test)]
impl NvCodecVideoCodecCapability {
    fn new_for_test(codecs: Vec<CodecAvailability>) -> Self {
        Self::new_with_codecs_and_device_id(codecs, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::{Environment, SdpVideoFormat};

    fn test_codec(
        codec_type: VideoCodecType,
        encoder_supported: bool,
        decoder_supported: bool,
    ) -> CodecAvailability {
        CodecAvailability {
            codec_type,
            encoder_supported,
            decoder_supported,
        }
    }

    #[test]
    fn nvcodec_capability_has_expected_implementation_name() {
        let capability = NvCodecVideoCodecCapability::new_for_test(vec![test_codec(
            VideoCodecType::H264,
            true,
            true,
        )]);
        assert_eq!(capability.get_implementation().name(), "nvcodec");
    }

    #[test]
    fn nvcodec_capability_accepts_device_id_configuration() {
        let capability = NvCodecVideoCodecCapability::new_with_codecs_and_device_id(
            vec![test_codec(VideoCodecType::H264, true, true)],
            7,
        );
        assert_eq!(capability.device_id, 7);
    }

    #[test]
    fn nvcodec_capability_supports_formats_per_direction() {
        let capability = NvCodecVideoCodecCapability::new_for_test(vec![
            test_codec(VideoCodecType::H264, true, true),
            test_codec(VideoCodecType::H265, true, true),
            test_codec(VideoCodecType::Av1, true, true),
            test_codec(VideoCodecType::Vp8, false, true),
            test_codec(VideoCodecType::Vp9, false, true),
        ]);

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
            .expect("h264 format should be resolved");
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
    fn nvcodec_capability_rejects_unsupported_encoder_creation() {
        let capability = NvCodecVideoCodecCapability::new_for_test(vec![test_codec(
            VideoCodecType::Vp8,
            false,
            true,
        )]);

        let env = Environment::new();
        assert!(
            capability
                .create_video_encoder(env.as_ref(), SdpVideoFormat::new("VP8").as_ref())
                .is_none()
        );
    }

    #[test]
    fn nvcodec_capability_rejects_unsupported_decoder_creation() {
        let capability = NvCodecVideoCodecCapability::new_for_test(vec![test_codec(
            VideoCodecType::H264,
            true,
            false,
        )]);

        let env = Environment::new();
        assert!(
            capability
                .create_video_decoder(env.as_ref(), SdpVideoFormat::new("H264").as_ref())
                .is_none()
        );
    }

    #[test]
    fn nvcodec_simulcast_adapter_encoder_info_contains_adapter_name() {
        let capability = NvCodecVideoCodecCapability::new_for_test(vec![test_codec(
            VideoCodecType::H264,
            true,
            true,
        )]);
        let env = Environment::new();
        let format = SdpVideoFormat::new("H264");
        let encoder = capability
            .create_video_encoder(env.as_ref(), format.as_ref())
            .expect("encoder must be created for supported format");
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
