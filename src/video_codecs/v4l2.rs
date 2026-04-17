use std::collections::HashMap;
use std::fs::OpenOptions;

use shiguredo_v4l2::v4l2_m2m::{
    DecodeOutput, DecoderConfig, EncoderConfig, H264Decoder, H264Encoder, InputFrame, Resolution,
};
use shiguredo_webrtc::{
    CodecSpecificInfo, EncodedImage, EncodedImageBuffer, EncodedImageRef, EnvironmentRef,
    H264PacketizationMode, I420Buffer, ScalabilityMode, SdpVideoFormat, SdpVideoFormatRef,
    VideoCodecRef, VideoCodecStatus, VideoCodecType, VideoDecoder,
    VideoDecoderDecodedImageCallbackPtr, VideoDecoderDecoderInfo, VideoDecoderHandler,
    VideoDecoderSettingsRef, VideoEncoder, VideoEncoderEncodedImageCallbackPtr,
    VideoEncoderEncodedImageCallbackRef, VideoEncoderEncodedImageCallbackResultError,
    VideoEncoderEncoderInfo, VideoEncoderHandler, VideoEncoderRateControlParametersRef,
    VideoEncoderSettingsRef, VideoFrame, VideoFrameRef, VideoFrameType, VideoFrameTypeVectorRef,
    i420_copy, rtc_log_error, rtc_log_warning,
};

use crate::error::{Error, Result};
use crate::video_codec::{SimulcastCapabilityHelper, codec_type_from_format};
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

fn v4l2_supported_formats() -> Vec<SdpVideoFormat> {
    vec![SdpVideoFormat::new_with_parameters(
        "H264",
        &HashMap::from([
            (String::from("level-asymmetry-allowed"), String::from("1")),
            (String::from("packetization-mode"), String::from("1")),
        ]),
        &[ScalabilityMode::L1T1],
    )]
}

fn requested_frame_type(
    frame_types: Option<VideoFrameTypeVectorRef<'_>>,
) -> Option<VideoFrameType> {
    frame_types.and_then(|frame_types| frame_types.get(0))
}

fn build_i420_frame(
    data: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    timestamp_us: i64,
    rtp_timestamp: u32,
) -> Option<VideoFrame> {
    let chroma_stride = stride.div_ceil(2);
    let chroma_height = height.div_ceil(2);

    let y_size = stride.checked_mul(height)? as usize;
    let uv_size = chroma_stride.checked_mul(chroma_height)? as usize;
    let total_size = y_size.checked_add(uv_size.checked_mul(2)?)?;
    if data.len() < total_size {
        return None;
    }

    let src_y = &data[..y_size];
    let src_u = &data[y_size..y_size + uv_size];
    let src_v = &data[y_size + uv_size..y_size + uv_size * 2];

    let src_stride_y = i32::try_from(stride).ok()?;
    let src_stride_u = i32::try_from(chroma_stride).ok()?;
    let src_stride_v = i32::try_from(chroma_stride).ok()?;

    let mut i420 = I420Buffer::new(width as i32, height as i32);
    let dst_stride_y = i420.stride_y();
    let dst_stride_u = i420.stride_u();
    let dst_stride_v = i420.stride_v();
    let (dst_y, dst_u, dst_v) = i420.planes_mut();

    if !i420_copy(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width as i32,
        height as i32,
    ) {
        return None;
    }

    Some(
        VideoFrame::builder(&i420.cast_to_video_frame_buffer())
            .set_timestamp_us(timestamp_us)
            .set_rtp_timestamp(rtp_timestamp)
            .build(),
    )
}

struct V4l2VideoEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
    encoder: Option<H264Encoder>,
    device_path: String,
    width: u32,
    height: u32,
    target_bitrate_bps: u32,
    rebuild_needed: bool,
}

impl V4l2VideoEncoder {
    fn new(device_path: String) -> Self {
        Self {
            callback: None,
            encoder: None,
            device_path,
            width: 0,
            height: 0,
            target_bitrate_bps: 500_000,
            rebuild_needed: false,
        }
    }

    fn rebuild_encoder(&mut self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(Error::V4l2Message {
                reason: "V4L2 encoder requires non-zero width and height".to_string(),
            });
        }

        let mut config =
            EncoderConfig::new(self.width, self.height, self.target_bitrate_bps.max(1));
        config.device_path = self.device_path.clone();
        self.encoder = Some(H264Encoder::new(config)?);
        self.rebuild_needed = false;
        Ok(())
    }
}

impl VideoEncoderHandler for V4l2VideoEncoder {
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
        self.target_bitrate_bps = codec.start_bitrate_kbps().saturating_mul(1000);
        self.rebuild_needed = true;

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
        let callback = match self.callback {
            Some(callback) => callback,
            None => return VideoCodecStatus::Uninitialized,
        };

        let frame_width = frame.width().max(0) as u32;
        let frame_height = frame.height().max(0) as u32;
        if frame_width == 0 || frame_height == 0 {
            return VideoCodecStatus::ErrParameter;
        }

        let requested_frame_type = requested_frame_type(frame_types);

        if frame_width != self.width || frame_height != self.height {
            self.width = frame_width;
            self.height = frame_height;
            self.rebuild_needed = true;
        }
        if self.encoder.is_none() {
            self.rebuild_needed = true;
        }
        if self.rebuild_needed && self.rebuild_encoder().is_err() {
            return VideoCodecStatus::Error;
        }

        let mut frame_buffer = frame.buffer();
        let Some(i420) = frame_buffer.to_i420() else {
            return VideoCodecStatus::Error;
        };

        let encoder = self.encoder.as_mut().expect("encoder should exist");
        let resolution = encoder.resolution();

        let chroma_stride = resolution.stride.div_ceil(2);
        let chroma_height = resolution.height.div_ceil(2);
        let yuv_size = resolution.yuv420_size();
        let y_size = resolution.stride * resolution.height;
        let uv_size = chroma_stride * chroma_height;
        let mut i420_data = vec![0u8; yuv_size];
        let (dst_y, dst_uv) = i420_data.split_at_mut(y_size as usize);
        let (dst_u, dst_v) = dst_uv.split_at_mut(uv_size as usize);
        if !i420_copy(
            i420.y_data(),
            i420.stride_y(),
            i420.u_data(),
            i420.stride_u(),
            i420.v_data(),
            i420.stride_v(),
            dst_y,
            resolution.stride as i32,
            dst_u,
            chroma_stride as i32,
            dst_v,
            chroma_stride as i32,
            i420.width(),
            i420.height(),
        ) {
            return VideoCodecStatus::Error;
        }

        let force_keyframe = matches!(requested_frame_type, Some(VideoFrameType::Key));
        let timestamp_us = frame.timestamp_us();
        let encoded =
            match encoder.encode(InputFrame::I420(&i420_data), timestamp_us, force_keyframe) {
                Ok(encoded) => encoded,
                Err(err) => {
                    rtc_log_error!("V4L2 encode failed: {}", err);
                    return VideoCodecStatus::Error;
                }
            };

        let mut encoded_image = EncodedImage::new();
        let encoded_buffer = EncodedImageBuffer::from_bytes(&encoded.data);
        encoded_image.set_encoded_data(&encoded_buffer);
        encoded_image.set_rtp_timestamp(frame.rtp_timestamp());
        encoded_image.set_encoded_width(frame_width);
        encoded_image.set_encoded_height(frame_height);
        encoded_image.set_frame_type(if encoded.is_keyframe {
            VideoFrameType::Key
        } else {
            VideoFrameType::Delta
        });

        let mut codec_specific_info = CodecSpecificInfo::new();
        codec_specific_info.set_codec_type(VideoCodecType::H264);
        codec_specific_info.set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
        codec_specific_info.set_h264_idr_frame(encoded.is_keyframe);

        let result = unsafe {
            callback.on_encoded_image(encoded_image.as_ref(), Some(codec_specific_info.as_ref()))
        };
        if result.error() != VideoEncoderEncodedImageCallbackResultError::Ok {
            rtc_log_warning!(
                "V4L2: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
            );
        }

        VideoCodecStatus::Ok
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
        self.rebuild_needed = false;
        VideoCodecStatus::Ok
    }

    fn set_rates(&mut self, parameters: VideoEncoderRateControlParametersRef<'_>) {
        let bitrate_bps = parameters
            .bitrate_sum_bps()
            .max(parameters.target_bitrate_sum_bps())
            .max(1);
        self.target_bitrate_bps = bitrate_bps;

        let Some(encoder) = self.encoder.as_mut() else {
            self.rebuild_needed = true;
            return;
        };

        if let Err(err) = encoder.set_bitrate(bitrate_bps) {
            rtc_log_warning!(
                "V4L2 set_bitrate failed: {}; mark rebuild for next frame",
                err
            );
            self.rebuild_needed = true;
        }
    }

    fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
        let mut info = VideoEncoderEncoderInfo::new();
        info.set_implementation_name("V4L2");
        info.set_is_hardware_accelerated(true);
        info
    }
}

struct V4l2VideoDecoder {
    callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    decoder: Option<H264Decoder>,
    device_path: String,
    resolution: Option<Resolution>,
}

impl V4l2VideoDecoder {
    fn new(device_path: String) -> Self {
        Self {
            callback: None,
            decoder: None,
            device_path,
            resolution: None,
        }
    }

    fn rebuild_decoder(&mut self) -> Result<()> {
        let mut config = DecoderConfig::new();
        config.device_path = self.device_path.clone();
        self.decoder = Some(H264Decoder::new(config)?);
        Ok(())
    }

    fn ensure_decoder(&mut self) -> Result<()> {
        if self.decoder.is_none() {
            self.rebuild_decoder()?;
        }
        Ok(())
    }
}

impl VideoDecoderHandler for V4l2VideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != VideoCodecType::H264 {
            return false;
        }
        self.rebuild_decoder().is_ok()
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

        let decoder = self.decoder.as_mut().expect("decoder should exist");

        let output = match decoder.decode(encoded_data.data(), render_time_ms.saturating_mul(1000))
        {
            Ok(output) => output,
            Err(err) => {
                rtc_log_error!("V4L2 decode failed: {}", err);
                return VideoCodecStatus::Error;
            }
        };

        match output {
            DecodeOutput::Pending => VideoCodecStatus::NoOutput,
            DecodeOutput::ResolutionChanged { .. } => {
                self.resolution = decoder.resolution();
                VideoCodecStatus::NoOutput
            }
            DecodeOutput::Frame(frame) => {
                let frame_data = frame.data;
                let frame_index = frame.index;
                let frame_timestamp_us = frame.timestamp_us;
                let Some(resolution) = self.resolution else {
                    return VideoCodecStatus::Error;
                };

                let decoded_frame = build_i420_frame(
                    frame_data,
                    resolution.width,
                    resolution.height,
                    resolution.stride,
                    frame_timestamp_us,
                    input_image.rtp_timestamp(),
                );

                if let Err(err) = decoder.release_buffer(frame_index) {
                    rtc_log_error!("V4L2 release_buffer failed: {}", err);
                    return VideoCodecStatus::Error;
                };

                let Some(decoded_frame) = decoded_frame else {
                    return VideoCodecStatus::Error;
                };

                let Some(callback) = self.callback.as_ref() else {
                    return VideoCodecStatus::Uninitialized;
                };
                unsafe {
                    callback.decoded(decoded_frame.as_ref());
                }

                VideoCodecStatus::Ok
            }
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
        info.set_implementation_name("V4L2");
        info.set_is_hardware_accelerated(true);
        info
    }
}

pub struct V4l2VideoCodecCapability {
    decoder_device_path: String,
    simulcast_capability_helper: SimulcastCapabilityHelper,
}

impl V4l2VideoCodecCapability {
    pub fn new() -> Result<Self> {
        Self::new_with_device_paths("/dev/video11".to_string(), "/dev/video10".to_string(), true)
    }

    fn new_with_device_paths(
        encoder_device_path: String,
        decoder_device_path: String,
        probe_device: bool,
    ) -> Result<Self> {
        if probe_device {
            probe_device_path(&encoder_device_path)?;
            probe_device_path(&decoder_device_path)?;
        }

        let simulcast_capability_helper = SimulcastCapabilityHelper::new_with_builder(
            v4l2_supported_formats,
            move |_env, format| {
                if codec_type_from_format(&format)? != VideoCodecType::H264 {
                    return None;
                }
                Some(VideoEncoder::new_with_handler(Box::new(
                    V4l2VideoEncoder::new(encoder_device_path.clone()),
                )))
            },
        );

        Ok(Self {
            decoder_device_path,
            simulcast_capability_helper,
        })
    }
}

fn probe_device_path(device_path: &str) -> Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(device_path)
        .map_err(|err| Error::V4l2Message {
            reason: format!("failed to open V4L2 device {device_path}: {err}"),
        })?;
    Ok(())
}

impl VideoCodecCapability for V4l2VideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("v4l2", "V4L2 M2M")
    }

    fn get_supported_formats(&self, _direction: CodecDirection) -> Vec<SdpVideoFormat> {
        v4l2_supported_formats()
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
        _format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        Some(VideoDecoder::new_with_handler(Box::new(
            V4l2VideoDecoder::new(self.decoder_device_path.clone()),
        )))
    }
}

#[cfg(test)]
impl V4l2VideoCodecCapability {
    fn new_for_test() -> Result<Self> {
        Self::new_with_device_paths(
            "/dev/video11".to_string(),
            "/dev/video10".to_string(),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::{Environment, SdpVideoFormat, VideoFrameType, VideoFrameTypeVector};

    #[test]
    fn v4l2_capability_has_expected_implementation_name() {
        let capability =
            V4l2VideoCodecCapability::new_for_test().expect("failed to create V4L2 capability");
        assert_eq!(capability.get_implementation().name(), "v4l2");
    }

    #[test]
    fn v4l2_capability_supports_only_h264() {
        let capability =
            V4l2VideoCodecCapability::new_for_test().expect("failed to create V4L2 capability");

        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));
        assert!(!capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp8));
        assert!(!capability.is_supported(CodecDirection::Decoder, VideoCodecType::Av1));

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
    }

    #[test]
    fn v4l2_requested_frame_type_uses_first_entry() {
        assert_eq!(requested_frame_type(None), None);

        let mut frame_types = VideoFrameTypeVector::new(2);
        frame_types.push(VideoFrameType::Empty);
        frame_types.push(VideoFrameType::Key);
        assert_eq!(
            requested_frame_type(Some(frame_types.as_ref())),
            Some(VideoFrameType::Empty)
        );
    }

    #[test]
    fn v4l2_create_video_encoder_uses_simulcast_adapter() {
        let capability =
            V4l2VideoCodecCapability::new_for_test().expect("failed to create V4L2 capability");
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

    #[test]
    fn build_i420_frame_fails_for_short_input() {
        assert!(build_i420_frame(&[0; 7], 4, 4, 4, 0, 0).is_none());
    }
}
