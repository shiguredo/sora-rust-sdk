//! Intel VPL ハードウェアエンコーダー/デコーダー実装。
//!
//! `#[cfg(all(feature = "vpl", target_os = "linux"))]` でのみコンパイルされる。
//!
//! VPL は Linux 専用のため、モジュールは Linux でのみコンパイルされる。
use std::sync::{Arc, Mutex};

use shiguredo_vpl::{
    AdapterSelector, Av1EncoderConfig, CodecConfig, DecodedFrame, Decoder, DecoderCodec,
    DecoderConfig, EncodeOptions, EncodedFrame, Encoder, EncoderConfig, Error as VplError,
    FnDecodeHandler, FnEncodeHandler, FrameFormat, H264EncoderConfig, HevcEncoderConfig,
    PictureType, RateControlMode, ReconfigureParams, VideoCodecType as VplCodecType,
    Vp9EncoderConfig, Vp9Profile, frame_type, list_adapters, supported_codecs,
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

use crate::error::Result;
use crate::video_codec::{SimulcastCapabilityHelper, codec_type_from_format};
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};
use crate::video_codecs::helpers;

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
            encoder_supported_formats.extend(helpers::supported_formats_for_codec(codec_type));
        }
        if info.decoding.supported {
            decoder_supported_formats.extend(helpers::supported_formats_for_codec(codec_type));
        }
    }
    Ok((encoder_supported_formats, decoder_supported_formats))
}

// コーデック種別から VPL 固有のエンコーダー設定を返す。
fn encoder_codec_config(codec_type: VideoCodecType) -> Option<CodecConfig> {
    match codec_type {
        VideoCodecType::H264 => Some(CodecConfig::H264(H264EncoderConfig { profile: None })),
        VideoCodecType::H265 => Some(CodecConfig::Hevc(HevcEncoderConfig { profile: None })),
        VideoCodecType::Vp9 => Some(CodecConfig::Vp9(Vp9EncoderConfig {
            profile: Some(Vp9Profile::Profile0),
            // IVF ヘッダー出力を無効化する
            write_ivf_headers: false,
        })),
        VideoCodecType::Av1 => Some(CodecConfig::Av1(Av1EncoderConfig { profile: None })),
        _ => None,
    }
}

// コーデック種別から VPL 固有のデコーダー設定を返す。
fn decoder_codec(codec_type: VideoCodecType) -> Option<DecoderCodec> {
    match codec_type {
        VideoCodecType::H264 => Some(DecoderCodec::H264),
        VideoCodecType::H265 => Some(DecoderCodec::Hevc),
        VideoCodecType::Vp9 => Some(DecoderCodec::Vp9),
        VideoCodecType::Av1 => Some(DecoderCodec::Av1),
        _ => None,
    }
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

// VPL 固有の `PictureType` を `VideoFrameType` に変換する。
fn frame_type_from_vpl(picture_type: PictureType) -> VideoFrameType {
    match picture_type {
        PictureType::Idr | PictureType::I => VideoFrameType::Key,
        PictureType::P | PictureType::B | PictureType::Unknown => VideoFrameType::Delta,
    }
}

// VPL の受け口 (`EncoderConfig::target_kbps` / `ReconfigureParams::target_kbps`) は
// `u16` のため、共通ヘルパーの戻り値 (`u32`) を変換する。65,535 kbps 超は
// 現行どおり `u16::MAX` でクリップする（サイレントに壊れるリスクは
// VPL の API 上限自体に由来するため）。
fn target_kbps_u16(target_bitrate_bps: u32) -> u16 {
    u16::try_from(helpers::target_kbps_from_bps(target_bitrate_bps)).unwrap_or(u16::MAX)
}

fn vpl_rate_control_mode(codec_type: VideoCodecType) -> RateControlMode {
    if codec_type == VideoCodecType::Vp9 {
        // VP9 は環境依存で CBR 初期化/実行失敗が出るため VBR を採用する。
        RateControlMode::Vbr
    } else {
        RateControlMode::Cbr
    }
}

// コールバックで受け取る値
// EncodedFrame の value に入れて渡す
#[derive(Clone, Copy)]
struct EncoderCallbackValue {
    rtp_timestamp: u32,
    frame_width: u32,
    frame_height: u32,
}

// コールバックの共有状態
// handle_vpl_encode_callback と register_encode_complete_callback / release の両方からアクセスされる
#[derive(Default)]
struct EncoderCallbackState {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
}

// VPL エンコードコールバックを処理する
// EncodedFrame<EncoderCallbackValue> を受け取り、EncodedImage を構築して callback を呼び出す
fn handle_vpl_encode_callback(
    callback_state: &Arc<Mutex<EncoderCallbackState>>,
    result: std::result::Result<EncodedFrame<EncoderCallbackValue>, VplError>,
    codec_type: VideoCodecType,
) {
    let encoded = match result {
        Ok(encoded) => encoded,
        Err(err) => {
            rtc_log_error!("VPL encode callback failed for {:?}: {}", codec_type, err);
            return;
        }
    };

    let picture_type = encoded.picture_type();
    let output_frame_type = frame_type_from_vpl(picture_type);
    let rtp_timestamp = encoded.user_data().rtp_timestamp;
    let frame_width = encoded.user_data().frame_width;
    let frame_height = encoded.user_data().frame_height;

    let data = encoded.into_data();
    let mut encoded_image = EncodedImage::new();
    let encoded_buffer = EncodedImageBuffer::from_bytes(&data);
    encoded_image.set_encoded_data(&encoded_buffer);
    encoded_image.set_rtp_timestamp(rtp_timestamp);
    encoded_image.set_encoded_width(frame_width);
    encoded_image.set_encoded_height(frame_height);
    encoded_image.set_frame_type(output_frame_type);

    let mut codec_specific_info = CodecSpecificInfo::new();
    codec_specific_info.set_codec_type(codec_type);
    if codec_type == VideoCodecType::Vp9 {
        let is_key = output_frame_type == VideoFrameType::Key;
        // VP9 では end_of_picture を明示しないと受信側で codec 判定が外れるケースがある。
        codec_specific_info.set_end_of_picture(true);
        // num_spatial_layers を設定しないと first_active_layer / num_spatial_layers が不整合でエラーになる
        codec_specific_info.set_vp9_num_spatial_layers(1);
        codec_specific_info
            .set_vp9_temporal_idx(shiguredo_webrtc::constants::no_temporal_idx().into());
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
            "VPL: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
        );
    }
}

struct VplVideoEncoder {
    encoder: Option<Encoder<FnEncodeHandler<EncoderCallbackValue>>>,
    callback_state: Arc<Mutex<EncoderCallbackState>>,
    adapter: AdapterSelector,
    codec_type: VideoCodecType,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    rebuild_needed: bool,
    reconfigure_needed: bool,
}

impl VplVideoEncoder {
    fn new(adapter: AdapterSelector, codec_type: VideoCodecType) -> Self {
        Self {
            encoder: None,
            callback_state: Arc::new(Mutex::new(EncoderCallbackState::default())),
            adapter,
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
        config.target_kbps = Some(target_kbps_u16(self.target_bitrate_bps));

        let callback_state_for_callback = self.callback_state.clone();
        let callback_codec_type = self.codec_type;
        let encoder = Encoder::new(
            config,
            FnEncodeHandler::new(move |result| {
                handle_vpl_encode_callback(
                    &callback_state_for_callback,
                    result,
                    callback_codec_type,
                );
            }),
        )?;

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
            target_kbps: Some(target_kbps_u16(self.target_bitrate_bps)),
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
        if self.width != frame_width || self.height != frame_height {
            self.width = frame_width;
            self.height = frame_height;
            // 解像度変更は再初期化が必要なのでフラグを立てる。
            self.rebuild_needed = true;
        }
        if self.encoder.is_none() {
            self.rebuild_needed = true;
        }

        // rebuild_needed, reconfigure_needed のハンドリング
        if self.rebuild_needed {
            if self.rebuild_encoder().is_err() {
                return VideoCodecStatus::Error;
            }
        } else if self.reconfigure_needed {
            // ビットレートやフレームレートの更新は再初期化ではなく reconfigure を行う
            if self.reconfigure_encoder().is_err() {
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

        let src_stride_y = i420.stride_y();
        let src_stride_u = i420.stride_u();
        let src_stride_v = i420.stride_v();

        let rtp_timestamp = frame.rtp_timestamp();
        let Some(encoder) = self.encoder.as_ref() else {
            return VideoCodecStatus::Error;
        };
        let (coded_width, coded_height) = encoder.coded_size();
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

        let requested = helpers::requested_frame_type(frame_types);
        let force_frame_type = vpl_force_frame_type(self.codec_type, requested);
        let encode_options = EncodeOptions {
            frame_type: force_frame_type,
        };
        let callback_value = EncoderCallbackValue {
            rtp_timestamp,
            frame_width,
            frame_height,
        };
        let Some(encoder) = self.encoder.as_mut() else {
            return VideoCodecStatus::Error;
        };
        if let Err(err) = encoder.encode(&nv12, callback_value, &encode_options) {
            rtc_log_error!("VPL encode failed for {:?}: {}", self.codec_type, err);
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
        // self.encoder = None で drain 処理が走ってコールバックハンドラが呼ばれ、
        // コールバックハンドラの中でロックを獲得しようとするので、
        // ここで Mutex を unlock しておかないとデッドロックになる
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
        info.set_implementation_name("VPL");
        info.set_is_hardware_accelerated(true);
        info
    }
}

// デコード時に渡すコールバック値
// DecodedFrame の value に入れて受け取る
#[derive(Clone, Copy)]
struct DecoderCallbackValue {
    rtp_timestamp: u32,
    render_time_ms: i64,
}

// デコードコールバックの共有状態
// handle_vpl_decode_callback と register_decode_complete_callback / release の両方からアクセスされる
#[derive(Default)]
struct DecoderCallbackState {
    callback: Option<VideoDecoderDecodedImageCallbackPtr>,
}

// VPL デコードコールバックを処理する
// DecodedFrame<DecoderCallbackValue> を受け取り、NV12 から VideoFrame を構築して callback を呼び出す
fn handle_vpl_decode_callback(
    callback_state: &Arc<Mutex<DecoderCallbackState>>,
    result: std::result::Result<DecodedFrame<'_, DecoderCallbackValue>, VplError>,
) {
    let frame = match result {
        Ok(frame) => frame,
        Err(err) => {
            rtc_log_error!("VPL decode callback failed: {}", err);
            return;
        }
    };

    let width_i32 = match i32::try_from(frame.width()) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!(
                "VPL decoded frame width exceeds i32::MAX: {}",
                frame.width()
            );
            return;
        }
    };
    let height_i32 = match i32::try_from(frame.height()) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!(
                "VPL decoded frame height exceeds i32::MAX: {}",
                frame.height()
            );
            return;
        }
    };
    let pitch_i32 = match i32::try_from(frame.pitch()) {
        Ok(v) => v,
        Err(_) => {
            rtc_log_error!(
                "VPL decoded frame pitch exceeds i32::MAX: {}",
                frame.pitch()
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
            frame.y(),
            pitch_i32,
            frame.uv(),
            pitch_i32,
            dst_y,
            dst_stride_y,
            dst_uv,
            dst_stride_uv,
            width_i32,
            height_i32,
        ) {
            rtc_log_error!("VPL decode callback failed to copy NV12 frame");
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

struct VplVideoDecoder {
    callback_state: Arc<Mutex<DecoderCallbackState>>,
    decoder: Option<Decoder<FnDecodeHandler<DecoderCallbackValue>>>,
    adapter: AdapterSelector,
    codec_type: VideoCodecType,
}

impl VplVideoDecoder {
    fn new(adapter: AdapterSelector, codec_type: VideoCodecType) -> Self {
        Self {
            callback_state: Arc::new(Mutex::new(DecoderCallbackState::default())),
            decoder: None,
            adapter,
            codec_type,
        }
    }

    fn rebuild_decoder(&mut self) -> Result<()> {
        let Some(codec) = decoder_codec(self.codec_type) else {
            return Err(crate::Error::VplMessage {
                reason: "VPL decoder codec type is not supported".to_string(),
            });
        };
        let config = DecoderConfig::new(self.adapter, codec);
        let callback_state = self.callback_state.clone();
        self.decoder = Some(Decoder::new(
            config,
            FnDecodeHandler::new(move |result| {
                handle_vpl_decode_callback(&callback_state, result);
            }),
        )?);
        Ok(())
    }
}

impl VideoDecoderHandler for VplVideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != self.codec_type {
            return false;
        }
        match self.rebuild_decoder() {
            Ok(()) => true,
            Err(err) => {
                rtc_log_error!("VPL rebuild decoder failed in configure: {}", err);
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
            rtc_log_error!("VPL rebuild decoder failed in decode: {}", err);
            return VideoCodecStatus::Error;
        }

        let callback_value = DecoderCallbackValue {
            rtp_timestamp: input_image.rtp_timestamp(),
            render_time_ms,
        };
        let decoder = self.decoder.as_mut().expect("decoder should exist");
        if let Err(err) = decoder.decode(encoded_data.data(), callback_value) {
            rtc_log_error!("VPL decode failed for {:?}: {}", self.codec_type, err);
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
        // self.decoder = None で drain 処理が走ってコールバックハンドラが呼ばれ、
        // コールバックハンドラの中でロックを獲得しようとするので、
        // ここで Mutex を unlock しておかないとデッドロックになる
        drop(callback_state);
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

/// Intel VPL を使用する [VideoCodecCapability]。
///
/// `#[cfg(all(feature = "vpl", target_os = "linux"))]` でのみ利用可能。
/// H.264 / H.265 / VP9 / AV1 のエンコード・デコードをサポートする。
pub struct VplVideoCodecCapability {
    adapter: AdapterSelector,
    encoder_supported_formats: Vec<SdpVideoFormat>,
    decoder_supported_formats: Vec<SdpVideoFormat>,
    simulcast_capability_helper: SimulcastCapabilityHelper,
}

impl VplVideoCodecCapability {
    /// 利用可能な最初の VPL アダプターで新しい `VplVideoCodecCapability` を生成する。
    ///
    /// 利用可能なエンコーダー/デコーダーが存在しない場合はエラーを返す。
    pub fn new() -> Result<Self> {
        let adapters = list_adapters()?;
        let adapter_info = adapters.first().ok_or_else(|| crate::Error::VplMessage {
            reason: "VPL: no hardware adapter found".to_string(),
        })?;
        let adapter = AdapterSelector::DrmRenderNode(adapter_info.drm_render_node);
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

        let simulcast_capability_helper = SimulcastCapabilityHelper::new_with_builder(
            move || encoder_supported_formats_for_factory.clone(),
            {
                move |_env, format| {
                    let codec_type = codec_type_from_format(&format)?;
                    Some(VideoEncoder::new_with_handler(Box::new(
                        VplVideoEncoder::new(adapter, codec_type),
                    )))
                }
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
    use std::collections::HashMap;

    use super::*;
    use shiguredo_webrtc::{Environment, SdpVideoFormat, VideoFrameType};

    fn test_supported_formats(codec_types: &[VideoCodecType]) -> Vec<SdpVideoFormat> {
        let mut supported_formats = Vec::new();
        for codec_type in codec_types {
            supported_formats.extend(helpers::supported_formats_for_codec(*codec_type));
        }
        supported_formats
    }

    #[test]
    fn vpl_capability_has_expected_implementation_name() {
        let capability = VplVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
        )
        .expect("テスト用の VplVideoCodecCapability の生成に失敗しました");
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
        .expect("テスト用の VplVideoCodecCapability の生成に失敗しました");

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
    }

    #[test]
    fn vpl_capability_creates_vp9_encoder() {
        let capability = VplVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::Vp9]),
            test_supported_formats(&[VideoCodecType::Vp9]),
        )
        .expect("テスト用の VplVideoCodecCapability の生成に失敗しました");
        let env = Environment::new();
        let format = SdpVideoFormat::new_with_parameters(
            "VP9",
            &HashMap::from([(String::from("profile-id"), String::from("0"))]),
            &[],
        );
        let encoder = capability
            .create_video_encoder(env.as_ref(), format.as_ref())
            .expect(
                "サポートされている VP9 フォーマットでエンコーダーが生成されなければなりません",
            );
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
        .expect("テスト用の VplVideoCodecCapability の生成に失敗しました");
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
