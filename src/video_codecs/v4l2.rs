use std::collections::HashMap;
use std::fs::OpenOptions;
use std::sync::{Arc, Mutex, MutexGuard};

use shiguredo_v4l2::v4l2_m2m::{
    DecodeCallbackOutput, DecodeInput, DecoderConfig, EncodeCallbackOutput, EncodeInput,
    EncoderConfig, Error as V4l2Error, H264Decoder, H264Encoder, Resolution,
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

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
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

fn handle_v4l2_encode_callback(
    callback_state: &Arc<Mutex<EncoderCallbackState>>,
    result: shiguredo_v4l2::v4l2_m2m::Result<EncodeCallbackOutput<EncoderCallbackValue>>,
) {
    let (encoded, value) = match result {
        Ok(EncodeCallbackOutput::Frame { frame, value }) => (frame, value),
        Err(err) => {
            rtc_log_error!("V4L2 encode callback failed: {}", err);
            return;
        }
    };
    let Some(encoded_data) = encoded.data() else {
        rtc_log_error!("V4L2 encode callback returned frame without MMAP data");
        return;
    };

    let mut encoded_image = EncodedImage::new();
    let encoded_buffer = EncodedImageBuffer::from_bytes(encoded_data);
    encoded_image.set_encoded_data(&encoded_buffer);
    encoded_image.set_rtp_timestamp(value.rtp_timestamp);
    encoded_image.set_encoded_width(value.frame_width);
    encoded_image.set_encoded_height(value.frame_height);
    encoded_image.set_frame_type(if encoded.is_keyframe() {
        VideoFrameType::Key
    } else {
        VideoFrameType::Delta
    });

    let mut codec_specific_info = CodecSpecificInfo::new();
    codec_specific_info.set_codec_type(VideoCodecType::H264);
    codec_specific_info.set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
    codec_specific_info.set_h264_idr_frame(encoded.is_keyframe());

    let callback_state = lock_unpoisoned(callback_state);
    let Some(callback) = callback_state.callback else {
        return;
    };

    let result = unsafe {
        callback.on_encoded_image(encoded_image.as_ref(), Some(codec_specific_info.as_ref()))
    };
    if result.error() != VideoEncoderEncodedImageCallbackResultError::Ok {
        rtc_log_warning!(
            "V4L2: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
        );
    }
}

struct V4l2VideoEncoder {
    encoder: Option<H264Encoder<EncoderCallbackValue>>,
    callback_state: Arc<Mutex<EncoderCallbackState>>,
    device_path: String,
    width: u32,
    height: u32,
    target_bitrate_bps: u32,
    rebuild_needed: bool,
}

impl V4l2VideoEncoder {
    fn new(device_path: String) -> Self {
        Self {
            encoder: None,
            callback_state: Arc::new(Mutex::new(EncoderCallbackState::default())),
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
        let callback_state = self.callback_state.clone();
        self.encoder = Some(H264Encoder::new(config, move |result| {
            handle_v4l2_encode_callback(&callback_state, result);
        })?);
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
        let has_callback = {
            let callback_state = lock_unpoisoned(&self.callback_state);
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

        let mut fill = |buf: &mut [u8],
                        resolution: &Resolution,
                        _value: &EncoderCallbackValue|
         -> Option<usize> {
            let chroma_stride = resolution.stride.div_ceil(2);
            let chroma_height = resolution.height.div_ceil(2);
            let yuv_size = resolution.yuv420_size();
            if buf.len() < yuv_size {
                return None;
            }
            let y_size = (resolution.stride as usize) * (resolution.height as usize);
            let uv_size = (chroma_stride as usize) * (chroma_height as usize);
            let dst_stride_y = i32::try_from(resolution.stride).ok()?;
            let dst_stride_uv = i32::try_from(chroma_stride).ok()?;
            let (dst_y, dst_uv) = buf.split_at_mut(y_size);
            let (dst_u, dst_v) = dst_uv.split_at_mut(uv_size);
            if !i420_copy(
                i420.y_data(),
                i420.stride_y(),
                i420.u_data(),
                i420.stride_u(),
                i420.v_data(),
                i420.stride_v(),
                dst_y,
                dst_stride_y,
                dst_u,
                dst_stride_uv,
                dst_v,
                dst_stride_uv,
                i420.width(),
                i420.height(),
            ) {
                return None;
            }
            Some(yuv_size)
        };

        let force_keyframe = matches!(requested_frame_type, Some(VideoFrameType::Key));
        let timestamp_us = frame.timestamp_us();
        let callback_value = EncoderCallbackValue {
            rtp_timestamp: frame.rtp_timestamp(),
            frame_width,
            frame_height,
        };
        match encoder.encode(
            EncodeInput::Mmap(&mut fill),
            timestamp_us,
            force_keyframe,
            callback_value,
        ) {
            Ok(()) => VideoCodecStatus::Ok,
            Err(V4l2Error::NoAvailableBuffer) => VideoCodecStatus::NoOutput,
            Err(err) => {
                rtc_log_error!("V4L2 encode failed: {}", err);
                VideoCodecStatus::Error
            }
        }
    }

    fn register_encode_complete_callback(
        &mut self,
        callback: Option<VideoEncoderEncodedImageCallbackRef<'_>>,
    ) -> VideoCodecStatus {
        let mut callback_state = lock_unpoisoned(&self.callback_state);
        callback_state.callback = callback
            .map(|callback| unsafe { VideoEncoderEncodedImageCallbackPtr::from_ref(callback) });
        VideoCodecStatus::Ok
    }

    fn release(&mut self) -> VideoCodecStatus {
        let mut callback_state = lock_unpoisoned(&self.callback_state);
        callback_state.callback = None;
        drop(callback_state);
        self.encoder = None;
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

#[derive(Clone, Copy)]
struct DecoderCallbackValue {
    rtp_timestamp: u32,
}

#[derive(Default)]
struct DecoderCallbackState {
    callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    resolution: Option<Resolution>,
}

fn handle_v4l2_decode_callback(
    callback_state: &Arc<Mutex<DecoderCallbackState>>,
    result: shiguredo_v4l2::v4l2_m2m::Result<DecodeCallbackOutput<DecoderCallbackValue>>,
) {
    match result {
        Ok(DecodeCallbackOutput::ResolutionChanged { resolution }) => {
            let mut callback_state = lock_unpoisoned(callback_state);
            callback_state.resolution = Some(resolution);
        }
        Ok(DecodeCallbackOutput::Frame { frame, value }) => {
            let callback_state = lock_unpoisoned(callback_state);
            let resolution = match callback_state.resolution {
                Some(resolution) => resolution,
                None => {
                    rtc_log_error!("V4L2 decode callback returned frame before resolution");
                    return;
                }
            };
            let Some(frame_data) = frame.data() else {
                rtc_log_error!("V4L2 decode callback returned frame without MMAP data");
                return;
            };

            let decoded_frame = match build_i420_frame(
                frame_data,
                resolution.width,
                resolution.height,
                resolution.stride,
                frame.timestamp_us(),
                value.rtp_timestamp,
            ) {
                Some(decoded_frame) => decoded_frame,
                None => {
                    rtc_log_error!(
                        "V4L2 decode callback failed to build I420 frame: width={}, height={}, stride={}, bytes={}",
                        resolution.width,
                        resolution.height,
                        resolution.stride,
                        frame_data.len(),
                    );
                    return;
                }
            };

            let Some(callback) = callback_state.callback else {
                return;
            };
            unsafe {
                callback.decoded(decoded_frame.as_ref());
            }
        }
        Err(err) => {
            rtc_log_error!("V4L2 decode callback failed: {}", err);
        }
    }
}

struct V4l2VideoDecoder {
    callback_state: Arc<Mutex<DecoderCallbackState>>,
    decoder: Option<H264Decoder<DecoderCallbackValue>>,
    device_path: String,
}

impl V4l2VideoDecoder {
    fn new(device_path: String) -> Self {
        Self {
            callback_state: Arc::new(Mutex::new(DecoderCallbackState::default())),
            decoder: None,
            device_path,
        }
    }

    fn rebuild_decoder(&mut self) -> Result<()> {
        let mut config = DecoderConfig::new();
        config.device_path = self.device_path.clone();
        let callback_state = self.callback_state.clone();
        self.decoder = Some(H264Decoder::new(config, move |result| {
            handle_v4l2_decode_callback(&callback_state, result);
        })?);
        let mut callback_state = lock_unpoisoned(&self.callback_state);
        callback_state.resolution = None;
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
        let has_callback = {
            let callback_state = lock_unpoisoned(&self.callback_state);
            callback_state.callback.is_some()
        };
        if !has_callback {
            return VideoCodecStatus::Uninitialized;
        }

        if self.ensure_decoder().is_err() {
            return VideoCodecStatus::Error;
        }

        let Some(encoded_data) = input_image.encoded_data() else {
            return VideoCodecStatus::ErrParameter;
        };
        let encoded_bytes = encoded_data.data();
        let mut fill = |buf: &mut [u8], _value: &DecoderCallbackValue| -> Option<usize> {
            if buf.len() < encoded_bytes.len() {
                return None;
            }
            buf[..encoded_bytes.len()].copy_from_slice(encoded_bytes);
            Some(encoded_bytes.len())
        };

        let decoder = self.decoder.as_mut().expect("decoder should exist");
        let callback_value = DecoderCallbackValue {
            rtp_timestamp: input_image.rtp_timestamp(),
        };
        match decoder.decode(
            DecodeInput::Mmap(&mut fill),
            render_time_ms.saturating_mul(1000),
            callback_value,
        ) {
            Ok(()) => {
                if let Some(resolution) = decoder.resolution() {
                    let mut callback_state = lock_unpoisoned(&self.callback_state);
                    callback_state.resolution = Some(resolution);
                }
                VideoCodecStatus::Ok
            }
            Err(V4l2Error::NoAvailableBuffer) => {
                rtc_log_warning!(
                    "V4L2 decode failed: no available buffer; consider increasing the number of buffers in the V4L2 device"
                );
                VideoCodecStatus::NoOutput
            }
            Err(err) => {
                rtc_log_error!("V4L2 decode failed: {}", err);
                VideoCodecStatus::Error
            }
        }
    }

    fn register_decode_complete_callback(
        &mut self,
        callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    ) -> VideoCodecStatus {
        let mut callback_state = lock_unpoisoned(&self.callback_state);
        callback_state.callback = callback;
        VideoCodecStatus::Ok
    }

    fn release(&mut self) -> VideoCodecStatus {
        let mut callback_state = lock_unpoisoned(&self.callback_state);
        callback_state.callback = None;
        callback_state.resolution = None;
        drop(callback_state);
        self.decoder = None;
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
