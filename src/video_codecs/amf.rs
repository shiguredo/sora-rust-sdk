use std::collections::HashMap;

use shiguredo_amf::{
    Av1EncoderConfig, CodecConfig, Decoder, DecoderCodec, DecoderConfig, EncodeOptions, Encoder,
    EncoderConfig, FrameFormat, H264EncoderConfig, HevcEncoderConfig, PictureType, RateControlMode,
    VideoCodecType as AmfCodecType, frame_type, supported_codecs,
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
    i420_to_nv12, nv12_to_i420, rtc_log_warning,
};

use crate::error::Result;
use crate::video_codec::{
    AlignmentEncoderAdapter, SimulcastCapabilityHelper, codec_type_from_format,
};
use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

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

struct AmfVideoEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
    encoder: Option<Encoder>,
    codec_type: VideoCodecType,
    width: u32,
    height: u32,
    framerate: u32,
    target_bitrate_bps: u32,
    reconfigure_needed: bool,
}

impl AmfVideoEncoder {
    fn new(codec_type: VideoCodecType) -> Self {
        Self {
            callback: None,
            encoder: None,
            codec_type,
            width: 0,
            height: 0,
            framerate: 30,
            target_bitrate_bps: 500_000,
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

        self.reconfigure_needed = false;
        self.encoder = Some(Encoder::new(config)?);
        Ok(())
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

        let options = EncodeOptions {
            frame_type: amf_force_frame_type(requested_frame_type),
        };
        let encoder = self.encoder.as_mut().expect("encoder should exist");
        if encoder.encode(nv12.data(), &options).is_err() {
            self.reconfigure_needed = true;
            return VideoCodecStatus::Error;
        }

        while let Some(encoded_frame) = encoder.next_frame() {
            let mut encoded_image = EncodedImage::new();
            let encoded_buffer = EncodedImageBuffer::from_bytes(encoded_frame.data());
            encoded_image.set_encoded_data(&encoded_buffer);
            encoded_image.set_rtp_timestamp(frame.rtp_timestamp());
            encoded_image.set_encoded_width(frame_width);
            encoded_image.set_encoded_height(frame_height);
            encoded_image.set_frame_type(frame_type_from_amf(encoded_frame.picture_type()));

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
                rtc_log_warning!(
                    "AMF: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
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
    callback: Option<VideoDecoderDecodedImageCallbackPtr>,
    decoder: Option<Decoder>,
    codec_type: VideoCodecType,
}

impl AmfVideoDecoder {
    fn new(codec_type: VideoCodecType) -> Self {
        Self {
            callback: None,
            decoder: None,
            codec_type,
        }
    }

    fn ensure_decoder(&mut self) -> Result<()> {
        if self.decoder.is_none() {
            let Some(codec) = decoder_codec(self.codec_type) else {
                return Err(crate::Error::AmfMessage {
                    reason: "AMF decoder codec type is not supported".to_string(),
                });
            };
            let decoder = Decoder::new(DecoderConfig { codec })?;
            self.decoder = Some(decoder);
        }
        Ok(())
    }
}

impl VideoDecoderHandler for AmfVideoDecoder {
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
                        // また `shiguredo_amf` 2026.1.0 の公開 API では
                        // `Av1AlignmentMode` を切り替える設定項目が露出していない。
                        // そのため SDK 側でアダプターを噛ませて吸収する。
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
        .expect("Failed to create AmfVideoCodecCapability for test");
        assert_eq!(capability.get_implementation().name(), "amf");
    }

    #[test]
    fn amf_capability_supports_formats_per_direction() {
        let capability = AmfVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264, VideoCodecType::Av1]),
            test_supported_formats(&[VideoCodecType::H264, VideoCodecType::H265]),
        )
        .expect("Failed to create AmfVideoCodecCapability for test");

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
    fn amf_capability_create_video_encoder_uses_simulcast_adapter() {
        let capability = AmfVideoCodecCapability::new_for_test(
            test_supported_formats(&[VideoCodecType::H264]),
            test_supported_formats(&[VideoCodecType::H264]),
        )
        .expect("Failed to create AmfVideoCodecCapability for test");

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
    fn amf_frame_type_mapping_matches_idr_only_keyframe() {
        assert_eq!(frame_type_from_amf(PictureType::Idr), VideoFrameType::Key);
        assert_eq!(frame_type_from_amf(PictureType::I), VideoFrameType::Delta);
        assert_eq!(frame_type_from_amf(PictureType::P), VideoFrameType::Delta);
    }
}
