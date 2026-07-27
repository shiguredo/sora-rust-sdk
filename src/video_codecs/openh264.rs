//! OpenH264 ソフトウェアエンコーダー/デコーダー実装。
//!
//! `#[cfg(feature = "openh264")]` でのみコンパイルされる。
use std::collections::HashMap;
use std::path::Path;

use shiguredo_openh264::{
    Decoder, EncodeOptions, Encoder, EncoderConfig, Error as Openh264Error, FrameType,
    Openh264Library, RateControlMode, SliceMode,
};
use shiguredo_webrtc::{
    CodecSpecificInfo, EncodedImage, EncodedImageBuffer, EncodedImageRef, H264PacketizationMode,
    I420Buffer, ScalabilityMode, SdpVideoFormat, SdpVideoFormatRef, VideoCodecRef,
    VideoCodecStatus, VideoCodecType, VideoDecoder, VideoDecoderDecodedImageCallbackPtr,
    VideoDecoderDecoderInfo, VideoDecoderHandler, VideoDecoderSettingsRef, VideoEncoder,
    VideoEncoderEncodedImageCallbackPtr, VideoEncoderEncodedImageCallbackRef,
    VideoEncoderEncodedImageCallbackResultError, VideoEncoderEncoderInfo, VideoEncoderHandler,
    VideoEncoderRateControlParametersRef, VideoEncoderSettingsRef, VideoFrame, VideoFrameRef,
    VideoFrameType, VideoFrameTypeVectorRef, i420_copy, rtc_log_warning,
};

use crate::error::Result;
use crate::video_codec::{SimulcastCapabilityHelper, codec_type_from_format};
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

fn openh264_supported_formats() -> Vec<SdpVideoFormat> {
    vec![SdpVideoFormat::new_with_parameters(
        "H264",
        &HashMap::from([
            (String::from("level-asymmetry-allowed"), String::from("1")),
            (String::from("packetization-mode"), String::from("1")),
        ]),
        &[ScalabilityMode::L1T1],
    )]
}

struct Openh264VideoEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
    encoder: Option<Encoder>,
    library: Openh264Library,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    max_spatial_bitrate_bps: u32,
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
            max_spatial_bitrate_bps: 500_000 * 2,
            reconfigure_needed: false,
            paused: false,
            force_idr_on_resume: false,
        }
    }

    fn normalize_i420_for_encoder(
        i420: &I420Buffer,
        width: u32,
        height: u32,
    ) -> Option<I420Buffer> {
        let width_i32 = i32::try_from(width).ok()?;
        let height_i32 = i32::try_from(height).ok()?;
        let mut normalized = I420Buffer::new(width_i32, height_i32);
        let dst_stride_y = normalized.stride_y();
        let dst_stride_u = normalized.stride_u();
        let dst_stride_v = normalized.stride_v();
        let (dst_y, dst_u, dst_v) = normalized.planes_mut();
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
            dst_stride_u,
            dst_v,
            dst_stride_v,
            width_i32,
            height_i32,
        ) {
            return None;
        }
        Some(normalized)
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
        self.max_spatial_bitrate_bps = self.target_bitrate_bps * 2;
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
        self.target_bitrate_bps = codec.start_bitrate_kbps().saturating_mul(1000);
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

        let Some(i420_frame) = Self::normalize_i420_for_encoder(&i420, frame_width, frame_height)
        else {
            return VideoCodecStatus::Error;
        };

        let force_idr_from_resume = self.force_idr_on_resume;
        let options = EncodeOptions {
            force_idr: force_idr_from_resume
                || matches!(requested_frame_type, Some(VideoFrameType::Key)),
        };
        let encoder = self.encoder.as_mut().expect("encoder should exist");
        let encoded = encoder.encode(
            i420_frame.y_data(),
            i420_frame.u_data(),
            i420_frame.v_data(),
            &options,
        );
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
            rtc_log_warning!(
                "OpenH264: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
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
        if self.target_bitrate_bps >= self.max_spatial_bitrate_bps {
            self.reconfigure_needed = true;
            return;
        }

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

        let src_stride_y = match i32::try_from(decoded.y_stride()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let src_stride_u = match i32::try_from(decoded.u_stride()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let src_stride_v = match i32::try_from(decoded.v_stride()) {
            Ok(v) => v,
            Err(_) => return VideoCodecStatus::Error,
        };
        let mut i420 =
            I420Buffer::new_with_strides(width, height, src_stride_y, src_stride_u, src_stride_v);
        let dst_stride_y = i420.stride_y();
        let dst_stride_u = i420.stride_u();
        let dst_stride_v = i420.stride_v();
        let (dst_y, dst_u, dst_v) = i420.planes_mut();

        if !i420_copy(
            decoded.y_plane(),
            src_stride_y,
            decoded.u_plane(),
            src_stride_u,
            decoded.v_plane(),
            src_stride_v,
            dst_y,
            dst_stride_y,
            dst_u,
            dst_stride_u,
            dst_v,
            dst_stride_v,
            width,
            height,
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

/// OpenH264 を使用する [VideoCodecCapability]。
///
/// `#[cfg(feature = "openh264")]` でのみ利用可能。
/// H.264 のエンコード・デコードのみをサポートする。
pub struct Openh264VideoCodecCapability {
    library: Openh264Library,
    simulcast_capability_helper: SimulcastCapabilityHelper,
}

impl Openh264VideoCodecCapability {
    /// OpenH264 ライブラリを読み込み、新しい `Openh264VideoCodecCapability` を生成する。
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let library = Openh264Library::load(path)?;
        let info = library.supported_codecs();
        if !info.encoding.supported || !info.decoding.supported {
            return Err(crate::Error::InvalidVideoCodecCapability {
                reason: "OpenH264 does not support both encoder and decoder".to_string(),
            });
        }
        let encoder_library = library.clone();
        Ok(Self {
            library,
            simulcast_capability_helper: SimulcastCapabilityHelper::new_with_builder(
                openh264_supported_formats,
                {
                    let library = encoder_library;
                    move |_env, format| {
                        let codec_type = codec_type_from_format(&format)?;
                        if codec_type != VideoCodecType::H264 {
                            return None;
                        }
                        Some(VideoEncoder::new_with_handler(Box::new(
                            Openh264VideoEncoder::new(library.clone()),
                        )))
                    }
                },
            ),
        })
    }
}

impl VideoCodecCapability for Openh264VideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("openh264", "OpenH264")
    }

    fn get_supported_formats(&self, _direction: CodecDirection) -> Vec<SdpVideoFormat> {
        openh264_supported_formats()
    }

    fn create_video_encoder(
        &self,
        env: shiguredo_webrtc::EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        self.simulcast_capability_helper
            .create_video_encoder(env, format)
    }

    #[expect(unused_variables)]
    fn create_video_decoder(
        &self,
        env: shiguredo_webrtc::EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        let codec_type = codec_type_from_format(&format)?;
        if codec_type != VideoCodecType::H264 {
            return None;
        }
        Some(VideoDecoder::new_with_handler(Box::new(
            Openh264VideoDecoder::new(self.library.clone()),
        )))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::Environment;
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
                .create_video_encoder(
                    shiguredo_webrtc::Environment::new().as_ref(),
                    SdpVideoFormat::new("H264").as_ref(),
                )
                .is_some()
        );
        assert!(
            capability
                .create_video_decoder(
                    shiguredo_webrtc::Environment::new().as_ref(),
                    SdpVideoFormat::new("H264").as_ref(),
                )
                .is_some()
        );

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

        let resolved_with_profile_level_id = capability.resolve_sdp_format(
            CodecDirection::Encoder,
            SdpVideoFormat::new_with_parameters(
                "H264",
                &HashMap::from([(String::from("profile-level-id"), String::from("42e01f"))]),
                &[],
            )
            .as_ref(),
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

    #[test]
    fn openh264_create_video_encoder_uses_simulcast_adapter() {
        let Some(path) = openh264_path() else {
            println!("SKIP: OPENH264_PATH is not set");
            return;
        };
        let capability = Openh264VideoCodecCapability::new(path)
            .expect("Openh264VideoCodecCapability::new must succeed");
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
