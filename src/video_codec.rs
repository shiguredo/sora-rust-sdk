use std::sync::{Arc, Mutex};

use shiguredo_webrtc::{
    EnvironmentRef, SdpVideoFormat, SdpVideoFormatRef, SimulcastEncoderAdapter, VideoCodec,
    VideoCodecRef, VideoCodecStatus, VideoCodecType, VideoDecoder, VideoDecoderFactoryHandler,
    VideoEncoder, VideoEncoderEncodedImageCallbackRef, VideoEncoderEncoderInfo,
    VideoEncoderFactory, VideoEncoderFactoryHandler, VideoEncoderHandler,
    VideoEncoderRateControlParametersRef, VideoEncoderSettingsRef, VideoFrame, VideoFrameRef,
    VideoFrameTypeVectorRef,
};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};
use crate::video_codec_preference::VideoCodecPreference;

type VideoCodecCapabilities = Vec<Box<dyn VideoCodecCapability>>;
type SharedVideoCodecCapabilities = Arc<Mutex<VideoCodecCapabilities>>;

pub struct SoraVideoEncoderFactory {
    preference: VideoCodecPreference,
    capabilities: SharedVideoCodecCapabilities,
}

pub struct SoraVideoDecoderFactory {
    preference: VideoCodecPreference,
    capabilities: SharedVideoCodecCapabilities,
}

impl SoraVideoEncoderFactory {
    pub(crate) fn new(
        preference: VideoCodecPreference,
        capabilities: SharedVideoCodecCapabilities,
    ) -> Self {
        Self {
            preference,
            capabilities,
        }
    }
}

impl SoraVideoDecoderFactory {
    pub(crate) fn new(
        preference: VideoCodecPreference,
        capabilities: SharedVideoCodecCapabilities,
    ) -> Self {
        Self {
            preference,
            capabilities,
        }
    }
}

impl VideoEncoderFactoryHandler for SoraVideoEncoderFactory {
    fn get_supported_formats(&mut self) -> Vec<SdpVideoFormat> {
        let capabilities = self.capabilities.lock().unwrap();
        collect_supported_formats(&self.preference, &capabilities, CodecDirection::Encoder)
    }

    fn create(
        &mut self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        let format_name = format.name().ok()?;
        let codec_type = VideoCodecType::try_from(format_name.as_str()).ok()?;
        let preference = self.preference.find(CodecDirection::Encoder, codec_type)?;
        let capabilities = self.capabilities.lock().unwrap();
        let capability = find_capability(&capabilities, preference.implementation())?;
        let resolved = capability.resolve_sdp_format(CodecDirection::Encoder, format)?;
        capability.create_video_encoder(env, resolved.as_ref())
    }
}

impl VideoDecoderFactoryHandler for SoraVideoDecoderFactory {
    fn get_supported_formats(&mut self) -> Vec<SdpVideoFormat> {
        let capabilities = self.capabilities.lock().unwrap();
        collect_supported_formats(&self.preference, &capabilities, CodecDirection::Decoder)
    }

    fn create(
        &mut self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        let format_name = format.name().ok()?;
        let codec_type = VideoCodecType::try_from(format_name.as_str()).ok()?;
        let preference = self.preference.find(CodecDirection::Decoder, codec_type)?;
        let capabilities = self.capabilities.lock().unwrap();
        let capability = find_capability(&capabilities, preference.implementation())?;
        let resolved = capability.resolve_sdp_format(CodecDirection::Decoder, format)?;
        capability.create_video_decoder(env, resolved.as_ref())
    }
}

/// 指定方向の `VideoCodecPreference` から公開する `SdpVideoFormat` 一覧を構築する。
///
/// 各 codec について `capability.get_supported_formats()` から対象 codec の
/// format を取り出す。
/// 返却時は `SdpVideoFormat::is_equal` で重複を除外する。
/// なお、走査順は入力の `preference.codecs()` に従う。
fn collect_supported_formats(
    preference: &VideoCodecPreference,
    capabilities: &[Box<dyn VideoCodecCapability>],
    direction: CodecDirection,
) -> Vec<SdpVideoFormat> {
    let mut formats = Vec::new();
    // preference の順序を維持しつつ、方向が一致する codec だけを列挙する。
    for codec in preference.codecs() {
        if codec.direction() != direction {
            continue;
        }
        // この codec が利用する capability を探す
        let Some(capability) = find_capability(capabilities, codec.implementation()) else {
            continue;
        };
        // capability が明示的な対応 format を返せる場合は、まずそれを優先する。
        // ただし別コーデックの format が混ざる可能性があるため、対象コーデックのみ採用する。
        for format in capability.get_supported_formats(codec.direction()) {
            let format_codec_type = format
                .name()
                .ok()
                .and_then(|name| VideoCodecType::try_from(name.as_str()).ok());
            if format_codec_type != Some(codec.codec_type()) {
                continue;
            }
            if !formats
                .iter()
                .any(|existing: &SdpVideoFormat| existing.is_equal(format.as_ref()))
            {
                formats.push(format);
            }
        }
    }
    formats
}

fn find_capability<'a>(
    capabilities: &'a [Box<dyn VideoCodecCapability>],
    implementation: &VideoCodecImplementation,
) -> Option<&'a dyn VideoCodecCapability> {
    capabilities
        .iter()
        .map(|capability| capability.as_ref())
        .find(|capability| capability.get_implementation() == implementation.clone())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrameSize {
    width: i32,
    height: i32,
}

fn align_down(value: i32, alignment: i32) -> i32 {
    if value <= 0 || alignment <= 1 {
        return value;
    }
    let aligned = value - (value % alignment);
    if aligned > 0 { aligned } else { value }
}

fn align_frame_size(
    size: FrameSize,
    horizontal_alignment: i32,
    vertical_alignment: i32,
) -> FrameSize {
    FrameSize {
        width: align_down(size.width, horizontal_alignment),
        height: align_down(size.height, vertical_alignment),
    }
}

fn apply_alignment_to_codec(
    codec: &mut VideoCodec,
    codec_type: VideoCodecType,
    horizontal_alignment: i32,
    vertical_alignment: i32,
) -> Option<FrameSize> {
    if codec.codec_type() != codec_type {
        return None;
    }

    let aligned_codec_size = align_frame_size(
        FrameSize {
            width: codec.width(),
            height: codec.height(),
        },
        horizontal_alignment,
        vertical_alignment,
    );
    if aligned_codec_size.width <= 0 || aligned_codec_size.height <= 0 {
        return None;
    }

    codec.set_width(aligned_codec_size.width);
    codec.set_height(aligned_codec_size.height);

    for index in 0..codec.number_of_simulcast_streams() {
        let Some(mut stream) = codec.simulcast_stream(index) else {
            continue;
        };
        let aligned_stream_size = align_frame_size(
            FrameSize {
                width: stream.width(),
                height: stream.height(),
            },
            horizontal_alignment,
            vertical_alignment,
        );
        if aligned_stream_size.width <= 0 || aligned_stream_size.height <= 0 {
            continue;
        }
        stream.set_width(aligned_stream_size.width);
        stream.set_height(aligned_stream_size.height);
    }

    Some(aligned_codec_size)
}

fn compute_center_crop(
    frame_width: i32,
    frame_height: i32,
    target_width: i32,
    target_height: i32,
) -> Option<(i32, i32, i32, i32)> {
    if frame_width <= 0 || frame_height <= 0 || target_width <= 0 || target_height <= 0 {
        return None;
    }

    let lhs = i64::from(frame_width) * i64::from(target_height);
    let rhs = i64::from(frame_height) * i64::from(target_width);
    let (mut crop_width, mut crop_height) = if lhs > rhs {
        (
            (i64::from(frame_height) * i64::from(target_width) / i64::from(target_height)) as i32,
            frame_height,
        )
    } else {
        (
            frame_width,
            (i64::from(frame_width) * i64::from(target_height) / i64::from(target_width)) as i32,
        )
    };

    crop_width = crop_width.clamp(1, frame_width);
    crop_height = crop_height.clamp(1, frame_height);
    if crop_width > 1 {
        crop_width &= !1;
    }
    if crop_height > 1 {
        crop_height &= !1;
    }
    if crop_width <= 0 || crop_height <= 0 {
        return None;
    }

    let mut offset_x = (frame_width - crop_width) / 2;
    let mut offset_y = (frame_height - crop_height) / 2;
    if offset_x > 0 {
        offset_x &= !1;
    }
    if offset_y > 0 {
        offset_y &= !1;
    }

    Some((offset_x.max(0), offset_y.max(0), crop_width, crop_height))
}

pub struct AlignmentEncoderAdapter {
    encoder: VideoEncoder,
    codec_type: VideoCodecType,
    horizontal_alignment: i32,
    vertical_alignment: i32,
    target_size: Option<FrameSize>,
}

impl AlignmentEncoderAdapter {
    pub fn new(
        encoder: VideoEncoder,
        codec_type: VideoCodecType,
        horizontal_alignment: i32,
        vertical_alignment: i32,
    ) -> Self {
        Self {
            encoder,
            codec_type,
            horizontal_alignment,
            vertical_alignment,
            target_size: None,
        }
    }

    fn build_aligned_frame(
        &self,
        frame: VideoFrameRef<'_>,
        target_size: FrameSize,
    ) -> Option<VideoFrame> {
        let frame_width = frame.width();
        let frame_height = frame.height();
        if frame_width <= 0 || frame_height <= 0 {
            return None;
        }
        if frame_width == target_size.width && frame_height == target_size.height {
            return Some(frame.to_owned());
        }

        let (offset_x, offset_y, crop_width, crop_height) = compute_center_crop(
            frame_width,
            frame_height,
            target_size.width,
            target_size.height,
        )?;

        let mut source_buffer = frame.buffer();
        let aligned_buffer = source_buffer.crop_and_scale(
            offset_x,
            offset_y,
            crop_width,
            crop_height,
            target_size.width,
            target_size.height,
        )?;

        let mut aligned_frame = frame.to_owned();
        aligned_frame.set_video_frame_buffer(&aligned_buffer);
        Some(aligned_frame)
    }
}

impl VideoEncoderHandler for AlignmentEncoderAdapter {
    fn init_encode(
        &mut self,
        codec: VideoCodecRef<'_>,
        settings: VideoEncoderSettingsRef<'_>,
    ) -> VideoCodecStatus {
        let mut codec_settings = codec.to_owned();
        self.target_size = apply_alignment_to_codec(
            &mut codec_settings,
            self.codec_type,
            self.horizontal_alignment,
            self.vertical_alignment,
        );
        self.encoder.init_encode(codec_settings.as_ref(), settings)
    }

    fn encode(
        &mut self,
        frame: VideoFrameRef<'_>,
        frame_types: Option<VideoFrameTypeVectorRef<'_>>,
    ) -> VideoCodecStatus {
        let Some(target_size) = self.target_size else {
            return self.encoder.encode(frame, frame_types);
        };
        let Some(aligned_frame) = self.build_aligned_frame(frame, target_size) else {
            return VideoCodecStatus::Error;
        };
        self.encoder.encode(aligned_frame.as_ref(), frame_types)
    }

    fn register_encode_complete_callback(
        &mut self,
        callback: Option<VideoEncoderEncodedImageCallbackRef<'_>>,
    ) -> VideoCodecStatus {
        self.encoder.register_encode_complete_callback(callback)
    }

    fn release(&mut self) -> VideoCodecStatus {
        self.encoder.release()
    }

    fn set_rates(&mut self, parameters: VideoEncoderRateControlParametersRef<'_>) {
        self.encoder.set_rates(parameters);
    }

    fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
        let mut info = self.encoder.get_encoder_info();
        let implementation_name = info.implementation_name().unwrap_or_default();
        if implementation_name.contains("AlignmentEncoderAdapter") {
            return info;
        }
        if implementation_name.is_empty() {
            info.set_implementation_name("AlignmentEncoderAdapter");
        } else {
            info.set_implementation_name(&format!("{implementation_name} AlignmentEncoderAdapter"));
        }
        info
    }
}

pub struct SimulcastCapabilityHelper {
    primary_factory: VideoEncoderFactory,
}

struct DelegatingVideoEncoderFactoryHandler<GetSupportedFormats, CreateEncoder> {
    get_supported_formats: GetSupportedFormats,
    create_encoder: CreateEncoder,
}

impl<GetSupportedFormats, CreateEncoder> VideoEncoderFactoryHandler
    for DelegatingVideoEncoderFactoryHandler<GetSupportedFormats, CreateEncoder>
where
    GetSupportedFormats: FnMut() -> Vec<SdpVideoFormat> + Send + 'static,
    CreateEncoder: for<'a> FnMut(EnvironmentRef<'a>, SdpVideoFormatRef<'a>) -> Option<VideoEncoder>
        + Send
        + 'static,
{
    fn get_supported_formats(&mut self) -> Vec<SdpVideoFormat> {
        (self.get_supported_formats)()
    }

    fn create(
        &mut self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        (self.create_encoder)(env, format)
    }
}

impl SimulcastCapabilityHelper {
    pub fn new(primary_factory: VideoEncoderFactory) -> Self {
        Self { primary_factory }
    }

    pub fn new_with_builder<GetSupportedFormats, CreateEncoder>(
        get_supported_formats: GetSupportedFormats,
        create_encoder: CreateEncoder,
    ) -> Self
    where
        GetSupportedFormats: FnMut() -> Vec<SdpVideoFormat> + Send + 'static,
        CreateEncoder: for<'a> FnMut(EnvironmentRef<'a>, SdpVideoFormatRef<'a>) -> Option<VideoEncoder>
            + Send
            + 'static,
    {
        let primary_factory =
            VideoEncoderFactory::new_with_handler(Box::new(DelegatingVideoEncoderFactoryHandler {
                get_supported_formats,
                create_encoder,
            }));
        Self { primary_factory }
    }

    pub fn get_supported_formats(&self) -> Vec<SdpVideoFormat> {
        self.primary_factory.get_supported_formats()
    }

    pub fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        Some(
            SimulcastEncoderAdapter::new(env, &self.primary_factory, None, format)
                .cast_to_video_encoder(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::video_codec_preference::PreferenceCodec;
    use shiguredo_webrtc::{ScalabilityMode, VideoDecoderHandler, VideoEncoderHandler};

    struct StubVideoEncoder;
    impl VideoEncoderHandler for StubVideoEncoder {}

    struct StubVideoDecoder;
    impl VideoDecoderHandler for StubVideoDecoder {}

    struct StubVideoEncoderWithInfoName;
    impl VideoEncoderHandler for StubVideoEncoderWithInfoName {
        fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
            let mut info = VideoEncoderEncoderInfo::new();
            info.set_implementation_name("StubEncoder");
            info
        }
    }

    struct MockCapability {
        implementation: VideoCodecImplementation,
        encoder_supported: Vec<VideoCodecType>,
        decoder_supported: Vec<VideoCodecType>,
        encoder_formats: Option<Vec<VideoCodecType>>,
        decoder_formats: Option<Vec<VideoCodecType>>,
    }

    impl MockCapability {
        fn new(
            implementation: VideoCodecImplementation,
            encoder_supported: Vec<VideoCodecType>,
            decoder_supported: Vec<VideoCodecType>,
        ) -> Self {
            Self {
                implementation,
                encoder_supported,
                decoder_supported,
                encoder_formats: None,
                decoder_formats: None,
            }
        }

        fn with_supported_formats(
            mut self,
            direction: CodecDirection,
            formats: Vec<VideoCodecType>,
        ) -> Self {
            match direction {
                CodecDirection::Encoder => self.encoder_formats = Some(formats),
                CodecDirection::Decoder => self.decoder_formats = Some(formats),
            }
            self
        }
    }

    impl VideoCodecCapability for MockCapability {
        fn get_implementation(&self) -> VideoCodecImplementation {
            self.implementation.clone()
        }

        fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
            let codec_types = match direction {
                CodecDirection::Encoder => self
                    .encoder_formats
                    .as_ref()
                    .unwrap_or(&self.encoder_supported),
                CodecDirection::Decoder => self
                    .decoder_formats
                    .as_ref()
                    .unwrap_or(&self.decoder_supported),
            };
            let mut formats = Vec::new();
            for codec_type in codec_types {
                let codec_name = codec_type
                    .as_str()
                    .expect("known codec type must be converted to codec name");
                formats.push(SdpVideoFormat::new(codec_name));
            }
            formats
        }

        fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
            match direction {
                CodecDirection::Encoder => self.encoder_supported.contains(&codec_type),
                CodecDirection::Decoder => self.decoder_supported.contains(&codec_type),
            }
        }

        fn resolve_sdp_format(
            &self,
            direction: CodecDirection,
            format: SdpVideoFormatRef<'_>,
        ) -> Option<SdpVideoFormat> {
            let codec_type = format
                .name()
                .ok()
                .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
            if !self.is_supported(direction, codec_type) {
                return None;
            }
            let parameters = format
                .to_owned()
                .parameters_mut()
                .iter()
                .collect::<HashMap<String, String>>();
            let requested_scalability_mode = format
                .scalability_modes()
                .iter()
                .find_map(|mode| mode.as_str().ok());
            match codec_type {
                VideoCodecType::H264 => {
                    let mut candidates = vec![
                        SdpVideoFormat::new_with_parameters(
                            "H264",
                            &HashMap::from([(
                                String::from("packetization-mode"),
                                String::from("1"),
                            )]),
                            &[ScalabilityMode::L1T2],
                        ),
                        SdpVideoFormat::new_with_parameters(
                            "H264",
                            &HashMap::from([(
                                String::from("packetization-mode"),
                                String::from("0"),
                            )]),
                            &[ScalabilityMode::L1T1],
                        ),
                    ];
                    let fallback = candidates.first().cloned()?;
                    for candidate in &mut candidates {
                        let params = candidate
                            .parameters_mut()
                            .iter()
                            .collect::<HashMap<String, String>>();
                        let params_match = parameters
                            .iter()
                            .all(|(k, v)| params.get(k).is_some_and(|value| value == v));
                        if !params_match {
                            continue;
                        }
                        let mode_match = match requested_scalability_mode.as_deref() {
                            Some(mode) => {
                                candidate.scalability_modes().iter().any(|candidate_mode| {
                                    candidate_mode.as_str().is_ok_and(|candidate_mode_text| {
                                        candidate_mode_text == mode
                                    })
                                })
                            }
                            None => true,
                        };
                        if mode_match {
                            return Some(candidate.clone());
                        }
                    }
                    Some(fallback)
                }
                VideoCodecType::Vp8 => Some(SdpVideoFormat::new("VP8")),
                _ => None,
            }
        }

        fn create_video_encoder(
            &self,
            _env: EnvironmentRef<'_>,
            format: SdpVideoFormatRef<'_>,
        ) -> Option<VideoEncoder> {
            let codec_type = format
                .name()
                .ok()
                .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
            if self.is_supported(CodecDirection::Encoder, codec_type) {
                Some(VideoEncoder::new_with_handler(Box::new(StubVideoEncoder)))
            } else {
                None
            }
        }

        fn create_video_decoder(
            &self,
            _env: EnvironmentRef<'_>,
            format: SdpVideoFormatRef<'_>,
        ) -> Option<VideoDecoder> {
            let codec_type = format
                .name()
                .ok()
                .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())?;
            if self.is_supported(CodecDirection::Decoder, codec_type) {
                Some(VideoDecoder::new_with_handler(Box::new(StubVideoDecoder)))
            } else {
                None
            }
        }
    }

    #[test]
    fn encoder_factory_uses_preference_order() {
        let preference = VideoCodecPreference::new(vec![
            PreferenceCodec::new(
                CodecDirection::Encoder,
                VideoCodecType::Vp8,
                VideoCodecImplementation::new("impl-b", "Implementation B"),
            ),
            PreferenceCodec::new(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("impl-a", "Implementation A"),
            ),
        ]);
        let capabilities: Vec<Box<dyn VideoCodecCapability>> = vec![
            Box::new(MockCapability::new(
                VideoCodecImplementation::new("impl-a", "Implementation A"),
                vec![VideoCodecType::H264],
                Vec::new(),
            )),
            Box::new(MockCapability::new(
                VideoCodecImplementation::new("impl-b", "Implementation B"),
                vec![VideoCodecType::Vp8],
                Vec::new(),
            )),
        ];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraVideoEncoderFactory::new(preference, shared);
        let formats = VideoEncoderFactoryHandler::get_supported_formats(&mut factory);
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].name().expect("name 取得失敗"), "VP8");
        assert_eq!(formats[1].name().expect("name 取得失敗"), "H264");
    }

    #[test]
    fn encoder_factory_uses_capability_formats_when_available() {
        let preference = VideoCodecPreference::new(vec![PreferenceCodec::new(
            CodecDirection::Encoder,
            VideoCodecType::H264,
            VideoCodecImplementation::new("impl-a", "Implementation A"),
        )]);
        let capabilities: Vec<Box<dyn VideoCodecCapability>> = vec![Box::new(
            MockCapability::new(
                VideoCodecImplementation::new("impl-a", "Implementation A"),
                vec![VideoCodecType::H264],
                Vec::new(),
            )
            .with_supported_formats(
                CodecDirection::Encoder,
                vec![VideoCodecType::Vp9, VideoCodecType::H264],
            ),
        )];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraVideoEncoderFactory::new(preference, shared);
        let formats = VideoEncoderFactoryHandler::get_supported_formats(&mut factory);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].name().expect("name 取得失敗"), "H264");
    }

    #[test]
    fn encoder_factory_uses_capability_formats_per_implementation_when_mixed() {
        let preference = VideoCodecPreference::new(vec![
            PreferenceCodec::new(
                CodecDirection::Encoder,
                VideoCodecType::Vp8,
                VideoCodecImplementation::new("impl-a", "Implementation A"),
            ),
            PreferenceCodec::new(
                CodecDirection::Encoder,
                VideoCodecType::H264,
                VideoCodecImplementation::new("impl-b", "Implementation B"),
            ),
        ]);
        let capabilities: Vec<Box<dyn VideoCodecCapability>> = vec![
            Box::new(
                MockCapability::new(
                    VideoCodecImplementation::new("impl-a", "Implementation A"),
                    vec![VideoCodecType::Vp8],
                    Vec::new(),
                )
                .with_supported_formats(
                    CodecDirection::Encoder,
                    vec![VideoCodecType::Av1, VideoCodecType::Vp8],
                ),
            ),
            Box::new(
                MockCapability::new(
                    VideoCodecImplementation::new("impl-b", "Implementation B"),
                    vec![VideoCodecType::H264],
                    Vec::new(),
                )
                .with_supported_formats(CodecDirection::Encoder, vec![VideoCodecType::H264]),
            ),
        ];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraVideoEncoderFactory::new(preference, shared);
        let formats = VideoEncoderFactoryHandler::get_supported_formats(&mut factory);
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].name().expect("name 取得失敗"), "VP8");
        assert_eq!(formats[1].name().expect("name 取得失敗"), "H264");
    }

    #[test]
    fn encoder_factory_ignores_resolve_when_capability_formats_missing_codec() {
        let preference = VideoCodecPreference::new(vec![PreferenceCodec::new(
            CodecDirection::Encoder,
            VideoCodecType::Vp8,
            VideoCodecImplementation::new("impl-a", "Implementation A"),
        )]);
        let capabilities: Vec<Box<dyn VideoCodecCapability>> = vec![Box::new(
            MockCapability::new(
                VideoCodecImplementation::new("impl-a", "Implementation A"),
                vec![VideoCodecType::Vp8],
                Vec::new(),
            )
            .with_supported_formats(CodecDirection::Encoder, vec![VideoCodecType::Av1]),
        )];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraVideoEncoderFactory::new(preference, shared);
        let formats = VideoEncoderFactoryHandler::get_supported_formats(&mut factory);
        assert!(formats.is_empty());
    }

    #[test]
    fn decoder_factory_create_requires_supported_codec_type() {
        let preference = VideoCodecPreference::new(vec![PreferenceCodec::new(
            CodecDirection::Decoder,
            VideoCodecType::H264,
            VideoCodecImplementation::new("impl-a", "Implementation A"),
        )]);
        let capabilities: Vec<Box<dyn VideoCodecCapability>> = vec![Box::new(MockCapability::new(
            VideoCodecImplementation::new("impl-a", "Implementation A"),
            Vec::new(),
            vec![VideoCodecType::H264],
        ))];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraVideoDecoderFactory::new(preference, shared);
        let env = shiguredo_webrtc::Environment::new();

        let mut unmatched = SdpVideoFormat::new_with_parameters(
            "H264",
            &HashMap::from([(String::from("packetization-mode"), String::from("0"))]),
            &[ScalabilityMode::L1T1],
        );
        unmatched.parameters_mut().set("packetization-mode", "0");
        assert!(
            VideoDecoderFactoryHandler::create(&mut factory, env.as_ref(), unmatched.as_ref())
                .is_some()
        );

        let mut matched = SdpVideoFormat::new_with_parameters(
            "H264",
            &HashMap::from([(String::from("packetization-mode"), String::from("1"))]),
            &[ScalabilityMode::L1T2],
        );
        matched.parameters_mut().set("packetization-mode", "1");
        assert!(
            VideoDecoderFactoryHandler::create(&mut factory, env.as_ref(), matched.as_ref())
                .is_some()
        );

        let vp8 = SdpVideoFormat::new("VP8");
        assert!(
            VideoDecoderFactoryHandler::create(&mut factory, env.as_ref(), vp8.as_ref()).is_none()
        );
    }

    #[test]
    fn alignment_updates_codec_and_simulcast_streams() {
        let mut codec = VideoCodec::new();
        codec.set_codec_type(VideoCodecType::Av1);
        codec.set_width(321);
        codec.set_height(181);
        codec.set_number_of_simulcast_streams(2);
        codec
            .simulcast_stream(0)
            .expect("simulcast stream 0 が必要")
            .set_width(321);
        codec
            .simulcast_stream(0)
            .expect("simulcast stream 0 が必要")
            .set_height(181);
        codec
            .simulcast_stream(1)
            .expect("simulcast stream 1 が必要")
            .set_width(161);
        codec
            .simulcast_stream(1)
            .expect("simulcast stream 1 が必要")
            .set_height(91);

        let aligned =
            apply_alignment_to_codec(&mut codec, VideoCodecType::Av1, 64, 16).expect("整列失敗");
        assert_eq!(
            aligned,
            FrameSize {
                width: 320,
                height: 176
            }
        );
        assert_eq!(codec.width(), 320);
        assert_eq!(codec.height(), 176);
        assert_eq!(
            codec
                .simulcast_stream(0)
                .expect("simulcast stream 0 が必要")
                .width(),
            320
        );
        assert_eq!(
            codec
                .simulcast_stream(0)
                .expect("simulcast stream 0 が必要")
                .height(),
            176
        );
        assert_eq!(
            codec
                .simulcast_stream(1)
                .expect("simulcast stream 1 が必要")
                .width(),
            128
        );
        assert_eq!(
            codec
                .simulcast_stream(1)
                .expect("simulcast stream 1 が必要")
                .height(),
            80
        );
    }

    #[test]
    fn alignment_is_not_applied_to_other_codec() {
        let mut codec = VideoCodec::new();
        codec.set_codec_type(VideoCodecType::H264);
        codec.set_width(321);
        codec.set_height(181);
        codec.set_number_of_simulcast_streams(1);
        codec
            .simulcast_stream(0)
            .expect("simulcast stream 0 が必要")
            .set_width(321);
        codec
            .simulcast_stream(0)
            .expect("simulcast stream 0 が必要")
            .set_height(181);

        assert!(apply_alignment_to_codec(&mut codec, VideoCodecType::Av1, 64, 16).is_none());
        assert_eq!(codec.width(), 321);
        assert_eq!(codec.height(), 181);
        assert_eq!(
            codec
                .simulcast_stream(0)
                .expect("simulcast stream 0 が必要")
                .width(),
            321
        );
        assert_eq!(
            codec
                .simulcast_stream(0)
                .expect("simulcast stream 0 が必要")
                .height(),
            181
        );
    }

    #[test]
    fn alignment_encoder_adapter_encoder_info_contains_adapter_name() {
        let base = VideoEncoder::new_with_handler(Box::new(StubVideoEncoderWithInfoName));
        let encoder = VideoEncoder::new_with_handler(Box::new(AlignmentEncoderAdapter::new(
            base,
            VideoCodecType::Av1,
            64,
            16,
        )));
        let info = encoder.get_encoder_info();
        let implementation_name = info
            .implementation_name()
            .expect("implementation_name の取得に失敗");
        assert!(
            implementation_name.contains("AlignmentEncoderAdapter"),
            "AlignmentEncoderAdapter を含む実装名が必要: {implementation_name}",
        );
        assert!(
            implementation_name.contains("StubEncoder"),
            "元の implementation_name を保持する必要があります: {implementation_name}",
        );
    }
}
