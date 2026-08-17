//! ビデオコーデックのエンコーダー/デコーダーファクトリと、
//! エンコーダーアダプター、simulcast ヘルパーを提供する。
use std::sync::{Arc, Mutex};

use shiguredo_webrtc::{
    EnvironmentRef, SdpVideoFormat, SdpVideoFormatRef, SimulcastEncoderAdapter, VideoCodec,
    VideoCodecRef, VideoCodecStatus, VideoCodecType, VideoDecoder, VideoDecoderFactoryHandler,
    VideoEncoder, VideoEncoderEncodedImageCallbackRef, VideoEncoderEncoderInfo,
    VideoEncoderFactory, VideoEncoderFactoryHandler, VideoEncoderHandler,
    VideoEncoderRateControlParametersRef, VideoEncoderSettingsRef, VideoFrame, VideoFrameRef,
    VideoFrameTypeVectorRef,
};

use crate::video_codec_capability::{CodecDirection, VideoCodecCapability, find_capability};
use crate::video_codec_preference::VideoCodecPreference;

type VideoCodecCapabilities = Vec<Box<dyn VideoCodecCapability>>;
type SharedVideoCodecCapabilities = Arc<Mutex<VideoCodecCapabilities>>;

/// [VideoCodecPreference] に基づき、利用可能なビデオエンコーダーを提供するファクトリ。
pub struct SoraVideoEncoderFactory {
    preference: VideoCodecPreference,
    capabilities: SharedVideoCodecCapabilities,
}

/// [VideoCodecPreference] に基づき、利用可能なビデオデコーダーを提供するファクトリ。
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
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
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
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
        let capability = find_capability(&capabilities, preference.implementation())?;
        let resolved = capability.resolve_sdp_format(CodecDirection::Encoder, format)?;
        capability.create_video_encoder(env, resolved.as_ref())
    }
}

impl VideoDecoderFactoryHandler for SoraVideoDecoderFactory {
    fn get_supported_formats(&mut self) -> Vec<SdpVideoFormat> {
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
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
        let capabilities = self
            .capabilities
            .lock()
            .expect("capabilities should not be poisoned");
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

fn align_down(value: i32, alignment: i32) -> Option<i32> {
    // alignment <= 0 は除算ゼロまたは無効な引数
    if alignment <= 0 {
        return None;
    }
    // value <= 0 は align 対象として意味をなさない。
    // alignment == 1 より先に判定することで、負の値を誤って返さない。
    if value <= 0 {
        return None;
    }
    // alignment == 1 は常に align 可能
    if alignment == 1 {
        return Some(value);
    }
    let aligned = value - (value % alignment);
    // align down の結果が 0 になることは数値演算としては異常ではないが、
    // このファイルでは幅・高さが 0 のコーデックは不正なため None を返す。
    if aligned > 0 { Some(aligned) } else { None }
}

fn apply_alignment_to_codec(
    codec: &mut VideoCodec,
    codec_type: VideoCodecType,
    horizontal_alignment: i32,
    vertical_alignment: i32,
) -> Option<(i32, i32)> {
    if codec.codec_type() != codec_type {
        return None;
    }

    // トップレベル codec のアライン結果を計算する。
    let aligned_codec_width = align_down(codec.width(), horizontal_alignment)?;
    let aligned_codec_height = align_down(codec.height(), vertical_alignment)?;

    // 全 simulcast stream のアライン結果を事前に計算する。
    // いずれか 1 つでも None なら codec に何も変更を加えずに None を返す。
    let mut stream_alignments: Vec<(usize, i32, i32)> = Vec::new();
    for index in 0..codec.number_of_simulcast_streams() {
        let Some(stream) = codec.simulcast_stream(index) else {
            continue;
        };
        let aligned_stream_width = align_down(stream.width(), horizontal_alignment)?;
        let aligned_stream_height = align_down(stream.height(), vertical_alignment)?;
        stream_alignments.push((index, aligned_stream_width, aligned_stream_height));
    }

    // 全要素がアライン可能な場合のみ、一括で適用する。
    // 部分状態（トップレベルだけ align 済み等）が発生しない。
    codec.set_width(aligned_codec_width);
    codec.set_height(aligned_codec_height);
    for (index, aligned_stream_width, aligned_stream_height) in stream_alignments {
        if let Some(mut stream) = codec.simulcast_stream(index) {
            stream.set_width(aligned_stream_width);
            stream.set_height(aligned_stream_height);
        }
    }

    Some((aligned_codec_width, aligned_codec_height))
}

/// エンコーダー固有の解像度アライメント制約を吸収するアダプター。
///
/// このアダプターを使うと、下流のエンコーダーに対して、
/// 入力フレームを指定されたアライメント制約に合わせたサイズに crop する。
///
/// アライメント時、align up ではなく align down する。
/// つまり 1080 サイズの入力映像を 16 でアライメントすると 1088 ではなく 1072 サイズになる。
///
/// 溢れた領域は削除されるため、画面端の情報が失われる可能性がある点に注意。
pub struct AlignmentEncoderAdapter {
    encoder: VideoEncoder,
    codec_type: VideoCodecType,
    horizontal_alignment: i32,
    vertical_alignment: i32,
    target_size: Option<(i32, i32)>,
}

impl AlignmentEncoderAdapter {
    /// 新しい `AlignmentEncoderAdapter` を生成する。
    ///
    /// 指定された水平・垂直アライメント制約に合わせて、
    /// 下流のエンコーダーに渡すフレームを crop する。
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
        target_width: i32,
        target_height: i32,
    ) -> Option<VideoFrame> {
        let frame_width = frame.width();
        let frame_height = frame.height();
        if frame_width <= 0 || frame_height <= 0 {
            return None;
        }
        if frame_width == target_width && frame_height == target_height {
            return Some(frame.to_owned());
        }
        if frame_width < target_width || frame_height < target_height {
            return None;
        }

        let mut offset_x = (frame_width - target_width) / 2;
        let mut offset_y = (frame_height - target_height) / 2;
        // I420/NV12 などのクロップは偶数ピクセル単位でしか行えないので偶数に丸める
        offset_x &= !1;
        offset_y &= !1;

        let mut source_buffer = frame.buffer();
        let aligned_buffer = source_buffer.crop_and_scale(
            offset_x,
            offset_y,
            target_width,
            target_height,
            target_width,
            target_height,
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
        // WebRTC 側の InitEncode は元々 const VideoCodec* を受けるため、
        // 入力設定は直接変更せずコピーを編集して下流へ渡す。
        let mut codec_settings = codec.to_owned();
        self.target_size = apply_alignment_to_codec(
            &mut codec_settings,
            self.codec_type,
            self.horizontal_alignment,
            self.vertical_alignment,
        );
        // アライン不能な場合は下流エンコーダーを初期化せずにエラーを返す。
        // 後続の encode 呼び出しでも target_size == None により
        // VideoCodecStatus::Error が返るため、非アライン解像度が下流に
        // 届く経路は存在しない。
        if self.target_size.is_none() {
            return VideoCodecStatus::Error;
        }
        self.encoder.init_encode(codec_settings.as_ref(), settings)
    }

    fn encode(
        &mut self,
        frame: VideoFrameRef<'_>,
        frame_types: Option<VideoFrameTypeVectorRef<'_>>,
    ) -> VideoCodecStatus {
        // target_size が None の場合:
        // - init_encode で apply_alignment_to_codec が None を返した (アライン不能)
        // - または codec_type 不一致 (アダプターが対象外のコーデックに適用された)
        // いずれも下流エンコーダーに非アライン解像度を渡さないよう Error を返す。
        let Some((target_width, target_height)) = self.target_size else {
            return VideoCodecStatus::Error;
        };
        let Some(aligned_frame) = self.build_aligned_frame(frame, target_width, target_height)
        else {
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

/// Simulcast に対応するエンコーダーを生成するためのヘルパー。
///
/// ベースとなる [VideoEncoderFactory] を受け取り、そのファクトリで作成したエンコーダーを
/// `SimulcastEncoderAdapter` でラップして返す。
/// これにより、ソフトウェアエンコーダーやハードウェアエンコーダーに対して
/// simulcast 機能を追加できる。
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
    /// 既存の [VideoEncoderFactory] から `SimulcastCapabilityHelper` を生成する。
    pub fn new(primary_factory: VideoEncoderFactory) -> Self {
        Self { primary_factory }
    }

    /// クロージャベースで `SimulcastCapabilityHelper` を生成する。
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

    /// 内部の [VideoEncoderFactory] がサポートする SDP フォーマット一覧を返す。
    pub fn get_supported_formats(&self) -> Vec<SdpVideoFormat> {
        self.primary_factory.get_supported_formats()
    }

    /// 指定された SDP フォーマットに対応する [VideoEncoder] を生成する。
    ///
    /// 生成されたエンコーダーは `SimulcastEncoderAdapter` でラップされているため、
    /// simulcast に対応する。
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

/// [SdpVideoFormatRef] から [VideoCodecType] を取得する。
///
/// format の name を解析し、対応する `VideoCodecType` があれば返す。
pub fn codec_type_from_format(format: &SdpVideoFormatRef<'_>) -> Option<VideoCodecType> {
    let format_name = format.name().ok()?;
    VideoCodecType::try_from(format_name.as_str()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestVideoCodecCapability;
    use crate::video_codec_capability::VideoCodecImplementation;
    use crate::video_codec_preference::PreferenceCodec;

    // VideoEncoderHandler を最小限に実装し、implementation name を返すテスト専用の型。
    struct NoopVideoEncoderWithInfoName;
    impl VideoEncoderHandler for NoopVideoEncoderWithInfoName {
        fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
            let mut info = VideoEncoderEncoderInfo::new();
            info.set_implementation_name("NoopEncoder");
            info
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
            Box::new(TestVideoCodecCapability::new(
                VideoCodecImplementation::new("impl-a", "Implementation A"),
                vec![VideoCodecType::H264],
                Vec::new(),
            )),
            Box::new(TestVideoCodecCapability::new(
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
        let capabilities: Vec<Box<dyn VideoCodecCapability>> =
            vec![Box::new(TestVideoCodecCapability::new(
                VideoCodecImplementation::new("impl-a", "Implementation A"),
                vec![VideoCodecType::Vp9, VideoCodecType::H264],
                Vec::new(),
            ))];

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
            Box::new(TestVideoCodecCapability::new(
                VideoCodecImplementation::new("impl-a", "Implementation A"),
                vec![VideoCodecType::Av1, VideoCodecType::Vp8],
                Vec::new(),
            )),
            Box::new(TestVideoCodecCapability::new(
                VideoCodecImplementation::new("impl-b", "Implementation B"),
                vec![VideoCodecType::H264],
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
    fn encoder_factory_ignores_resolve_when_capability_formats_missing_codec() {
        let preference = VideoCodecPreference::new(vec![PreferenceCodec::new(
            CodecDirection::Encoder,
            VideoCodecType::Vp8,
            VideoCodecImplementation::new("impl-a", "Implementation A"),
        )]);
        let capabilities: Vec<Box<dyn VideoCodecCapability>> =
            vec![Box::new(TestVideoCodecCapability::new(
                VideoCodecImplementation::new("impl-a", "Implementation A"),
                vec![VideoCodecType::Av1],
                Vec::new(),
            ))];

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
        let capabilities: Vec<Box<dyn VideoCodecCapability>> =
            vec![Box::new(TestVideoCodecCapability::new(
                VideoCodecImplementation::new("impl-a", "Implementation A"),
                Vec::new(),
                vec![VideoCodecType::H264],
            ))];

        let shared = Arc::new(Mutex::new(capabilities));
        let mut factory = SoraVideoDecoderFactory::new(preference, shared);
        let env = shiguredo_webrtc::Environment::new();

        // サポート済みの H264 は生成でき、未サポートの VP8 は生成できないことを検証する。
        let h264 = SdpVideoFormat::new("H264");
        assert!(
            VideoDecoderFactoryHandler::create(&mut factory, env.as_ref(), h264.as_ref()).is_some()
        );

        let vp8 = SdpVideoFormat::new("VP8");
        assert!(
            VideoDecoderFactoryHandler::create(&mut factory, env.as_ref(), vp8.as_ref()).is_none()
        );
    }

    #[test]
    fn align_down_normal_noop() {
        assert_eq!(align_down(320, 16), Some(320));
    }

    #[test]
    fn align_down_normal_round_down() {
        assert_eq!(align_down(321, 16), Some(320));
    }

    #[test]
    fn align_down_boundary_value_equals_alignment() {
        assert_eq!(align_down(16, 16), Some(16));
    }

    #[test]
    fn align_down_unable_to_align() {
        assert_eq!(align_down(15, 16), None);
    }

    #[test]
    fn align_down_value_zero() {
        assert_eq!(align_down(0, 16), None);
    }

    #[test]
    fn align_down_negative_value() {
        assert_eq!(align_down(-1, 16), None);
    }

    #[test]
    fn align_down_alignment_one_always_alignable() {
        assert_eq!(align_down(16, 1), Some(16));
    }

    #[test]
    fn align_down_alignment_zero_invalid() {
        assert_eq!(align_down(16, 0), None);
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
        assert_eq!(aligned, (320, 176));
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
    fn alignment_rejects_partial_simulcast_stream_failure() {
        let mut codec = VideoCodec::new();
        codec.set_codec_type(VideoCodecType::Av1);
        codec.set_width(320);
        codec.set_height(180);
        codec.set_number_of_simulcast_streams(2);
        codec
            .simulcast_stream(0)
            .expect("simulcast stream 0 が必要")
            .set_width(320);
        codec
            .simulcast_stream(0)
            .expect("simulcast stream 0 が必要")
            .set_height(180);
        codec
            .simulcast_stream(1)
            .expect("simulcast stream 1 が必要")
            .set_width(15);
        codec
            .simulcast_stream(1)
            .expect("simulcast stream 1 が必要")
            .set_height(10);

        // stream 1 が alignment=16 に満たないため、関数全体が None を返す。
        // codec の状態は変更されない。
        let result = apply_alignment_to_codec(&mut codec, VideoCodecType::Av1, 16, 16);
        assert!(
            result.is_none(),
            "stream 1 がアライン不能のため None になるべき"
        );
        assert_eq!(codec.width(), 320);
        assert_eq!(codec.height(), 180);
        assert_eq!(
            codec
                .simulcast_stream(0)
                .expect("simulcast stream 0 が必要")
                .width(),
            320,
            "stream 0 の幅は変更されていない"
        );
        assert_eq!(
            codec
                .simulcast_stream(0)
                .expect("simulcast stream 0 が必要")
                .height(),
            180,
            "stream 0 の高さは変更されていない"
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
        let base = VideoEncoder::new_with_handler(Box::new(NoopVideoEncoderWithInfoName));
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
            implementation_name.contains("NoopEncoder"),
            "元の implementation_name を保持する必要があります: {implementation_name}",
        );
    }
}
