use std::collections::HashMap;

use shiguredo_vpl::{
    Av1EncoderConfig, CodecConfig, Decoder, DecoderCodec, DecoderConfig, EncodeOptions, Encoder,
    EncoderConfig, FrameFormat, H264EncoderConfig, HevcEncoderConfig, PictureType, RateControlMode,
    ReconfigureParams, VideoCodecType as VplCodecType, Vp9EncoderConfig, Vp9Profile, frame_type,
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
    i420_to_nv12, nv12_to_i420, rtc_log_error, rtc_log_warning,
};

use crate::error::Result;
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
        VideoCodecType::Vp9 => 2,
        VideoCodecType::Av1 => 3,
        _ => u8::MAX,
    }
}

fn collect_codec_availability() -> Result<Vec<CodecAvailability>> {
    let codec_infos =
        supported_codecs().map_err(|err| crate::Error::InvalidVideoCodecCapability {
            reason: format!("failed to query VPL supported codecs: {err}"),
        })?;

    let mut codecs = Vec::new();
    for info in codec_infos {
        let codec_type = match info.codec {
            VplCodecType::H264 => VideoCodecType::H264,
            VplCodecType::Hevc => VideoCodecType::H265,
            VplCodecType::Vp9 => VideoCodecType::Vp9,
            VplCodecType::Av1 => VideoCodecType::Av1,
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
    Ok(codecs)
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

struct VplVideoEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
    encoder: Option<Encoder>,
    codec_type: VideoCodecType,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    rebuild_needed: bool,
    reconfigure_needed: bool,
}

impl VplVideoEncoder {
    fn new(codec_type: VideoCodecType) -> Self {
        Self {
            callback: None,
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

    fn rebuild_encoder(&mut self) -> std::result::Result<(), ()> {
        if self.width == 0 || self.height == 0 {
            return Err(());
        }
        let Some(codec_config) = encoder_codec_config(self.codec_type) else {
            return Err(());
        };

        let mut config = EncoderConfig::new(
            codec_config,
            self.width,
            self.height,
            FrameFormat::Nv12,
            self.framerate.max(1),
            1,
            vpl_rate_control_mode(self.codec_type),
        );
        config.target_kbps = Some(target_kbps_from_bps(self.target_bitrate_bps));

        self.encoder = match Encoder::new(config) {
            Ok(encoder) => Some(encoder),
            Err(err) => {
                rtc_log_error!(
                    "VPL encoder initialization failed for {:?}: {}",
                    self.codec_type,
                    err
                );
                None
            }
        };
        self.rebuild_needed = false;
        self.reconfigure_needed = false;
        self.encoder.as_ref().map(|_| ()).ok_or(())
    }

    fn reconfigure_encoder(&mut self) -> std::result::Result<(), ()> {
        let Some(encoder) = self.encoder.as_mut() else {
            return Err(());
        };
        let params = ReconfigureParams {
            target_kbps: Some(target_kbps_from_bps(self.target_bitrate_bps)),
            max_kbps: None,
            framerate_num: Some(self.framerate.max(1)),
            framerate_den: Some(1),
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
                Err(())
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
            // ビットレート更新は再初期化ではなく reconfigure を行う
            if self.reconfigure_encoder().is_err() {
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
        let requested = requested_frame_type(frame_types);
        let force_frame_type = vpl_force_frame_type(self.codec_type, requested);
        let encode_options = EncodeOptions {
            frame_type: force_frame_type,
        };
        if let Err(err) = encoder.encode(nv12.data(), &encode_options) {
            rtc_log_error!("VPL encode failed for {:?}: {}", self.codec_type, err);
            return VideoCodecStatus::Error;
        }

        while let Some(encoded_frame) = encoder.next_frame() {
            let mut encoded_image = EncodedImage::new();
            let encoded_payload = if self.codec_type == VideoCodecType::Vp9 {
                match vp9_payload_from_vpl(encoded_frame.data()) {
                    Ok(payload) => payload,
                    Err(err) => {
                        rtc_log_error!("VPL VP9 payload normalization failed: {}", err);
                        return VideoCodecStatus::Error;
                    }
                }
            } else {
                encoded_frame.data()
            };
            let encoded_buffer = EncodedImageBuffer::from_bytes(encoded_payload);
            encoded_image.set_encoded_data(&encoded_buffer);
            encoded_image.set_rtp_timestamp(rtp_timestamp);
            encoded_image.set_encoded_width(frame_width);
            encoded_image.set_encoded_height(frame_height);
            let output_frame_type = frame_type_from_vpl(encoded_frame.picture_type());
            encoded_image.set_frame_type(output_frame_type);

            let mut codec_specific_info = CodecSpecificInfo::new();
            codec_specific_info.set_codec_type(self.codec_type);
            if self.codec_type == VideoCodecType::Vp9 {
                let is_key = output_frame_type == VideoFrameType::Key;
                // VP9 では end_of_picture を明示しないと受信側で codec 判定が外れるケースがある。
                codec_specific_info.set_end_of_picture(true);
                // num_spatial_layers を設定しないと first_active_layer / num_spatial_layers が不整合でエラーになる
                codec_specific_info.set_vp9_num_spatial_layers(1);
                codec_specific_info
                    .set_vp9_temporal_idx(shiguredo_webrtc::no_temporal_idx().into());
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
                rtc_log_warning!(
                    "NVCODEC: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
                );
            }
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
    callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    decoder: Option<Decoder>,
    codec_type: VideoCodecType,
}

impl VplVideoDecoder {
    fn new(codec_type: VideoCodecType) -> Self {
        Self {
            callback: None,
            decoder: None,
            codec_type,
        }
    }

    fn ensure_decoder(&mut self) -> std::result::Result<(), ()> {
        if self.decoder.is_none() {
            let Some(codec) = decoder_codec(self.codec_type) else {
                return Err(());
            };
            self.decoder = Decoder::new(DecoderConfig { codec }).ok();
        }
        self.decoder.as_ref().map(|_| ()).ok_or(())
    }
}

impl VideoDecoderHandler for VplVideoDecoder {
    fn configure(&mut self, settings: VideoDecoderSettingsRef<'_>) -> bool {
        if settings.codec_type() != self.codec_type {
            return false;
        }
        let Some(codec) = decoder_codec(self.codec_type) else {
            return false;
        };
        self.decoder = Decoder::new(DecoderConfig { codec }).ok();
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
        let decoder = self.decoder.as_mut().expect("decoder should exist");
        if decoder.decode(encoded_data.data()).is_err() {
            return VideoCodecStatus::Error;
        }

        let mut decoded_images = Vec::new();
        while let Some(frame) = decoder.next_frame() {
            let Some(y_plane_size) = frame.width().checked_mul(frame.height()) else {
                return VideoCodecStatus::Error;
            };
            if frame.data().len() < y_plane_size {
                return VideoCodecStatus::Error;
            }
            let (src_y, src_uv) = frame.data().split_at(y_plane_size);

            let width_i32 = match i32::try_from(frame.width()) {
                Ok(v) => v,
                Err(_) => return VideoCodecStatus::Error,
            };
            let height_i32 = match i32::try_from(frame.height()) {
                Ok(v) => v,
                Err(_) => return VideoCodecStatus::Error,
            };

            let mut i420 = I420Buffer::new(width_i32, height_i32);
            let dst_stride_y = i420.stride_y();
            let dst_stride_u = i420.stride_u();
            let dst_stride_v = i420.stride_v();
            let (dst_y, dst_u, dst_v) = i420.planes_mut();
            if !nv12_to_i420(
                src_y,
                width_i32,
                src_uv,
                width_i32,
                dst_y,
                dst_stride_y,
                dst_u,
                dst_stride_u,
                dst_v,
                dst_stride_v,
                width_i32,
                height_i32,
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
        info.set_implementation_name("VPL");
        info.set_is_hardware_accelerated(true);
        info
    }
}

pub struct VplVideoCodecCapability {
    codecs: Vec<CodecAvailability>,
    simulcast_capability_helper: SimulcastCapabilityHelper,
}

impl VplVideoCodecCapability {
    pub fn new() -> Result<Self> {
        let codecs = collect_codec_availability()?;
        if !codecs
            .iter()
            .any(|codec| codec.encoder_supported || codec.decoder_supported)
        {
            return Err(crate::Error::InvalidVideoCodecCapability {
                reason: "VPL does not support any encoder or decoder codec".to_string(),
            });
        }
        Ok(Self::new_with_codecs(codecs))
    }

    fn new_with_codecs(codecs: Vec<CodecAvailability>) -> Self {
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
                        VplVideoEncoder::new(codec_type),
                    )))
                }
            },
        );

        Self {
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

impl VideoCodecCapability for VplVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("vpl", "Intel VPL")
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
            VplVideoDecoder::new(codec_type),
        )))
    }
}

#[cfg(test)]
impl VplVideoCodecCapability {
    fn new_for_test(codecs: Vec<CodecAvailability>) -> Self {
        Self::new_with_codecs(codecs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::{Environment, SdpVideoFormat, VideoFrameType, VideoFrameTypeVector};

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
    fn vpl_capability_has_expected_implementation_name() {
        let capability = VplVideoCodecCapability::new_for_test(vec![test_codec(
            VideoCodecType::H264,
            true,
            true,
        )]);
        assert_eq!(capability.get_implementation().name(), "vpl");
    }

    #[test]
    fn vpl_capability_supports_formats_per_direction() {
        let capability = VplVideoCodecCapability::new_for_test(vec![
            test_codec(VideoCodecType::H264, true, true),
            test_codec(VideoCodecType::H265, true, false),
            test_codec(VideoCodecType::Vp9, false, true),
            test_codec(VideoCodecType::Av1, true, true),
        ]);

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
    fn vpl_capability_rejects_unsupported_encoder_creation() {
        let capability = VplVideoCodecCapability::new_for_test(vec![test_codec(
            VideoCodecType::Vp9,
            false,
            true,
        )]);

        let env = Environment::new();
        assert!(
            capability
                .create_video_encoder(env.as_ref(), SdpVideoFormat::new("VP9").as_ref())
                .is_none()
        );
    }

    #[test]
    fn vpl_capability_rejects_unsupported_decoder_creation() {
        let capability = VplVideoCodecCapability::new_for_test(vec![test_codec(
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
    fn vpl_capability_creates_vp9_encoder() {
        let capability = VplVideoCodecCapability::new_for_test(vec![test_codec(
            VideoCodecType::Vp9,
            true,
            true,
        )]);
        let env = Environment::new();
        let format = SdpVideoFormat::new_with_parameters(
            "VP9",
            &HashMap::from([(String::from("profile-id"), String::from("0"))]),
            &[],
        );
        let encoder = capability
            .create_video_encoder(env.as_ref(), format.as_ref())
            .expect("encoder must be created for supported VP9 format");
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
        let capability = VplVideoCodecCapability::new_for_test(vec![test_codec(
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
        let payload = vp9_payload_from_vpl(&data).expect("vp9 payload should be extracted");
        assert_eq!(payload, &[1, 2, 3, 4]);
    }

    #[test]
    fn vp9_payload_from_vpl_rejects_truncated_frame_header() {
        let data = vec![0u8; 11];
        let err = vp9_payload_from_vpl(&data).expect_err("truncated frame header must fail");
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
