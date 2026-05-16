use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use shiguredo_amf::{
    Av1EncoderConfig, CodecConfig, DecodedFrame, Decoder, DecoderCodec, DecoderConfig,
    EncodeOptions, EncodedFrame, Encoder, EncoderConfig, FnDecodeHandler, FnEncodeHandler,
    FrameFormat, H264EncoderConfig, HevcEncoderConfig, PictureType, RateControlMode,
    ReconfigureParams, VideoCodecType as AmfCodecType, ffi::AMF_PLANE_TYPE, frame_type,
    supported_codecs,
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
    i420_to_nv12, nv12_to_i420, rtc_log_error, rtc_log_warning,
};

use crate::error::Result;
use crate::video_codec::{
    AlignmentEncoderAdapter, SimulcastCapabilityHelper, codec_type_from_format,
};
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

/// AMF エンコーダーのコールバックハンドラに渡すユーザーデータ
struct AmfEncoderUserData {
    rtp_timestamp: u32,
    width: u32,
    height: u32,
}

/// AMF デコーダーのコールバックハンドラに渡すユーザーデータ
struct AmfDecoderUserData {
    rtp_timestamp: u32,
    timestamp_us: i64,
}

/// 複数スレッド間で共有する libwebrtc コールバックポインタ
type SharedEncoderCallback = Arc<Mutex<Option<VideoEncoderEncodedImageCallbackPtr>>>;
type SharedDecoderCallback = Arc<Mutex<Option<VideoDecoderDecodedImageCallbackPtr>>>;

fn collect_supported_formats() -> (Vec<SdpVideoFormat>, Vec<SdpVideoFormat>) {
    let mut encoder_supported_formats = Vec::new();
    let mut decoder_supported_formats = Vec::new();
    for info in supported_codecs() {
        let codec_type = match info.codec {
            AmfCodecType::H264 => VideoCodecType::H264,
            AmfCodecType::Hevc => VideoCodecType::H265,
            AmfCodecType::Av1 => VideoCodecType::Av1,
        };
        if info.encoding.supported {
            encoder_supported_formats.extend(supported_formats_for_codec(codec_type));
        }
        if info.decoding.supported {
            decoder_supported_formats.extend(supported_formats_for_codec(codec_type));
        }
    }
    (encoder_supported_formats, decoder_supported_formats)
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
        _ => Vec::new(),
    }
}

fn encoder_codec_config(codec_type: VideoCodecType) -> Option<CodecConfig> {
    match codec_type {
        VideoCodecType::H264 => Some(CodecConfig::H264(H264EncoderConfig { profile: None })),
        VideoCodecType::H265 => Some(CodecConfig::Hevc(HevcEncoderConfig { profile: None })),
        VideoCodecType::Av1 => Some(CodecConfig::Av1(Av1EncoderConfig { profile: None })),
        _ => None,
    }
}

fn decoder_codec(codec_type: VideoCodecType) -> Option<DecoderCodec> {
    match codec_type {
        VideoCodecType::H264 => Some(DecoderCodec::H264),
        VideoCodecType::H265 => Some(DecoderCodec::Hevc),
        VideoCodecType::Av1 => Some(DecoderCodec::Av1),
        _ => None,
    }
}

fn requested_frame_type(
    frame_types: Option<VideoFrameTypeVectorRef<'_>>,
) -> Option<VideoFrameType> {
    frame_types.and_then(|frame_types| frame_types.get(0))
}

fn amf_force_frame_type(requested: Option<VideoFrameType>) -> u16 {
    if requested == Some(VideoFrameType::Key) {
        frame_type::IDR | frame_type::I | frame_type::REF
    } else {
        frame_type::UNKNOWN
    }
}

fn frame_type_from_amf(picture_type: PictureType) -> VideoFrameType {
    match picture_type {
        PictureType::Idr => VideoFrameType::Key,
        PictureType::I | PictureType::P | PictureType::B | PictureType::Unknown => {
            VideoFrameType::Delta
        }
    }
}

fn target_kbps_from_bps(target_bitrate_bps: u32) -> u32 {
    (target_bitrate_bps.max(1) as u64).div_ceil(1000) as u32
}

fn amf_reconfigure_params(target_bitrate_bps: u32, framerate: u32) -> ReconfigureParams {
    ReconfigureParams {
        framerate_num: Some(framerate.max(1)),
        framerate_den: Some(1),
        target_kbps: Some(target_kbps_from_bps(target_bitrate_bps)),
        ..ReconfigureParams::default()
    }
}

/// AMF Plane の `hpitch` / `vpitch` (i32) からスライス長 (usize) を安全に計算する
///
/// AMF API は i32 を返すため負値や 0、`usize` 乗算オーバーフローが理論上ありうる。
/// `from_raw_parts_mut` の長さに負値由来の `usize::MAX` を渡すと UB なので、
/// `u32::try_from` で正値化したうえで `checked_mul` する。
fn plane_buffer_size(hpitch: i32, vpitch: i32) -> Option<usize> {
    let hpitch = u32::try_from(hpitch).ok()?;
    let vpitch = u32::try_from(vpitch).ok()?;
    if hpitch == 0 || vpitch == 0 {
        return None;
    }
    (hpitch as usize).checked_mul(vpitch as usize)
}

/// AMF Surface の Y/UV プレーンへ I420 フレームを NV12 として書き込む
fn write_i420_into_amf_surface(
    surface: &shiguredo_amf::amf::Surface,
    i420: &I420Buffer,
    frame_width: i32,
    frame_height: i32,
) -> std::result::Result<(), shiguredo_amf::Error> {
    let y_plane = surface.get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)?;
    let uv_plane = surface.get_plane(AMF_PLANE_TYPE::AMF_PLANE_UV)?;

    let y_ptr = y_plane.get_native() as *mut u8;
    let uv_ptr = uv_plane.get_native() as *mut u8;
    if y_ptr.is_null() || uv_ptr.is_null() {
        return Err(shiguredo_amf::Error::new_custom(
            "write_i420_into_amf_surface",
            "plane native pointer is null",
        ));
    }

    let y_hpitch = y_plane.get_hpitch();
    let y_vpitch = y_plane.get_vpitch();
    let uv_hpitch = uv_plane.get_hpitch();
    let uv_vpitch = uv_plane.get_vpitch();

    // 負値や乗算オーバーフローでスライス長が壊れると `from_raw_parts_mut` が UB になるため、
    // u32 で検証してから usize 乗算する
    let y_size = plane_buffer_size(y_hpitch, y_vpitch).ok_or_else(|| {
        shiguredo_amf::Error::new_custom("write_i420_into_amf_surface", "Y plane pitch is invalid")
    })?;
    let uv_size = plane_buffer_size(uv_hpitch, uv_vpitch).ok_or_else(|| {
        shiguredo_amf::Error::new_custom("write_i420_into_amf_surface", "UV plane pitch is invalid")
    })?;

    let dst_y = unsafe { std::slice::from_raw_parts_mut(y_ptr, y_size) };
    let dst_uv = unsafe { std::slice::from_raw_parts_mut(uv_ptr, uv_size) };

    if !i420_to_nv12(
        i420.y_data(),
        i420.stride_y(),
        i420.u_data(),
        i420.stride_u(),
        i420.v_data(),
        i420.stride_v(),
        dst_y,
        y_hpitch,
        dst_uv,
        uv_hpitch,
        frame_width,
        frame_height,
    ) {
        return Err(shiguredo_amf::Error::new_custom(
            "write_i420_into_amf_surface",
            "i420_to_nv12 conversion failed",
        ));
    }
    Ok(())
}

/// AMF の `EncodedFrame` を libwebrtc コールバックへ転送する
fn dispatch_encoded_frame(
    callback_state: &SharedEncoderCallback,
    codec_type: VideoCodecType,
    frame: EncodedFrame<AmfEncoderUserData>,
) {
    let callback = {
        let guard = callback_state.lock().unwrap();
        *guard
    };
    let Some(callback) = callback else {
        return;
    };

    let buffer = frame.buffer();
    let buf_size = buffer.get_size();
    let buf_native = buffer.get_native() as *const u8;
    if buf_native.is_null() || buf_size == 0 {
        rtc_log_warning!("AMF encoded buffer is empty; skipping frame");
        return;
    }
    let bitstream = unsafe { std::slice::from_raw_parts(buf_native, buf_size) };

    let picture_type = frame.picture_type();
    let user_data = frame.user_data();

    let mut encoded_image = EncodedImage::new();
    let encoded_buffer = EncodedImageBuffer::from_bytes(bitstream);
    encoded_image.set_encoded_data(&encoded_buffer);
    encoded_image.set_rtp_timestamp(user_data.rtp_timestamp);
    encoded_image.set_encoded_width(user_data.width);
    encoded_image.set_encoded_height(user_data.height);
    encoded_image.set_frame_type(frame_type_from_amf(picture_type));

    let mut codec_specific_info = CodecSpecificInfo::new();
    codec_specific_info.set_codec_type(codec_type);
    if codec_type == VideoCodecType::H264 {
        codec_specific_info.set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
        codec_specific_info.set_h264_idr_frame(matches!(picture_type, PictureType::Idr));
    }

    let result = unsafe {
        callback.on_encoded_image(encoded_image.as_ref(), Some(codec_specific_info.as_ref()))
    };
    if result.error() != VideoEncoderEncodedImageCallbackResultError::Ok {
        rtc_log_warning!(
            "AMF: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
        );
    }
}

/// AMF の `DecodedFrame` を libwebrtc コールバックへ転送する
fn dispatch_decoded_frame(
    callback_state: &SharedDecoderCallback,
    frame: DecodedFrame<AmfDecoderUserData>,
) {
    let (surface, user_data) = frame.into_parts();

    let y_plane = match surface.get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y) {
        Ok(p) => p,
        Err(err) => {
            rtc_log_error!("AMF decoded surface Y plane fetch failed: {}", err);
            return;
        }
    };
    let uv_plane = match surface.get_plane(AMF_PLANE_TYPE::AMF_PLANE_UV) {
        Ok(p) => p,
        Err(err) => {
            rtc_log_error!("AMF decoded surface UV plane fetch failed: {}", err);
            return;
        }
    };

    let width_i32 = y_plane.get_width();
    let height_i32 = y_plane.get_height();
    if width_i32 <= 0 || height_i32 <= 0 {
        rtc_log_error!(
            "AMF decoded plane reports invalid size: {}x{}",
            width_i32,
            height_i32
        );
        return;
    }

    let y_ptr = y_plane.get_native() as *const u8;
    let uv_ptr = uv_plane.get_native() as *const u8;
    if y_ptr.is_null() || uv_ptr.is_null() {
        rtc_log_error!("AMF decoded plane native pointer is null");
        return;
    }
    let y_hpitch = y_plane.get_hpitch();
    let uv_hpitch = uv_plane.get_hpitch();
    let y_vpitch = y_plane.get_vpitch();
    let uv_vpitch = uv_plane.get_vpitch();

    // 負値や乗算オーバーフローでスライス長が壊れると `from_raw_parts` が UB になるため、
    // u32 で検証してから usize 乗算する
    let Some(src_y_len) = plane_buffer_size(y_hpitch, y_vpitch) else {
        rtc_log_error!(
            "AMF decoded Y plane pitch is invalid: hpitch={}, vpitch={}",
            y_hpitch,
            y_vpitch
        );
        return;
    };
    let Some(src_uv_len) = plane_buffer_size(uv_hpitch, uv_vpitch) else {
        rtc_log_error!(
            "AMF decoded UV plane pitch is invalid: hpitch={}, vpitch={}",
            uv_hpitch,
            uv_vpitch
        );
        return;
    };
    let src_y = unsafe { std::slice::from_raw_parts(y_ptr, src_y_len) };
    let src_uv = unsafe { std::slice::from_raw_parts(uv_ptr, src_uv_len) };

    let mut i420 = I420Buffer::new(width_i32, height_i32);
    let dst_stride_y = i420.stride_y();
    let dst_stride_u = i420.stride_u();
    let dst_stride_v = i420.stride_v();
    let (dst_y, dst_u, dst_v) = i420.planes_mut();
    if !nv12_to_i420(
        src_y,
        y_hpitch,
        src_uv,
        uv_hpitch,
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width_i32,
        height_i32,
    ) {
        rtc_log_error!("AMF nv12_to_i420 conversion failed");
        return;
    }

    let decoded_image = VideoFrame::builder(&i420.cast_to_video_frame_buffer())
        .set_timestamp_us(user_data.timestamp_us)
        .set_rtp_timestamp(user_data.rtp_timestamp)
        .build();

    let callback = {
        let guard = callback_state.lock().unwrap();
        *guard
    };
    let Some(callback) = callback else {
        return;
    };
    unsafe {
        callback.decoded(decoded_image.as_ref());
    }
}

type AmfEncoderInstance = Encoder<FnEncodeHandler<AmfEncoderUserData>>;
type AmfDecoderInstance = Decoder<FnDecodeHandler<AmfDecoderUserData>>;

struct AmfVideoEncoder {
    callback: SharedEncoderCallback,
    encoder: Option<AmfEncoderInstance>,
    codec_type: VideoCodecType,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    rebuild_needed: bool,
    reconfigure_needed: bool,
}

impl AmfVideoEncoder {
    fn new(codec_type: VideoCodecType) -> Self {
        Self {
            callback: Arc::new(Mutex::new(None)),
            encoder: None,
            codec_type,
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
            return Err(crate::Error::AmfMessage {
                reason: "AMF encoder requires non-zero width and height".to_string(),
            });
        }
        let Some(codec_config) = encoder_codec_config(self.codec_type) else {
            return Err(crate::Error::AmfMessage {
                reason: "AMF encoder codec type is not supported".to_string(),
            });
        };

        let mut config = EncoderConfig::new(
            codec_config,
            self.width,
            self.height,
            FrameFormat::Nv12,
            self.framerate.max(1),
            1,
            RateControlMode::Cbr,
        );
        config.target_kbps = Some(target_kbps_from_bps(self.target_bitrate_bps));

        // 既存のエンコーダがあれば、新規生成前に確実に終了させる
        self.encoder = None;

        let callback_state = self.callback.clone();
        let codec_type = self.codec_type;
        let handler = FnEncodeHandler::new(move |result| match result {
            Ok(frame) => dispatch_encoded_frame(&callback_state, codec_type, frame),
            Err(err) => {
                rtc_log_error!("AMF encode worker error for {:?}: {}", codec_type, err);
            }
        });

        let encoder = Encoder::new(config, handler)?;
        self.encoder = Some(encoder);
        self.rebuild_needed = false;
        self.reconfigure_needed = false;
        Ok(())
    }

    fn reconfigure_encoder(&mut self) -> Result<()> {
        let Some(encoder) = self.encoder.as_mut() else {
            return Err(crate::Error::AmfMessage {
                reason: "AMF encoder instance is not initialized".to_string(),
            });
        };
        match encoder.reconfigure(amf_reconfigure_params(
            self.target_bitrate_bps,
            self.framerate,
        )) {
            Ok(()) => {
                self.reconfigure_needed = false;
                Ok(())
            }
            Err(err) => {
                rtc_log_warning!(
                    "AMF encoder reconfigure failed for {:?}: {}",
                    self.codec_type,
                    err
                );
                Err(err.into())
            }
        }
    }
}

impl VideoEncoderHandler for AmfVideoEncoder {
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
        if self.callback.lock().unwrap().is_none() {
            return VideoCodecStatus::Uninitialized;
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
            // 解像度変更は再初期化が必要
            self.rebuild_needed = true;
        }
        if self.encoder.is_none() {
            self.rebuild_needed = true;
        }

        if self.rebuild_needed {
            if self.rebuild_encoder().is_err() {
                return VideoCodecStatus::Error;
            }
        } else if self.reconfigure_needed && self.reconfigure_encoder().is_err() {
            rtc_log_warning!(
                "AMF encoder reconfigure failed for {:?}; falling back to rebuild",
                self.codec_type
            );
            self.rebuild_needed = true;
            if self.rebuild_encoder().is_err() {
                return VideoCodecStatus::Error;
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

        let encoder = self.encoder.as_mut().expect("encoder should exist");
        let surface = match encoder.alloc_surface() {
            Ok(s) => s,
            Err(err) => {
                rtc_log_error!(
                    "AMF alloc_surface failed for {:?}: {}",
                    self.codec_type,
                    err
                );
                return VideoCodecStatus::Error;
            }
        };

        if let Err(err) =
            write_i420_into_amf_surface(&surface, &i420, frame_width_i32, frame_height_i32)
        {
            rtc_log_error!(
                "AMF surface write failed for {:?}: {}",
                self.codec_type,
                err
            );
            return VideoCodecStatus::Error;
        }

        let options = EncodeOptions {
            frame_type: amf_force_frame_type(requested_frame_type),
        };
        let user_data = AmfEncoderUserData {
            rtp_timestamp: frame.rtp_timestamp(),
            width: frame_width,
            height: frame_height,
        };

        if let Err(err) = encoder.encode(surface, &options, user_data) {
            rtc_log_error!("AMF encode failed for {:?}: {}", self.codec_type, err);
            return VideoCodecStatus::Error;
        }

        VideoCodecStatus::Ok
    }

    fn register_encode_complete_callback(
        &mut self,
        callback: Option<VideoEncoderEncodedImageCallbackRef<'_>>,
    ) -> VideoCodecStatus {
        let mut guard = self.callback.lock().unwrap();
        *guard = callback
            .map(|callback| unsafe { VideoEncoderEncodedImageCallbackPtr::from_ref(callback) });
        VideoCodecStatus::Ok
    }

    fn release(&mut self) -> VideoCodecStatus {
        // libwebrtc 側に解放後の callback を触らせないため、先に callback をクリアする
        *self.callback.lock().unwrap() = None;
        // Encoder::Drop が worker スレッドを join するためここで完了する
        self.encoder = None;
        self.rebuild_needed = false;
        self.reconfigure_needed = false;
        VideoCodecStatus::Ok
    }

    fn set_rates(&mut self, parameters: VideoEncoderRateControlParametersRef<'_>) {
        self.framerate = parameters.framerate_fps().max(1.0) as u32;
        self.target_bitrate_bps = parameters
            .bitrate_sum_bps()
            .max(parameters.target_bitrate_sum_bps())
            .max(1);
        self.reconfigure_needed = true;
    }

    fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
        let mut info = VideoEncoderEncoderInfo::new();
        info.set_implementation_name("AMF");
        info.set_is_hardware_accelerated(true);
        info
    }
}

struct AmfVideoDecoder {
    callback: SharedDecoderCallback,
    decoder: Option<AmfDecoderInstance>,
    codec_type: VideoCodecType,
}

impl AmfVideoDecoder {
    fn new(codec_type: VideoCodecType) -> Self {
        Self {
            callback: Arc::new(Mutex::new(None)),
            decoder: None,
            codec_type,
        }
    }

    fn ensure_decoder(&mut self) -> Result<()> {
        if self.decoder.is_some() {
            return Ok(());
        }
        let Some(codec) = decoder_codec(self.codec_type) else {
            return Err(crate::Error::AmfMessage {
                reason: "AMF decoder codec type is not supported".to_string(),
            });
        };
        let callback_state = self.callback.clone();
        let handler = FnDecodeHandler::new(move |result| match result {
            Ok(frame) => dispatch_decoded_frame(&callback_state, frame),
            Err(err) => {
                rtc_log_error!("AMF decode worker error: {}", err);
            }
        });
        let decoder = Decoder::new(DecoderConfig { codec }, handler)?;
        self.decoder = Some(decoder);
        Ok(())
    }
}

impl VideoDecoderHandler for AmfVideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != self.codec_type {
            return false;
        }
        // 既存デコーダーがあれば一度破棄してから作り直す
        self.decoder = None;
        self.ensure_decoder().is_ok()
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
        let bitstream = encoded_data.data();
        if bitstream.is_empty() {
            return VideoCodecStatus::ErrParameter;
        }

        let decoder = self.decoder.as_mut().expect("decoder should exist");

        let buffer = match decoder.alloc_buffer(bitstream.len()) {
            Ok(b) => b,
            Err(err) => {
                rtc_log_error!("AMF alloc_buffer failed: {}", err);
                return VideoCodecStatus::Error;
            }
        };

        let dst_ptr = buffer.get_native() as *mut u8;
        if dst_ptr.is_null() {
            rtc_log_error!("AMF decoder input buffer native pointer is null");
            return VideoCodecStatus::Error;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bitstream.as_ptr(), dst_ptr, bitstream.len());
        }

        let user_data = AmfDecoderUserData {
            rtp_timestamp: input_image.rtp_timestamp(),
            timestamp_us: render_time_ms.saturating_mul(1000),
        };

        if let Err(err) = decoder.decode(buffer, user_data) {
            rtc_log_error!("AMF decode failed: {}", err);
            return VideoCodecStatus::Error;
        }
        VideoCodecStatus::Ok
    }

    fn register_decode_complete_callback(
        &mut self,
        callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    ) -> VideoCodecStatus {
        let mut guard = self.callback.lock().unwrap();
        *guard = callback;
        VideoCodecStatus::Ok
    }

    fn release(&mut self) -> VideoCodecStatus {
        *self.callback.lock().unwrap() = None;
        self.decoder = None;
        VideoCodecStatus::Ok
    }

    fn get_decoder_info(&mut self) -> VideoDecoderDecoderInfo {
        let mut info = VideoDecoderDecoderInfo::new();
        info.set_implementation_name("AMF");
        info.set_is_hardware_accelerated(true);
        info
    }
}

pub struct AmfVideoCodecCapability {
    encoder_supported_formats: Vec<SdpVideoFormat>,
    decoder_supported_formats: Vec<SdpVideoFormat>,
    simulcast_capability_helper: SimulcastCapabilityHelper,
}

impl AmfVideoCodecCapability {
    pub fn new() -> Result<Self> {
        let (encoder_supported_formats, decoder_supported_formats) = collect_supported_formats();
        if encoder_supported_formats.is_empty() && decoder_supported_formats.is_empty() {
            return Err(crate::Error::InvalidVideoCodecCapability {
                reason: "AMF does not support any encoder or decoder codec".to_string(),
            });
        }
        Self::new_with_formats(encoder_supported_formats, decoder_supported_formats)
    }

    fn new_with_formats(
        encoder_supported_formats: Vec<SdpVideoFormat>,
        decoder_supported_formats: Vec<SdpVideoFormat>,
    ) -> Result<Self> {
        let encoder_supported_formats_for_factory = encoder_supported_formats.clone();

        let simulcast_capability_helper = SimulcastCapabilityHelper::new_with_builder(
            move || encoder_supported_formats_for_factory.clone(),
            {
                move |_env, format| {
                    let codec_type = codec_type_from_format(&format)?;
                    let encoder =
                        VideoEncoder::new_with_handler(Box::new(AmfVideoEncoder::new(codec_type)));
                    if codec_type == VideoCodecType::Av1 {
                        // AMF AV1 エンコーダーは 64x16 のアライメント制約があるため、アダプターを噛ませて対応する。
                        //
                        // AMF AV1 では `AMF_VIDEO_ENCODER_AV1_ALIGNMENT_MODE` が定義されており、
                        // `64X16_ONLY` / `64X16_1080P_CODED_1082` /
                        // `NO_RESTRICTIONS` / `8X2_ONLY` のモードが存在する。
                        // デフォルトでは `64X16_ONLY` が選ばれ、その場合は
                        // 64x16 に揃っていない解像度を渡すと `AMF_NOT_SUPPORTED` になる
                        // （例: 320x180 は高さ 180 が 16 の倍数でないため失敗）。
                        //
                        // `NO_RESTRICTIONS` モードであればエンコード時にアライメント制約は存在しないが、
                        // 出力される映像が 64x16 に align up されて出力されてしまう。
                        //
                        // `shiguredo_amf` の公開 API では `Av1AlignmentMode` を切り替える
                        // 設定項目が露出していないため SDK 側でアダプターを噛ませて吸収する。
                        Some(VideoEncoder::new_with_handler(Box::new(
                            AlignmentEncoderAdapter::new(encoder, VideoCodecType::Av1, 64, 16),
                        )))
                    } else {
                        Some(encoder)
                    }
                }
            },
        );

        Ok(Self {
            encoder_supported_formats,
            decoder_supported_formats,
            simulcast_capability_helper,
        })
    }
}

impl VideoCodecCapability for AmfVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("amf", "AMD AMF")
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
            AmfVideoDecoder::new(codec_type),
        )))
    }
}

#[cfg(test)]
impl AmfVideoCodecCapability {
    fn new_for_test(
        encoder_supported_formats: Vec<SdpVideoFormat>,
        decoder_supported_formats: Vec<SdpVideoFormat>,
    ) -> Result<Self> {
        Self::new_with_formats(encoder_supported_formats, decoder_supported_formats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::{Environment, SdpVideoFormat, VideoFrameType, VideoFrameTypeVector};

    fn test_supported_formats(codec_types: &[VideoCodecType]) -> Vec<SdpVideoFormat> {
        let mut supported_formats = Vec::new();
        for codec_type in codec_types {
            supported_formats.extend(supported_formats_for_codec(*codec_type));
        }
        supported_formats
    }

    #[test]
    fn amf_capability_has_expected_implementation_name() {
        let capability = AmfVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
        )
        .expect("AmfVideoCodecCapability の生成に失敗");
        assert_eq!(capability.get_implementation().name(), "amf");
    }

    #[test]
    fn amf_capability_supports_formats_per_direction() {
        let capability = AmfVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264, VideoCodecType::Av1]),
            test_supported_formats(&[VideoCodecType::H264, VideoCodecType::H265]),
        )
        .expect("AmfVideoCodecCapability の生成に失敗");

        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));
        assert!(!capability.is_supported(CodecDirection::Encoder, VideoCodecType::H265));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H265));
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::Av1));
        assert!(!capability.is_supported(CodecDirection::Decoder, VideoCodecType::Av1));

        let encoder_formats = capability
            .get_supported_formats(CodecDirection::Encoder)
            .into_iter()
            .map(|format| format.name().expect("format name の取得に失敗"))
            .collect::<Vec<_>>();
        assert_eq!(encoder_formats, vec!["H264", "AV1"]);

        let decoder_formats = capability
            .get_supported_formats(CodecDirection::Decoder)
            .into_iter()
            .map(|format| format.name().expect("format name の取得に失敗"))
            .collect::<Vec<_>>();
        assert_eq!(decoder_formats, vec!["H264", "H265"]);

        let resolved = capability
            .resolve_sdp_format(
                CodecDirection::Encoder,
                SdpVideoFormat::new("H264").as_ref(),
            )
            .expect("H264 フォーマットの解決に失敗");
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
    fn amf_capability_create_video_encoder_uses_simulcast_adapter() {
        let capability = AmfVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
        )
        .expect("AmfVideoCodecCapability の生成に失敗");

        let env = Environment::new();
        let format = SdpVideoFormat::new("H264");
        let encoder = capability
            .create_video_encoder(env.as_ref(), format.as_ref())
            .expect("対応フォーマットで encoder の生成に失敗");
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
    fn amf_requested_frame_type_uses_first_entry() {
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
    fn amf_reconfigure_params_uses_rate_values() {
        let params = amf_reconfigure_params(1_234_567, 24);
        assert_eq!(params.framerate_num, Some(24));
        assert_eq!(params.framerate_den, Some(1));
        assert_eq!(params.target_kbps, Some(1235));
        assert_eq!(params.max_kbps, None);
        assert_eq!(params.qpi, None);
        assert_eq!(params.qpp, None);
        assert_eq!(params.qpb, None);
        assert_eq!(params.gop_pic_size, None);
    }

    #[test]
    fn amf_reconfigure_params_clamps_zero_to_one() {
        let params = amf_reconfigure_params(0, 0);
        assert_eq!(params.framerate_num, Some(1));
        assert_eq!(params.framerate_den, Some(1));
        assert_eq!(params.target_kbps, Some(1));
    }

    #[test]
    fn amf_frame_type_mapping_matches_idr_only_keyframe() {
        assert_eq!(frame_type_from_amf(PictureType::Idr), VideoFrameType::Key);
        assert_eq!(frame_type_from_amf(PictureType::I), VideoFrameType::Delta);
        assert_eq!(frame_type_from_amf(PictureType::P), VideoFrameType::Delta);
    }
}
