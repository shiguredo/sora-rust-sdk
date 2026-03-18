use std::collections::HashMap;

use shiguredo_nvcodec::{
    Decoder, DecoderConfig, Encoder, EncoderConfig, PictureType, RateControlMode,
};
use shiguredo_webrtc::{
    CodecSpecificInfo, EncodedImage, EncodedImageBuffer, EncodedImageRef, H264PacketizationMode,
    SdpVideoFormat, VideoCodecRef, VideoCodecStatus, VideoCodecType,
    VideoDecoderDecodedImageCallbackPtr, VideoDecoderDecoderInfo, VideoDecoderHandler,
    VideoDecoderSettingsRef, VideoEncoderEncodedImageCallbackPtr,
    VideoEncoderEncodedImageCallbackRef, VideoEncoderEncodedImageCallbackResultError,
    VideoEncoderEncoderInfo, VideoEncoderHandler, VideoEncoderRateControlParametersRef,
    VideoEncoderSettingsRef, VideoFrame, VideoFrameRef, VideoFrameType, VideoFrameTypeVectorRef,
    i420_to_nv12, nv12_to_i420,
};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

struct NvCodecVideoEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
    encoder: Option<Encoder>,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    reconfigure_needed: bool,
}

impl NvCodecVideoEncoder {
    fn new() -> Self {
        Self {
            callback: None,
            encoder: None,
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
        let mut config = EncoderConfig::default();
        config.width = self.width;
        config.height = self.height;
        config.max_encode_width = Some(self.width);
        config.max_encode_height = Some(self.height);
        config.fps_numerator = self.framerate.max(1);
        config.fps_denominator = 1;
        config.target_bitrate = Some(self.target_bitrate_bps.max(1));
        config.rate_control_mode = RateControlMode::Cbr;
        self.encoder = Encoder::new_h264(config).ok();
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
        if codec.codec_type() != VideoCodecType::H264 {
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
        if self.reconfigure_needed || self.encoder.is_none() {
            if self.rebuild_encoder().is_err() {
                return VideoCodecStatus::Error;
            }
        }

        let i420 = frame.buffer();
        let nv12 = match i420_to_nv12(&i420) {
            Some(v) => v,
            None => return VideoCodecStatus::Error,
        };

        let rtp_timestamp = frame.rtp_timestamp();
        let encoder = self.encoder.as_mut().expect("encoder should exist");
        if encoder.encode(&nv12).is_err() {
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
            codec_specific_info.set_codec_type(VideoCodecType::H264);
            codec_specific_info.set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
            codec_specific_info
                .set_h264_idr_frame(matches!(encoded_frame.picture_type(), PictureType::Idr));

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
}

impl NvCodecVideoDecoder {
    fn new() -> Self {
        Self {
            callback: None,
            decoder: None,
        }
    }

    fn ensure_decoder(&mut self) -> Result<(), ()> {
        if self.decoder.is_none() {
            self.decoder = Decoder::new_h264(DecoderConfig::default()).ok();
        }
        self.decoder.as_ref().map(|_| ()).ok_or(())
    }
}

impl VideoDecoderHandler for NvCodecVideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != VideoCodecType::H264 {
            return false;
        }
        self.decoder = Decoder::new_h264(DecoderConfig::default()).ok();
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

            let Some(i420) = nv12_to_i420(
                frame.y_plane(),
                frame.y_stride() as i32,
                frame.uv_plane(),
                frame.uv_stride() as i32,
                frame.width() as i32,
                frame.height() as i32,
            ) else {
                return VideoCodecStatus::Error;
            };

            decoded_images.push(VideoFrame::from_i420(
                &i420,
                render_time_ms.saturating_mul(1000),
                rtp_timestamp,
            ));
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

pub struct NvCodecVideoCodecCapability;

impl NvCodecVideoCodecCapability {
    pub fn new() -> Self {
        Self
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

    fn is_supported(&self, _direction: CodecDirection, codec_type: VideoCodecType) -> bool {
        codec_type == VideoCodecType::H264
    }

    fn resolve_sdp_format(
        &self,
        _direction: CodecDirection,
        codec_type: VideoCodecType,
        parameters: &HashMap<String, String>,
        _scalability_mode: Option<&str>,
    ) -> Option<SdpVideoFormat> {
        if codec_type != VideoCodecType::H264 {
            return None;
        }

        let mut h264_params = HashMap::from([
            (String::from("level-asymmetry-allowed"), String::from("1")),
            (String::from("packetization-mode"), String::from("1")),
        ]);
        if let Some(profile_level_id) = parameters.get("profile-level-id") {
            h264_params.insert(String::from("profile-level-id"), profile_level_id.clone());
        }

        Some(SdpVideoFormat::new_with_parameters(
            "H264",
            &h264_params,
            &[],
        ))
    }

    fn create_video_encoder(
        &self,
        format: &SdpVideoFormat,
    ) -> Option<Box<dyn VideoEncoderHandler>> {
        if format.name().ok().as_deref() == Some("H264") {
            Some(Box::new(NvCodecVideoEncoder::new()))
        } else {
            None
        }
    }

    fn create_video_decoder(
        &self,
        format: &SdpVideoFormat,
    ) -> Option<Box<dyn VideoDecoderHandler>> {
        if format.name().ok().as_deref() == Some("H264") {
            Some(Box::new(NvCodecVideoDecoder::new()))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvcodec_capability_supports_only_h264() {
        let capability = NvCodecVideoCodecCapability::new();

        assert_eq!(capability.get_implementation().name(), "nvcodec");
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(!capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp9));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));
        assert!(!capability.is_supported(CodecDirection::Decoder, VideoCodecType::Av1));

        assert!(
            capability
                .create_video_encoder(&SdpVideoFormat::new("H264"))
                .is_some()
        );
        assert!(
            capability
                .create_video_encoder(&SdpVideoFormat::new("H265"))
                .is_none()
        );
        assert!(
            capability
                .create_video_decoder(&SdpVideoFormat::new("H264"))
                .is_some()
        );
        assert!(
            capability
                .create_video_decoder(&SdpVideoFormat::new("H265"))
                .is_none()
        );

        let resolved = capability
            .resolve_sdp_format(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                &HashMap::new(),
                None,
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
            VideoCodecType::H264,
            &HashMap::from([(String::from("packetization-mode"), String::from("0"))]),
            None,
        );
        assert!(resolved_with_packetization_mode_0.is_some());
    }
}
