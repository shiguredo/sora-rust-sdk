use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use shiguredo_vpl::supported_codecs;
use shiguredo_vpl::{
    AdapterSelector, Av1EncoderConfig, CodecConfig, DecodedFrame, Decoder, DecoderCodec,
    DecoderConfig, EncodeOptions, EncodedFrame, Encoder, EncoderConfig, FnDecodeHandler,
    FnEncodeHandler, FrameFormat, H264EncoderConfig, HevcEncoderConfig, PictureType,
    RateControlMode, ReconfigureParams, VideoCodecType as VplCodecType, Vp9EncoderConfig,
    Vp9Profile, frame_type, list_adapters,
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
use crate::video_codec::{SimulcastCapabilityHelper, codec_type_from_format};
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

/// 非 Linux 環境向け `supported_codecs` スタブ
///
/// `shiguredo_vpl::supported_codecs` は Linux 専用関数のため、それ以外の
/// プラットフォームでは常に空のリストを返す。
#[cfg(not(target_os = "linux"))]
fn supported_codecs(
    _adapter: AdapterSelector,
) -> std::result::Result<Vec<shiguredo_vpl::CodecInfo>, shiguredo_vpl::Error> {
    Ok(Vec::new())
}

/// VPL エンコーダーのコールバックハンドラに渡すユーザーデータ
struct VplEncoderUserData {
    rtp_timestamp: u32,
    width: u32,
    height: u32,
}

/// VPL デコーダーのコールバックハンドラに渡すユーザーデータ
struct VplDecoderUserData {
    rtp_timestamp: u32,
    timestamp_us: i64,
}

type SharedEncoderCallback = Arc<Mutex<Option<VideoEncoderEncodedImageCallbackPtr>>>;
type SharedDecoderCallback = Arc<Mutex<Option<VideoDecoderDecodedImageCallbackPtr>>>;

/// 利用可能な Intel HW アダプターから先頭のものを選択する
fn default_adapter() -> Result<AdapterSelector> {
    let adapters = list_adapters()?;
    let adapter = adapters
        .into_iter()
        .next()
        .ok_or_else(|| crate::Error::VplMessage {
            reason: "no Intel VPL adapter found".to_string(),
        })?;
    Ok(AdapterSelector::DrmRenderNode(adapter.drm_render_node))
}

fn collect_supported_formats(
    adapter: AdapterSelector,
) -> Result<(Vec<SdpVideoFormat>, Vec<SdpVideoFormat>)> {
    let codec_infos = supported_codecs(adapter)?;

    let mut encoder_supported_formats = Vec::new();
    let mut decoder_supported_formats = Vec::new();
    for info in codec_infos {
        let codec_type = match info.codec {
            VplCodecType::H264 => VideoCodecType::H264,
            VplCodecType::Hevc => VideoCodecType::H265,
            VplCodecType::Vp9 => VideoCodecType::Vp9,
            VplCodecType::Av1 => VideoCodecType::Av1,
        };
        if info.encoding.supported {
            encoder_supported_formats.extend(supported_formats_for_codec(codec_type));
        }
        if info.decoding.supported {
            decoder_supported_formats.extend(supported_formats_for_codec(codec_type));
        }
    }
    Ok((encoder_supported_formats, decoder_supported_formats))
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
        VideoCodecType::Vp9 => vec![SdpVideoFormat::new_with_parameters(
            "VP9",
            &HashMap::from([(String::from("profile-id"), String::from("0"))]),
            &[],
        )],
        VideoCodecType::Av1 => vec![SdpVideoFormat::new("AV1")],
        _ => Vec::new(),
    }
}

fn encoder_codec_config(codec_type: VideoCodecType) -> Option<CodecConfig> {
    match codec_type {
        VideoCodecType::H264 => Some(CodecConfig::H264(H264EncoderConfig { profile: None })),
        VideoCodecType::H265 => Some(CodecConfig::Hevc(HevcEncoderConfig { profile: None })),
        VideoCodecType::Vp9 => Some(CodecConfig::Vp9(Vp9EncoderConfig {
            profile: Some(Vp9Profile::Profile0),
        })),
        VideoCodecType::Av1 => Some(CodecConfig::Av1(Av1EncoderConfig { profile: None })),
        _ => None,
    }
}

fn decoder_codec(codec_type: VideoCodecType) -> Option<DecoderCodec> {
    match codec_type {
        VideoCodecType::H264 => Some(DecoderCodec::H264),
        VideoCodecType::H265 => Some(DecoderCodec::Hevc),
        VideoCodecType::Vp9 => Some(DecoderCodec::Vp9),
        VideoCodecType::Av1 => Some(DecoderCodec::Av1),
        _ => None,
    }
}

fn requested_frame_type(
    frame_types: Option<VideoFrameTypeVectorRef<'_>>,
) -> Option<VideoFrameType> {
    frame_types.and_then(|frame_types| frame_types.get(0))
}

fn vpl_force_frame_type(codec_type: VideoCodecType, requested: Option<VideoFrameType>) -> u16 {
    if requested == Some(VideoFrameType::Key) {
        if codec_type == VideoCodecType::Vp9 {
            // VP9 は I/P を前提にしているため、key 要求時は I のみを明示する。
            frame_type::I
        } else {
            frame_type::IDR | frame_type::I | frame_type::REF
        }
    } else {
        frame_type::UNKNOWN
    }
}

fn vp9_payload_from_vpl(data: &[u8]) -> std::result::Result<&[u8], &'static str> {
    let mut payload = data;
    if payload.starts_with(b"DKIF") {
        // VPL 実装によっては IVF ファイルヘッダー + フレームヘッダー付きで返るため除去する。
        if payload.len() < 32 {
            return Err("VP9 IVF file header is truncated");
        }
        payload = &payload[32..];
    }
    if payload.len() < 12 {
        return Err("VP9 IVF frame header is truncated");
    }
    payload = &payload[12..];
    if payload.is_empty() {
        return Err("VP9 payload is empty after stripping IVF headers");
    }
    Ok(payload)
}

fn frame_type_from_vpl(picture_type: PictureType) -> VideoFrameType {
    match picture_type {
        PictureType::Idr | PictureType::I => VideoFrameType::Key,
        PictureType::P | PictureType::B | PictureType::Unknown => VideoFrameType::Delta,
    }
}

fn target_kbps_from_bps(target_bitrate_bps: u32) -> u16 {
    let target_kbps = (target_bitrate_bps.max(1) as u64).div_ceil(1000);
    u16::try_from(target_kbps).unwrap_or(u16::MAX)
}

fn vpl_rate_control_mode(codec_type: VideoCodecType) -> RateControlMode {
    if codec_type == VideoCodecType::Vp9 {
        // VP9 は環境依存で CBR 初期化/実行失敗が出るため VBR を採用する。
        RateControlMode::Vbr
    } else {
        RateControlMode::Cbr
    }
}

fn dispatch_encoded_frame(
    callback_state: &SharedEncoderCallback,
    codec_type: VideoCodecType,
    frame: EncodedFrame<VplEncoderUserData>,
) {
    let callback = {
        let guard = callback_state.lock().unwrap();
        *guard
    };
    let Some(callback) = callback else {
        return;
    };

    let picture_type = frame.picture_type();
    let user_data = frame.user_data();

    let bitstream = frame.data();
    let encoded_payload = if codec_type == VideoCodecType::Vp9 {
        match vp9_payload_from_vpl(bitstream) {
            Ok(payload) => payload,
            Err(err) => {
                rtc_log_error!("VPL VP9 payload normalization failed: {}", err);
                return;
            }
        }
    } else {
        bitstream
    };

    let mut encoded_image = EncodedImage::new();
    let encoded_buffer = EncodedImageBuffer::from_bytes(encoded_payload);
    encoded_image.set_encoded_data(&encoded_buffer);
    encoded_image.set_rtp_timestamp(user_data.rtp_timestamp);
    encoded_image.set_encoded_width(user_data.width);
    encoded_image.set_encoded_height(user_data.height);
    let output_frame_type = frame_type_from_vpl(picture_type);
    encoded_image.set_frame_type(output_frame_type);

    let mut codec_specific_info = CodecSpecificInfo::new();
    codec_specific_info.set_codec_type(codec_type);
    if codec_type == VideoCodecType::Vp9 {
        let is_key = output_frame_type == VideoFrameType::Key;
        // VP9 では end_of_picture を明示しないと受信側で codec 判定が外れるケースがある。
        codec_specific_info.set_end_of_picture(true);
        // num_spatial_layers を設定しないと first_active_layer / num_spatial_layers が不整合でエラーになる
        codec_specific_info.set_vp9_num_spatial_layers(1);
        codec_specific_info.set_vp9_temporal_idx(shiguredo_webrtc::no_temporal_idx().into());
        // 実機で動作確認したところ、以下の設定は必須ではないことが分かっている。
        // ただ他の実機や libwebrtc の仕様変更のことを考えると、
        // 一応明示しておいた方が安定しそうなので設定しておく。
        codec_specific_info.set_vp9_first_frame_in_picture(true);
        codec_specific_info.set_vp9_spatial_layer_resolution_present(false);
        codec_specific_info.set_vp9_ss_data_available(false);
        codec_specific_info.set_vp9_temporal_up_switch(true);
        codec_specific_info.set_vp9_inter_pic_predicted(!is_key);
        codec_specific_info.set_vp9_flexible_mode(false);
        codec_specific_info.set_vp9_inter_layer_predicted(false);
    }
    if codec_type == VideoCodecType::H264 {
        codec_specific_info.set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
        codec_specific_info.set_h264_idr_frame(matches!(picture_type, PictureType::Idr));
    }

    let result = unsafe {
        callback.on_encoded_image(encoded_image.as_ref(), Some(codec_specific_info.as_ref()))
    };
    if result.error() != VideoEncoderEncodedImageCallbackResultError::Ok {
        rtc_log_warning!(
            "VPL: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
        );
    }
}

fn dispatch_decoded_frame(
    callback_state: &SharedDecoderCallback,
    frame: DecodedFrame<'_, VplDecoderUserData>,
) {
    let width = frame.width();
    let height = frame.height();
    let pitch = frame.pitch();
    let src_y = frame.y();
    let src_uv = frame.uv();
    let user_data = frame.user_data();

    let width_i32 = match i32::try_from(width) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!("VPL decoded width overflows i32: {}", width);
            return;
        }
    };
    let height_i32 = match i32::try_from(height) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!("VPL decoded height overflows i32: {}", height);
            return;
        }
    };
    let pitch_i32 = match i32::try_from(pitch) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!("VPL decoded pitch overflows i32: {}", pitch);
            return;
        }
    };

    let mut i420 = I420Buffer::new(width_i32, height_i32);
    let dst_stride_y = i420.stride_y();
    let dst_stride_u = i420.stride_u();
    let dst_stride_v = i420.stride_v();
    let (dst_y, dst_u, dst_v) = i420.planes_mut();
    if !nv12_to_i420(
        src_y,
        pitch_i32,
        src_uv,
        pitch_i32,
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width_i32,
        height_i32,
    ) {
        rtc_log_error!("VPL nv12_to_i420 conversion failed");
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

type VplEncoderInstance = Encoder<FnEncodeHandler<VplEncoderUserData>>;
type VplDecoderInstance = Decoder<FnDecodeHandler<VplDecoderUserData>>;

struct VplVideoEncoder {
    adapter: AdapterSelector,
    callback: SharedEncoderCallback,
    encoder: Option<VplEncoderInstance>,
    codec_type: VideoCodecType,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    rebuild_needed: bool,
    reconfigure_needed: bool,
    /// 最後に作成したエンコーダーの coded_size をキャッシュする
    coded_size: (usize, usize),
}

impl VplVideoEncoder {
    fn new(adapter: AdapterSelector, codec_type: VideoCodecType) -> Self {
        Self {
            adapter,
            callback: Arc::new(Mutex::new(None)),
            encoder: None,
            codec_type,
            width: 0,
            height: 0,
            framerate: 30,
            target_bitrate_bps: 500_000,
            rebuild_needed: false,
            reconfigure_needed: false,
            coded_size: (0, 0),
        }
    }

    fn rebuild_encoder(&mut self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(crate::Error::VplMessage {
                reason: "VPL encoder requires non-zero width and height".to_string(),
            });
        }
        let Some(codec_config) = encoder_codec_config(self.codec_type) else {
            return Err(crate::Error::VplMessage {
                reason: "VPL encoder codec type is not supported".to_string(),
            });
        };

        let mut config = EncoderConfig::new(
            self.adapter,
            codec_config,
            self.width,
            self.height,
            FrameFormat::Nv12,
            self.framerate.max(1),
            1,
            vpl_rate_control_mode(self.codec_type),
        );
        config.target_kbps = Some(target_kbps_from_bps(self.target_bitrate_bps));

        // 既存のエンコーダーがあれば、新規生成前に確実に終了させる
        self.encoder = None;

        let callback_state = self.callback.clone();
        let codec_type = self.codec_type;
        let handler = FnEncodeHandler::new(move |result| match result {
            Ok(frame) => dispatch_encoded_frame(&callback_state, codec_type, frame),
            Err(err) => {
                rtc_log_error!("VPL encode worker error for {:?}: {}", codec_type, err);
            }
        });

        let encoder = Encoder::new(config, handler)?;
        self.coded_size = encoder.coded_size();
        self.encoder = Some(encoder);
        self.rebuild_needed = false;
        self.reconfigure_needed = false;
        Ok(())
    }

    fn reconfigure_encoder(&mut self) -> Result<()> {
        let Some(encoder) = self.encoder.as_mut() else {
            return Err(crate::Error::VplMessage {
                reason: "VPL encoder instance is not initialized".to_string(),
            });
        };
        let params = ReconfigureParams {
            target_kbps: Some(target_kbps_from_bps(self.target_bitrate_bps)),
            framerate_num: Some(self.framerate.max(1)),
            framerate_den: Some(1),
            ..ReconfigureParams::default()
        };
        match encoder.reconfigure(params) {
            Ok(()) => {
                self.reconfigure_needed = false;
                Ok(())
            }
            Err(err) => {
                rtc_log_warning!(
                    "VPL encoder reconfigure failed for {:?}: {}",
                    self.codec_type,
                    err
                );
                Err(err.into())
            }
        }
    }
}

impl VideoEncoderHandler for VplVideoEncoder {
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
        if self.width != frame_width || self.height != frame_height {
            self.width = frame_width;
            self.height = frame_height;
            // 解像度変更は再初期化が必要なのでフラグを立てる。
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
            // ビットレートやフレームレートの更新は再初期化ではなく reconfigure を行うが、
            // reconfigure が失敗することがあるため、再初期化にフォールバックする。
            //
            // reconfigure のエラーは実際に発生する。GMKtec K9 のマシンでは H265 と AV1 で以下のエラーになった。
            //
            // ```
            // MFXVideoENCODE_Reset() failed[status=-14]: Incompatible video parameters (MFX_ERR_INCOMPATIBLE_VIDEO_PARAM)
            // ```
            //
            // そのためこのフォールバックは必須となる。
            rtc_log_warning!(
                "VPL encoder reconfigure failed for {:?}; falling back to rebuild",
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

        let (coded_width, coded_height) = self.coded_size;
        let Some((coded_nv12_size, y_plane_size, dst_stride_y, dst_stride_uv)) =
            || -> Option<(usize, usize, i32, i32)> {
                let y_plane_size = coded_width.checked_mul(coded_height)?;
                let coded_nv12_size = FrameFormat::Nv12.frame_size(coded_width, coded_height)?;
                let dst_stride_y = i32::try_from(coded_width).ok()?;
                let dst_stride_uv = dst_stride_y;
                Some((coded_nv12_size, y_plane_size, dst_stride_y, dst_stride_uv))
            }()
        else {
            rtc_log_error!(
                "VPL coded size {}x{} is invalid for {:?}",
                coded_width,
                coded_height,
                self.codec_type
            );
            return VideoCodecStatus::Error;
        };

        let mut nv12 = vec![0u8; coded_nv12_size];
        {
            let (dst_y, dst_uv) = nv12.split_at_mut(y_plane_size);
            if !i420_to_nv12(
                i420.y_data(),
                i420.stride_y(),
                i420.u_data(),
                i420.stride_u(),
                i420.v_data(),
                i420.stride_v(),
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

        let requested = requested_frame_type(frame_types);
        let force_frame_type = vpl_force_frame_type(self.codec_type, requested);
        let encode_options = EncodeOptions {
            frame_type: force_frame_type,
        };
        let user_data = VplEncoderUserData {
            rtp_timestamp: frame.rtp_timestamp(),
            width: frame_width,
            height: frame_height,
        };

        let encoder = self.encoder.as_mut().expect("encoder should exist");
        if let Err(err) = encoder.encode(&nv12, user_data, &encode_options) {
            rtc_log_error!("VPL encode failed for {:?}: {}", self.codec_type, err);
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
        *self.callback.lock().unwrap() = None;
        self.encoder = None;
        self.rebuild_needed = false;
        self.reconfigure_needed = false;
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
        info.set_implementation_name("VPL");
        info.set_is_hardware_accelerated(true);
        info
    }
}

struct VplVideoDecoder {
    adapter: AdapterSelector,
    callback: SharedDecoderCallback,
    decoder: Option<VplDecoderInstance>,
    codec_type: VideoCodecType,
}

impl VplVideoDecoder {
    fn new(adapter: AdapterSelector, codec_type: VideoCodecType) -> Self {
        Self {
            adapter,
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
            return Err(crate::Error::VplMessage {
                reason: "VPL decoder codec type is not supported".to_string(),
            });
        };
        let callback_state = self.callback.clone();
        let handler = FnDecodeHandler::new(move |result| match result {
            Ok(frame) => dispatch_decoded_frame(&callback_state, frame),
            Err(err) => {
                rtc_log_error!("VPL decode worker error: {}", err);
            }
        });
        let decoder = Decoder::new(DecoderConfig::new(self.adapter, codec), handler)?;
        self.decoder = Some(decoder);
        Ok(())
    }
}

impl VideoDecoderHandler for VplVideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != self.codec_type {
            return false;
        }
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

        let user_data = VplDecoderUserData {
            rtp_timestamp: input_image.rtp_timestamp(),
            timestamp_us: render_time_ms.saturating_mul(1000),
        };

        let decoder = self.decoder.as_mut().expect("decoder should exist");
        if let Err(err) = decoder.decode(bitstream, user_data) {
            rtc_log_error!("VPL decode failed: {}", err);
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
        info.set_implementation_name("VPL");
        info.set_is_hardware_accelerated(true);
        info
    }
}

pub struct VplVideoCodecCapability {
    adapter: AdapterSelector,
    encoder_supported_formats: Vec<SdpVideoFormat>,
    decoder_supported_formats: Vec<SdpVideoFormat>,
    simulcast_capability_helper: SimulcastCapabilityHelper,
}

impl VplVideoCodecCapability {
    pub fn new() -> Result<Self> {
        let adapter = default_adapter()?;
        let (encoder_supported_formats, decoder_supported_formats) =
            collect_supported_formats(adapter)?;
        if encoder_supported_formats.is_empty() && decoder_supported_formats.is_empty() {
            return Err(crate::Error::VplMessage {
                reason: "VPL does not support any encoder or decoder codec".to_string(),
            });
        }
        Self::new_with_formats(
            adapter,
            encoder_supported_formats,
            decoder_supported_formats,
        )
    }

    fn new_with_formats(
        adapter: AdapterSelector,
        encoder_supported_formats: Vec<SdpVideoFormat>,
        decoder_supported_formats: Vec<SdpVideoFormat>,
    ) -> Result<Self> {
        let encoder_supported_formats_for_factory = encoder_supported_formats.clone();
        let adapter_for_factory = adapter;

        let simulcast_capability_helper = SimulcastCapabilityHelper::new_with_builder(
            move || encoder_supported_formats_for_factory.clone(),
            move |_env, format| {
                let codec_type = codec_type_from_format(&format)?;
                Some(VideoEncoder::new_with_handler(Box::new(
                    VplVideoEncoder::new(adapter_for_factory, codec_type),
                )))
            },
        );

        Ok(Self {
            adapter,
            encoder_supported_formats,
            decoder_supported_formats,
            simulcast_capability_helper,
        })
    }
}

impl VideoCodecCapability for VplVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("vpl", "Intel VPL")
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
            VplVideoDecoder::new(self.adapter, codec_type),
        )))
    }
}

#[cfg(test)]
impl VplVideoCodecCapability {
    fn new_for_test(
        encoder_supported_formats: Vec<SdpVideoFormat>,
        decoder_supported_formats: Vec<SdpVideoFormat>,
    ) -> Result<Self> {
        // テストでは実際のアダプターを参照しないが、AdapterSelector::DrmRenderNode(0) は
        // libvpl の予約値のため、ダミーとして 128 (Linux の典型値) を使う。
        let adapter = AdapterSelector::DrmRenderNode(128);
        Self::new_with_formats(
            adapter,
            encoder_supported_formats,
            decoder_supported_formats,
        )
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
    fn vpl_capability_has_expected_implementation_name() {
        let capability = VplVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
        )
        .expect("VplVideoCodecCapability の生成に失敗");
        assert_eq!(capability.get_implementation().name(), "vpl");
    }

    #[test]
    fn vpl_capability_supports_formats_per_direction() {
        let capability = VplVideoCodecCapability::new_for_test(
            test_supported_formats(&[
                VideoCodecType::H264,
                VideoCodecType::H265,
                VideoCodecType::Av1,
            ]),
            test_supported_formats(&[
                VideoCodecType::H264,
                VideoCodecType::Vp9,
                VideoCodecType::Av1,
            ]),
        )
        .expect("VplVideoCodecCapability の生成に失敗");

        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H265));
        assert!(!capability.is_supported(CodecDirection::Decoder, VideoCodecType::H265));
        assert!(!capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp9));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::Vp9));
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::Av1));
        assert!(capability.is_supported(CodecDirection::Decoder, VideoCodecType::Av1));

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
        assert_eq!(decoder_formats, vec!["H264", "VP9", "AV1"]);

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
    fn vpl_capability_creates_vp9_encoder() {
        let capability = VplVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::Vp9]),
            test_supported_formats(&[VideoCodecType::Vp9]),
        )
        .expect("VplVideoCodecCapability の生成に失敗");
        let env = Environment::new();
        let format = SdpVideoFormat::new_with_parameters(
            "VP9",
            &HashMap::from([(String::from("profile-id"), String::from("0"))]),
            &[],
        );
        let encoder = capability
            .create_video_encoder(env.as_ref(), format.as_ref())
            .expect("VP9 対応フォーマットで encoder の生成に失敗");
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
    fn vpl_simulcast_adapter_encoder_info_contains_adapter_name() {
        let capability = VplVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
        )
        .expect("VplVideoCodecCapability の生成に失敗");
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
    fn vpl_requested_frame_type_uses_first_entry() {
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
    fn vpl_force_frame_type_matches_codec_requirements() {
        assert_eq!(
            vpl_force_frame_type(VideoCodecType::Vp9, Some(VideoFrameType::Key)),
            frame_type::I
        );
        assert_eq!(
            vpl_force_frame_type(VideoCodecType::H264, Some(VideoFrameType::Key)),
            frame_type::IDR | frame_type::I | frame_type::REF
        );
        assert_eq!(
            vpl_force_frame_type(VideoCodecType::Vp9, Some(VideoFrameType::Delta)),
            frame_type::UNKNOWN
        );
    }

    #[test]
    fn vpl_rate_control_mode_uses_vbr_for_vp9() {
        assert_eq!(
            vpl_rate_control_mode(VideoCodecType::Vp9),
            RateControlMode::Vbr
        );
        assert_eq!(
            vpl_rate_control_mode(VideoCodecType::H264),
            RateControlMode::Cbr
        );
    }

    #[test]
    fn vp9_payload_from_vpl_strips_ivf_headers() {
        let mut data = Vec::new();
        data.extend_from_slice(b"DKIF");
        data.resize(32, 0);
        data.extend_from_slice(&[4, 0, 0, 0]);
        data.extend_from_slice(&[0; 8]);
        data.extend_from_slice(&[1, 2, 3, 4]);
        let payload = vp9_payload_from_vpl(&data).expect("VP9 payload の抽出に失敗");
        assert_eq!(payload, &[1, 2, 3, 4]);
    }

    #[test]
    fn vp9_payload_from_vpl_rejects_truncated_frame_header() {
        let data = vec![0u8; 11];
        let err = vp9_payload_from_vpl(&data).expect_err("frame header 不足のとき失敗するべき");
        assert_eq!(err, "VP9 IVF frame header is truncated");
    }

    #[test]
    fn vpl_frame_type_mapping_matches_expected_values() {
        assert_eq!(frame_type_from_vpl(PictureType::Idr), VideoFrameType::Key);
        assert_eq!(frame_type_from_vpl(PictureType::I), VideoFrameType::Key);
        assert_eq!(frame_type_from_vpl(PictureType::P), VideoFrameType::Delta);
        assert_eq!(frame_type_from_vpl(PictureType::B), VideoFrameType::Delta);
        assert_eq!(
            frame_type_from_vpl(PictureType::Unknown),
            VideoFrameType::Delta
        );
    }
}
