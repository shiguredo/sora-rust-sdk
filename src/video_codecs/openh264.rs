use std::collections::HashMap;
use std::path::Path;

use shiguredo_openh264::{
    Decoder, EncodeOptions, Encoder, EncoderConfig, Error as Openh264Error, FrameType,
    Openh264Library, RateControlMode, SliceMode,
};
use shiguredo_webrtc::{
    CodecSpecificInfo, EncodedImage, EncodedImageBuffer, EncodedImageRef, H264PacketizationMode,
    I420Buffer, SdpVideoFormat, VideoCodecRef, VideoCodecStatus, VideoCodecType,
    VideoDecoderDecodedImageCallbackPtr, VideoDecoderDecoderInfo, VideoDecoderHandler,
    VideoDecoderSettingsRef, VideoEncoderEncodedImageCallbackPtr,
    VideoEncoderEncodedImageCallbackRef, VideoEncoderEncodedImageCallbackResultError,
    VideoEncoderEncoderInfo, VideoEncoderHandler, VideoEncoderRateControlParametersRef,
    VideoEncoderSettingsRef, VideoFrame, VideoFrameRef, VideoFrameType, VideoFrameTypeVectorRef,
};

use crate::error::Result;
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

struct Openh264VideoEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
    encoder: Option<Encoder>,
    library: Openh264Library,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    reconfigure_needed: bool,
    paused: bool,
    force_idr_on_resume: bool,
}

impl Openh264VideoEncoder {
    fn new(library: Openh264Library) -> Self {
        Self {
            callback: None,
            encoder: None,
            library,
            width: 0,
            height: 0,
            framerate: 30,
            target_bitrate_bps: 500_000,
            reconfigure_needed: false,
            paused: false,
            force_idr_on_resume: false,
        }
    }

    fn flatten_i420_for_encoder(i420: &I420Buffer, width: u32, height: u32) -> Option<I420Frame> {
        let width = usize::try_from(width).ok()?;
        let height = usize::try_from(height).ok()?;
        let stride_y = usize::try_from(i420.stride_y()).ok()?;
        let stride_u = usize::try_from(i420.stride_u()).ok()?;
        let stride_v = usize::try_from(i420.stride_v()).ok()?;

        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);

        if width == 0
            || height == 0
            || stride_y < width
            || stride_u < chroma_width
            || stride_v < chroma_width
        {
            return None;
        }

        let y_size = width.checked_mul(height)?;
        let uv_size = chroma_width.checked_mul(chroma_height)?;

        let src_y = i420.y_data();
        let src_u = i420.u_data();
        let src_v = i420.v_data();

        let mut y = vec![0u8; y_size];
        let mut u = vec![0u8; uv_size];
        let mut v = vec![0u8; uv_size];

        for row in 0..height {
            let src_offset = row.checked_mul(stride_y)?;
            let dst_offset = row.checked_mul(width)?;
            let src_row = src_y.get(src_offset..src_offset + width)?;
            y[dst_offset..dst_offset + width].copy_from_slice(src_row);
        }

        for row in 0..chroma_height {
            let src_offset_u = row.checked_mul(stride_u)?;
            let src_offset_v = row.checked_mul(stride_v)?;
            let dst_offset = row.checked_mul(chroma_width)?;
            let src_row_u = src_u.get(src_offset_u..src_offset_u + chroma_width)?;
            let src_row_v = src_v.get(src_offset_v..src_offset_v + chroma_width)?;
            u[dst_offset..dst_offset + chroma_width].copy_from_slice(src_row_u);
            v[dst_offset..dst_offset + chroma_width].copy_from_slice(src_row_v);
        }

        Some(I420Frame { y, u, v })
    }

    fn prepare_i420_for_encoder<'a>(
        i420: &'a I420Buffer,
        width: u32,
        height: u32,
    ) -> Option<I420Input<'a>> {
        let width = usize::try_from(width).ok()?;
        let height = usize::try_from(height).ok()?;
        let stride_y = usize::try_from(i420.stride_y()).ok()?;
        let stride_u = usize::try_from(i420.stride_u()).ok()?;
        let stride_v = usize::try_from(i420.stride_v()).ok()?;

        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);
        let y_size = width.checked_mul(height)?;
        let uv_size = chroma_width.checked_mul(chroma_height)?;

        if can_use_borrowed_i420(I420Layout {
            width,
            height,
            stride_y,
            stride_u,
            stride_v,
            y_len: i420.y_data().len(),
            u_len: i420.u_data().len(),
            v_len: i420.v_data().len(),
        }) {
            let y = i420.y_data().get(..y_size)?;
            let u = i420.u_data().get(..uv_size)?;
            let v = i420.v_data().get(..uv_size)?;
            Some(I420Input::Borrowed { y, u, v })
        } else {
            Self::flatten_i420_for_encoder(i420, width as u32, height as u32).map(I420Input::Owned)
        }
    }

    fn build_encoder_config(&self) -> Option<EncoderConfig> {
        if self.width == 0 || self.height == 0 {
            return None;
        }

        let mut config = EncoderConfig::new(
            usize::try_from(self.width).ok()?,
            usize::try_from(self.height).ok()?,
            usize::try_from(self.target_bitrate_bps.max(1)).ok()?,
            usize::try_from(self.framerate.max(1)).ok()?,
            1,
        );
        config.rate_control_mode = Some(RateControlMode::Bitrate);
        config.slice_mode = Some(SliceMode::FixedCount(1));
        Some(config)
    }

    fn rebuild_encoder(&mut self) -> std::result::Result<(), Openh264Error> {
        let Some(config) = self.build_encoder_config() else {
            return Err(Openh264Error::InvalidParameter(
                "width and height must be non-zero".to_string(),
            ));
        };

        self.encoder = Some(Encoder::new(self.library.clone(), config)?);
        self.reconfigure_needed = false;
        Ok(())
    }
}

impl VideoEncoderHandler for Openh264VideoEncoder {
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
        self.paused = false;
        self.force_idr_on_resume = false;

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
        if self.paused {
            return VideoCodecStatus::NoOutput;
        }

        let frame_width = frame.width().max(0) as u32;
        let frame_height = frame.height().max(0) as u32;
        if frame_width == 0 || frame_height == 0 {
            return VideoCodecStatus::ErrParameter;
        }
        let requested_frame_type = requested_frame_type(frame_types);
        if matches!(requested_frame_type, Some(VideoFrameType::Empty)) {
            return VideoCodecStatus::NoOutput;
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

        let Some(i420_frame) = Self::prepare_i420_for_encoder(&i420, frame_width, frame_height)
        else {
            return VideoCodecStatus::Error;
        };

        let force_idr_from_resume = self.force_idr_on_resume;
        let options = EncodeOptions {
            force_idr: force_idr_from_resume
                || matches!(requested_frame_type, Some(VideoFrameType::Key)),
        };
        let encoder = self.encoder.as_mut().expect("encoder should exist");
        let encoded = match i420_frame {
            I420Input::Borrowed { y, u, v } => encoder.encode(y, u, v, &options),
            I420Input::Owned(frame) => encoder.encode(&frame.y, &frame.u, &frame.v, &options),
        };
        let encoded = match encoded {
            Ok(v) => v,
            Err(_) => {
                self.reconfigure_needed = true;
                return VideoCodecStatus::Error;
            }
        };

        let Some(encoded) = encoded else {
            return VideoCodecStatus::NoOutput;
        };
        if force_idr_from_resume {
            self.force_idr_on_resume = false;
        }

        let mut annexb = Vec::new();
        for sps in &encoded.sps_list {
            append_annexb_nalu(&mut annexb, sps);
        }
        for pps in &encoded.pps_list {
            append_annexb_nalu(&mut annexb, pps);
        }
        if !encoded.data.is_empty() {
            if has_annexb_start_code(&encoded.data) {
                annexb.extend_from_slice(&encoded.data);
            } else {
                append_annexb_nalu(&mut annexb, &encoded.data);
            }
        }
        if annexb.is_empty() {
            return VideoCodecStatus::NoOutput;
        }

        let mut encoded_image = EncodedImage::new();
        let encoded_buffer = EncodedImageBuffer::from_bytes(&annexb);
        encoded_image.set_encoded_data(&encoded_buffer);
        encoded_image.set_rtp_timestamp(frame.rtp_timestamp());
        encoded_image.set_encoded_width(frame_width);
        encoded_image.set_encoded_height(frame_height);
        encoded_image.set_frame_type(frame_type_from_openh264(encoded.frame_type));

        let mut codec_specific_info = CodecSpecificInfo::new();
        codec_specific_info.set_codec_type(VideoCodecType::H264);
        codec_specific_info.set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
        codec_specific_info.set_h264_idr_frame(matches!(encoded.frame_type, FrameType::Idr));

        let result = unsafe {
            callback.on_encoded_image(encoded_image.as_ref(), Some(codec_specific_info.as_ref()))
        };
        if result.error() != VideoEncoderEncodedImageCallbackResultError::Ok {
            return VideoCodecStatus::Error;
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
        self.paused = false;
        self.force_idr_on_resume = false;
        VideoCodecStatus::Ok
    }

    fn set_rates(&mut self, parameters: VideoEncoderRateControlParametersRef<'_>) {
        self.framerate = parameters.framerate_fps().max(1.0) as u32;
        let bitrate = update_pause_state(
            &mut self.paused,
            &mut self.force_idr_on_resume,
            parameters.bitrate_sum_bps(),
            parameters.target_bitrate_sum_bps(),
        );
        let Some(bitrate) = bitrate else {
            return;
        };
        self.target_bitrate_bps = bitrate;

        let Some(encoder) = self.encoder.as_mut() else {
            self.reconfigure_needed = true;
            return;
        };

        let mut failed = false;
        match usize::try_from(self.target_bitrate_bps) {
            Ok(bitrate_bps) => {
                if encoder.set_bitrate(bitrate_bps).is_err() {
                    failed = true;
                }
            }
            Err(_) => {
                failed = true;
            }
        }
        match usize::try_from(self.framerate) {
            Ok(framerate) => {
                if encoder.set_frame_rate(framerate.max(1), 1).is_err() {
                    failed = true;
                }
            }
            Err(_) => {
                failed = true;
            }
        }
        if failed {
            self.reconfigure_needed = true;
        }
    }

    fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
        let mut info = VideoEncoderEncoderInfo::new();
        info.set_implementation_name("OpenH264");
        info.set_is_hardware_accelerated(false);
        info
    }
}

struct Openh264VideoDecoder {
    callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    decoder: Option<Decoder>,
    library: Openh264Library,
}

impl Openh264VideoDecoder {
    fn new(library: Openh264Library) -> Self {
        Self {
            callback: None,
            decoder: None,
            library,
        }
    }

    fn ensure_decoder(&mut self) -> bool {
        if self.decoder.is_none() {
            self.decoder = Decoder::new(self.library.clone()).ok();
        }
        self.decoder.is_some()
    }
}

impl VideoDecoderHandler for Openh264VideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != VideoCodecType::H264 {
            return false;
        }
        self.decoder = Decoder::new(self.library.clone()).ok();
        self.decoder.is_some()
    }

    fn decode(
        &mut self,
        input_image: EncodedImageRef<'_>,
        render_time_ms: i64,
    ) -> VideoCodecStatus {
        if !self.ensure_decoder() {
            return VideoCodecStatus::Error;
        }
        let Some(callback) = self.callback.as_ref() else {
            return VideoCodecStatus::Uninitialized;
        };

        let Some(encoded_data) = input_image.encoded_data() else {
            return VideoCodecStatus::ErrParameter;
        };

        let decoder = self.decoder.as_mut().expect("decoder should exist");
        let decoded = match decoder.decode(encoded_data.data()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let Some(decoded) = decoded else {
            return VideoCodecStatus::NoOutput;
        };

        let width = match i32::try_from(decoded.width()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let height = match i32::try_from(decoded.height()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };

        let mut i420 = I420Buffer::new(width, height);
        let dst_stride_y = match usize::try_from(i420.stride_y()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let dst_stride_u = match usize::try_from(i420.stride_u()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let dst_stride_v = match usize::try_from(i420.stride_v()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let (dst_y, dst_u, dst_v) = i420.planes_mut();

        let width_usize = decoded.width();
        let height_usize = decoded.height();
        let chroma_width = width_usize.div_ceil(2);
        let chroma_height = height_usize.div_ceil(2);

        if !copy_plane(
            dst_y,
            dst_stride_y,
            decoded.y_plane(),
            decoded.y_stride(),
            width_usize,
            height_usize,
        ) {
            return VideoCodecStatus::Error;
        }
        if !copy_plane(
            dst_u,
            dst_stride_u,
            decoded.u_plane(),
            decoded.u_stride(),
            chroma_width,
            chroma_height,
        ) {
            return VideoCodecStatus::Error;
        }
        if !copy_plane(
            dst_v,
            dst_stride_v,
            decoded.v_plane(),
            decoded.v_stride(),
            chroma_width,
            chroma_height,
        ) {
            return VideoCodecStatus::Error;
        }

        let frame = VideoFrame::builder(&i420.cast_to_video_frame_buffer())
            .set_timestamp_us(render_time_ms.saturating_mul(1000))
            .set_rtp_timestamp(input_image.rtp_timestamp())
            .build();
        unsafe {
            callback.decoded(frame.as_ref());
        }

        VideoCodecStatus::Ok
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
        info.set_implementation_name("OpenH264");
        info.set_is_hardware_accelerated(false);
        info
    }
}

pub struct Openh264VideoCodecCapability {
    library: Openh264Library,
}

impl Openh264VideoCodecCapability {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let library = Openh264Library::load(path)?;
        let info = library.supported_codecs();
        if !info.encoding.supported || !info.decoding.supported {
            return Err(crate::Error::InvalidVideoCodecCapability {
                reason: "OpenH264 does not support both encoder and decoder".to_string(),
            });
        }
        Ok(Self { library })
    }
}

impl VideoCodecCapability for Openh264VideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("openh264", "OpenH264")
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
            Some(Box::new(Openh264VideoEncoder::new(self.library.clone())))
        } else {
            None
        }
    }

    fn create_video_decoder(
        &self,
        format: &SdpVideoFormat,
    ) -> Option<Box<dyn VideoDecoderHandler>> {
        if format.name().ok().as_deref() == Some("H264") {
            Some(Box::new(Openh264VideoDecoder::new(self.library.clone())))
        } else {
            None
        }
    }
}

struct I420Frame {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

enum I420Input<'a> {
    Borrowed {
        y: &'a [u8],
        u: &'a [u8],
        v: &'a [u8],
    },
    Owned(I420Frame),
}

fn frame_type_from_openh264(frame_type: FrameType) -> VideoFrameType {
    match frame_type {
        FrameType::Idr => VideoFrameType::Key,
        FrameType::I | FrameType::P => VideoFrameType::Delta,
    }
}

fn requested_frame_type(
    frame_types: Option<VideoFrameTypeVectorRef<'_>>,
) -> Option<VideoFrameType> {
    frame_types.and_then(|frame_types| frame_types.get(0))
}

struct I420Layout {
    width: usize,
    height: usize,
    stride_y: usize,
    stride_u: usize,
    stride_v: usize,
    y_len: usize,
    u_len: usize,
    v_len: usize,
}

fn can_use_borrowed_i420(layout: I420Layout) -> bool {
    if layout.width == 0 || layout.height == 0 {
        return false;
    }
    let chroma_width = layout.width.div_ceil(2);
    let chroma_height = layout.height.div_ceil(2);
    let Some(y_size) = layout.width.checked_mul(layout.height) else {
        return false;
    };
    let Some(uv_size) = chroma_width.checked_mul(chroma_height) else {
        return false;
    };
    layout.stride_y == layout.width
        && layout.stride_u == chroma_width
        && layout.stride_v == chroma_width
        && layout.y_len >= y_size
        && layout.u_len >= uv_size
        && layout.v_len >= uv_size
}

fn update_pause_state(
    paused: &mut bool,
    force_idr_on_resume: &mut bool,
    bitrate_sum_bps: u32,
    target_bitrate_sum_bps: u32,
) -> Option<u32> {
    let bitrate = bitrate_sum_bps.max(target_bitrate_sum_bps);
    if bitrate == 0 {
        *paused = true;
        return None;
    }
    if *paused {
        *paused = false;
        *force_idr_on_resume = true;
    }
    Some(bitrate.max(1))
}

fn append_annexb_nalu(out: &mut Vec<u8>, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    if has_annexb_start_code(data) {
        out.extend_from_slice(data);
        return;
    }
    out.extend_from_slice(&[0, 0, 0, 1]);
    out.extend_from_slice(data);
}

fn has_annexb_start_code(data: &[u8]) -> bool {
    (data.len() >= 4 && data[0] == 0 && data[1] == 0 && data[2] == 0 && data[3] == 1)
        || (data.len() >= 3 && data[0] == 0 && data[1] == 0 && data[2] == 1)
}

fn copy_plane(
    dst: &mut [u8],
    dst_stride: usize,
    src: &[u8],
    src_stride: usize,
    row_bytes: usize,
    row_count: usize,
) -> bool {
    if row_bytes == 0 || row_count == 0 || src_stride < row_bytes || dst_stride < row_bytes {
        return false;
    }

    for row in 0..row_count {
        let src_offset = match row.checked_mul(src_stride) {
            Some(v) => v,
            None => return false,
        };
        let dst_offset = match row.checked_mul(dst_stride) {
            Some(v) => v,
            None => return false,
        };
        let Some(src_row) = src.get(src_offset..src_offset + row_bytes) else {
            return false;
        };
        let Some(dst_row) = dst.get_mut(dst_offset..dst_offset + row_bytes) else {
            return false;
        };
        dst_row.copy_from_slice(src_row);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::{VideoFrameType, VideoFrameTypeVector};

    fn openh264_path() -> Option<String> {
        std::env::var("OPENH264_PATH").ok()
    }

    #[test]
    fn openh264_capability_creation_fails_with_invalid_path() {
        let result = Openh264VideoCodecCapability::new("/path/to/not-found/libopenh264.so");
        assert!(result.is_err());
    }

    #[test]
    fn openh264_capability_supports_only_h264() {
        let Some(path) = openh264_path() else {
            println!("SKIP: OPENH264_PATH is not set");
            return;
        };

        let capability = Openh264VideoCodecCapability::new(path)
            .expect("Openh264VideoCodecCapability::new must succeed");

        assert_eq!(capability.get_implementation().name(), "openh264");
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

        let resolved_with_profile_level_id = capability.resolve_sdp_format(
            CodecDirection::Encoder,
            VideoCodecType::H264,
            &HashMap::from([(String::from("profile-level-id"), String::from("42e01f"))]),
            None,
        );
        assert!(resolved_with_profile_level_id.is_some());
    }

    #[test]
    fn openh264_frame_type_mapping_is_compatible_with_cpp() {
        assert_eq!(
            frame_type_from_openh264(FrameType::Idr),
            VideoFrameType::Key
        );
        assert_eq!(
            frame_type_from_openh264(FrameType::I),
            VideoFrameType::Delta
        );
        assert_eq!(
            frame_type_from_openh264(FrameType::P),
            VideoFrameType::Delta
        );
    }

    #[test]
    fn openh264_requested_frame_type_uses_first_entry() {
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
    fn openh264_can_use_borrowed_i420_only_for_contiguous_planes() {
        assert!(can_use_borrowed_i420(I420Layout {
            width: 640,
            height: 480,
            stride_y: 640,
            stride_u: 320,
            stride_v: 320,
            y_len: 640 * 480,
            u_len: 320 * 240,
            v_len: 320 * 240,
        }));
        assert!(!can_use_borrowed_i420(I420Layout {
            width: 640,
            height: 480,
            stride_y: 672,
            stride_u: 320,
            stride_v: 320,
            y_len: 640 * 480,
            u_len: 320 * 240,
            v_len: 320 * 240,
        }));
    }

    #[test]
    fn openh264_pause_state_sets_force_idr_on_resume() {
        let mut paused = false;
        let mut force_idr_on_resume = false;

        assert_eq!(
            update_pause_state(&mut paused, &mut force_idr_on_resume, 0, 0),
            None
        );
        assert!(paused);
        assert!(!force_idr_on_resume);

        assert_eq!(
            update_pause_state(&mut paused, &mut force_idr_on_resume, 120_000, 100_000),
            Some(120_000)
        );
        assert!(!paused);
        assert!(force_idr_on_resume);
    }
}
