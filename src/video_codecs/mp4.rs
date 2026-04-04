//! # MP4 パススルー送信
//!
//! MP4 ファイルからエンコード済みビデオフレームを抽出し、再エンコードなしに
//! WebRTC で送信する機能。以下の 4 つのコンポーネントで構成される:
//!
//! 1. Mp4SampleReader     - MP4 ファイルを読み込み、ビデオサンプルを抽出する
//! 2. Mp4PassthroughEncoder - WebRTC のエンコーダーインターフェースを実装し、エンコード済みデータをそのまま出力する
//! 3. Mp4PassthroughVideoCodecCapability - パススルーエンコーダーを WebRTC のコーデックパイプラインに登録する
//! 4. Mp4VideoCapturer - フレームペーシングを行い、MP4 のタイミングに従ってフレームを WebRTC に供給する
//!
//! データフロー:
//!   Mp4SampleReader --[Mp4EncodedSample を内包した VideoFrame]--> Mp4PassthroughEncoder --> WebRTC RTP
//!                                         (native VideoFrameBuffer)
//!
//! 対応コーデック: H.264, H.265, VP8, VP9, AV1
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use shiguredo_webrtc::{
    AdaptFrameResult, AdaptedVideoTrackSource, CodecSpecificInfo, EncodedImage, EncodedImageBuffer,
    H264PacketizationMode, I420Buffer, SdpVideoFormat, SdpVideoFormatRef, TimestampAligner,
    VideoCodecRef, VideoCodecStatus, VideoCodecType, VideoDecoder, VideoEncoder,
    VideoEncoderEncodedImageCallbackPtr, VideoEncoderEncodedImageCallbackRef,
    VideoEncoderEncodedImageCallbackResultError, VideoEncoderEncoderInfo, VideoEncoderHandler,
    VideoEncoderRateControlParametersRef, VideoEncoderSettingsRef, VideoFrame, VideoFrameBuffer,
    VideoFrameBufferHandler, VideoFrameRef, VideoFrameType, VideoFrameTypeVectorRef,
    VideoTrackSource, rtc_log_info, rtc_log_warning,
};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

#[derive(Debug)]
pub enum Mp4Error {
    Io(io::Error),
    Demux(shiguredo_mp4::demux::DemuxError),
    NoVideoTrack,
    NoVideoSamples,
    UnsupportedVideoCodec,
}

impl std::fmt::Display for Mp4Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read MP4 file: {err}"),
            Self::Demux(err) => write!(f, "failed to demux MP4 file: {err}"),
            Self::NoVideoTrack => f.write_str("no video track found in MP4"),
            Self::NoVideoSamples => f.write_str("no video samples found in MP4"),
            Self::UnsupportedVideoCodec => {
                f.write_str("unsupported MP4 video codec: expected H.264, H.265, VP8, VP9, or AV1")
            }
        }
    }
}

impl std::error::Error for Mp4Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Demux(err) => Some(err),
            Self::NoVideoTrack | Self::NoVideoSamples | Self::UnsupportedVideoCodec => None,
        }
    }
}

impl From<io::Error> for Mp4Error {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<shiguredo_mp4::demux::DemuxError> for Mp4Error {
    fn from(err: shiguredo_mp4::demux::DemuxError) -> Self {
        Self::Demux(err)
    }
}

type Result<T> = std::result::Result<T, Mp4Error>;

/// MP4 から抽出したエンコード済みビデオサンプル。
///
/// `Mp4SampleReader` が生成し、native `VideoFrameBuffer` に保持されて
/// `Mp4PassthroughEncoder` に渡される。
pub struct Mp4EncodedSample {
    /// エンコード済みフレームデータ。
    /// H.264/H.265 の場合は Annex B 形式に変換済み。
    /// VP8/VP9/AV1 の場合は MP4 から抽出したそのまま。
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub width: u32,
    pub height: u32,
    pub codec_type: VideoCodecType,
}

impl VideoFrameBufferHandler for Mp4EncodedSample {
    fn width(&self) -> i32 {
        self.width as i32
    }

    fn height(&self) -> i32 {
        self.height as i32
    }

    fn to_i420(&mut self) -> Option<I420Buffer> {
        None
    }
}

/// MP4 のビデオトラックから抽出したコーデック情報。
///
/// SampleEntry (stsd ボックス) から取得する。
struct Mp4VideoTrackInfo {
    codec_type: VideoCodecType,
    width: u16,
    height: u16,
    /// MP4 のタイムスケール (1 秒あたりのタイムスタンプ単位数)。
    /// duration をマイクロ秒に変換する際に使用する。
    timescale: u32,
    /// H.264 の SPS/PPS または H.265 の VPS/SPS/PPS (Annex B 形式)。
    /// キーフレーム送信時にフレームデータの先頭に付与する。
    /// VP8/VP9/AV1 では `None`。
    parameter_sets: Option<Vec<u8>>,
}

/// MP4 ファイルからビデオサンプルを読み出すリーダー。
///
/// コンストラクタでファイル全体をメモリに読み込み、全サンプルのメタデータを事前解析する。
/// `get_sample()` でインデックス指定でサンプルを取得できる。
/// ファイルデータを保持し続けるため、サンプル取得時にディスク I/O は発生しない。
pub struct Mp4SampleReader {
    /// MP4 ファイル全体のバイトデータ。
    file_data: Vec<u8>,
    track_info: Mp4VideoTrackInfo,
    /// 各サンプルのメタデータ: (data_offset, data_size, keyframe, timestamp, duration)。
    /// data_offset と data_size は file_data 内の位置を指す。
    /// timestamp と duration は MP4 のタイムスケール単位。
    samples: Vec<(u64, usize, bool, u64, u32)>,
    /// 各フレームの累積再生時刻 (マイクロ秒)。
    /// cumulative_us[0] = 0, cumulative_us[i] = フレーム 0..i の合計再生時間。
    /// 長さは samples.len() + 1 で、末尾が動画全体の長さ。
    /// フレームペーシングで絶対時刻ベースの待機に使用する。
    cumulative_us: Vec<u64>,
}

impl Mp4SampleReader {
    /// MP4 ファイルを読み込み、ビデオトラックの全サンプルを事前解析する。
    ///
    /// ファイル全体をメモリに保持するため、大きなファイルではメモリ使用量に注意。
    pub fn new(path: &str) -> Result<Self> {
        use shiguredo_mp4::demux::{Input, Mp4FileDemuxer};

        let file_data = std::fs::read(path)?;

        let mut demuxer = Mp4FileDemuxer::new();

        // shiguredo_mp4 のデマルチプレクサにファイルデータを供給する。
        // required_input() が要求する範囲のデータを順次渡すことで、
        // ボックス構造の解析が進む。
        while let Some(required) = demuxer.required_input() {
            let start = required.position as usize;
            let end = match required.size {
                Some(size) => (start + size).min(file_data.len()),
                None => file_data.len(),
            };
            let data = &file_data[start..end];
            demuxer.handle_input(Input {
                position: required.position,
                data,
            });
        }

        let tracks = demuxer.tracks()?;

        // 最初に見つかったビデオトラックを使用する (音声トラックは無視)。
        let video_track = tracks
            .iter()
            .find(|t| t.kind == shiguredo_mp4::TrackKind::Video)
            .ok_or(Mp4Error::NoVideoTrack)?;

        let video_track_id = video_track.track_id;
        let timescale = video_track.timescale.get();

        // 全サンプルを順次読み出す。
        // 最初のサンプルの sample_entry からコーデック情報 (解像度、parameter sets 等) を取得する。
        let mut track_info: Option<Mp4VideoTrackInfo> = None;
        let mut samples = Vec::new();

        loop {
            let Some(sample) = demuxer.next_sample()? else {
                break;
            };

            // 音声など他トラックのサンプルはスキップする。
            if sample.track.track_id != video_track_id {
                continue;
            }

            // 最初のサンプルの sample_entry からコーデック情報を取得する。
            if track_info.is_none()
                && let Some(entry) = sample.sample_entry
            {
                track_info = Some(Self::extract_track_info(entry, timescale)?);
            }

            samples.push((
                sample.data_offset,
                sample.data_size,
                sample.keyframe,
                sample.timestamp,
                sample.duration,
            ));
        }

        let track_info = track_info.ok_or(Mp4Error::NoVideoSamples)?;

        if samples.is_empty() {
            return Err(Mp4Error::NoVideoSamples);
        }

        // 累積再生時刻テーブルを事前計算する。
        // フレームペーシングで「次のフレームをいつ送るべきか」を O(1) で求めるため。
        // thread::sleep の相対待ちでは処理時間の累積ドリフトが発生するが、
        // このテーブルを使って Instant ベースの絶対時刻待ちを行うことで防止する。
        let timescale = track_info.timescale as u64;
        let mut cumulative_us = Vec::with_capacity(samples.len() + 1);
        let mut acc: u64 = 0;
        cumulative_us.push(0);
        for &(_, _, _, _, duration) in &samples {
            acc += duration as u64;
            cumulative_us.push((acc * 1_000_000) / timescale);
        }

        Ok(Self {
            file_data,
            track_info,
            samples,
            cumulative_us,
        })
    }

    /// SampleEntry からコーデック種別、解像度、parameter sets を抽出する。
    ///
    /// H.264: AvccBox から SPS/PPS を Annex B 形式で取得。
    /// H.265: HvccBox から VPS/SPS/PPS を Annex B 形式で取得。
    /// VP8/VP9/AV1: parameter sets は不要 (フレームデータに内包されている)。
    fn extract_track_info(
        entry: &shiguredo_mp4::boxes::SampleEntry,
        timescale: u32,
    ) -> Result<Mp4VideoTrackInfo> {
        use shiguredo_mp4::boxes::SampleEntry;

        match entry {
            SampleEntry::Avc1(avc1) => {
                let (width, height) = (avc1.visual.width, avc1.visual.height);
                // H.264 の SPS (Sequence Parameter Set) と PPS (Picture Parameter Set) を
                // Annex B 形式 (0x00000001 プレフィックス付き) で結合する。
                // デコーダーはキーフレームの前にこれらを受け取る必要がある。
                let mut parameter_sets = Vec::new();
                for sps in &avc1.avcc_box.sps_list {
                    parameter_sets.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                    parameter_sets.extend_from_slice(sps);
                }
                for pps in &avc1.avcc_box.pps_list {
                    parameter_sets.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                    parameter_sets.extend_from_slice(pps);
                }
                Ok(Mp4VideoTrackInfo {
                    codec_type: VideoCodecType::H264,
                    width,
                    height,
                    timescale,
                    parameter_sets: Some(parameter_sets),
                })
            }
            // H.265 には hev1 と hvc1 の 2 種類の SampleEntry がある。
            // hev1: parameter sets がサンプルデータ内にも含まれる (帯域内シグナリング)。
            // hvc1: parameter sets は SampleEntry のみに含まれる (帯域外シグナリング)。
            // どちらの場合も HvccBox から parameter sets を抽出して使用する。
            SampleEntry::Hev1(hev1) => {
                let (width, height) = (hev1.visual.width, hev1.visual.height);
                let parameter_sets = Self::extract_hevc_parameter_sets(&hev1.hvcc_box);
                Ok(Mp4VideoTrackInfo {
                    codec_type: VideoCodecType::H265,
                    width,
                    height,
                    timescale,
                    parameter_sets: Some(parameter_sets),
                })
            }
            SampleEntry::Hvc1(hvc1) => {
                let (width, height) = (hvc1.visual.width, hvc1.visual.height);
                let parameter_sets = Self::extract_hevc_parameter_sets(&hvc1.hvcc_box);
                Ok(Mp4VideoTrackInfo {
                    codec_type: VideoCodecType::H265,
                    width,
                    height,
                    timescale,
                    parameter_sets: Some(parameter_sets),
                })
            }
            SampleEntry::Vp08(vp08) => Ok(Mp4VideoTrackInfo {
                codec_type: VideoCodecType::Vp8,
                width: vp08.visual.width,
                height: vp08.visual.height,
                timescale,
                parameter_sets: None,
            }),
            SampleEntry::Vp09(vp09) => Ok(Mp4VideoTrackInfo {
                codec_type: VideoCodecType::Vp9,
                width: vp09.visual.width,
                height: vp09.visual.height,
                timescale,
                parameter_sets: None,
            }),
            SampleEntry::Av01(av01) => Ok(Mp4VideoTrackInfo {
                codec_type: VideoCodecType::Av1,
                width: av01.visual.width,
                height: av01.visual.height,
                timescale,
                parameter_sets: None,
            }),
            _ => Err(Mp4Error::UnsupportedVideoCodec),
        }
    }

    /// HEVC の HvccBox から VPS/SPS/PPS を Annex B 形式で抽出する。
    ///
    /// HvccBox の nalu_arrays には NAL ユニット種別ごとに配列が格納されている。
    /// VPS (Video Parameter Set), SPS, PPS がそれぞれ別の配列に入っている。
    /// 全てを Annex B スタートコード (0x00000001) 付きで結合して返す。
    fn extract_hevc_parameter_sets(hvcc: &shiguredo_mp4::boxes::HvccBox) -> Vec<u8> {
        let mut parameter_sets = Vec::new();
        for array in &hvcc.nalu_arrays {
            for nalu in &array.nalus {
                parameter_sets.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                parameter_sets.extend_from_slice(nalu);
            }
        }
        parameter_sets
    }

    /// サンプル数を返す。
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// サンプルが 0 件かどうかを返す。
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// この MP4 のビデオコーデック種別を返す。
    pub fn codec_type(&self) -> VideoCodecType {
        self.track_info.codec_type
    }

    /// 指定インデックスのサンプルデータを取得する。
    ///
    /// H.264/H.265 の場合:
    /// - MP4 内の AVCC/HVCC 形式 (4 バイト長さプレフィックス) を
    ///   Annex B 形式 (0x00000001 スタートコード) に変換する。
    /// - キーフレームの場合は先頭に parameter sets (SPS/PPS 等) を付与する。
    ///
    /// VP8/VP9/AV1 の場合:
    /// - MP4 から抽出したデータをそのまま使用する。
    fn get_sample(&self, index: usize) -> Mp4EncodedSample {
        let (data_offset, data_size, keyframe, _, _) = self.samples[index];
        let raw_data = &self.file_data[data_offset as usize..data_offset as usize + data_size];

        let data = match self.track_info.codec_type {
            VideoCodecType::H264 | VideoCodecType::H265 => {
                let mut annex_b = Vec::new();
                if keyframe && let Some(ref ps) = self.track_info.parameter_sets {
                    annex_b.extend_from_slice(ps);
                }
                annex_b.extend_from_slice(&length_prefixed_nalu_to_annex_b(raw_data));
                annex_b
            }
            _ => raw_data.to_vec(),
        };

        Mp4EncodedSample {
            data,
            is_keyframe: keyframe,
            width: self.track_info.width as u32,
            height: self.track_info.height as u32,
            codec_type: self.track_info.codec_type,
        }
    }

    /// 先頭からフレーム index までの累積再生時間をマイクロ秒で返す。
    /// index=0 なら 0、index=len() なら動画全体の長さ。
    fn cumulative_duration_us(&self, index: usize) -> u64 {
        self.cumulative_us[index]
    }
}

/// 長さプレフィックス付き NAL ユニットを Annex B 形式に変換する。
///
/// MP4 内の H.264/H.265 フレームデータは AVCC/HVCC 形式で格納されている:
///   [4 バイト NAL 長][NAL データ][4 バイト NAL 長][NAL データ]...
///
/// WebRTC (RTP) では Annex B 形式が期待される:
///   [0x00 0x00 0x00 0x01][NAL データ][0x00 0x00 0x00 0x01][NAL データ]...
///
/// この関数は 4 バイトの長さプレフィックスを 4 バイトのスタートコードに置き換える。
fn length_prefixed_nalu_to_annex_b(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len());
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let nal_size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;
        if offset + nal_size > data.len() {
            break;
        }
        result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        result.extend_from_slice(&data[offset..offset + nal_size]);
        offset += nal_size;
    }
    result
}

/// MP4 パススルーエンコーダー。
///
/// WebRTC の `VideoEncoderHandler` インターフェースを実装する。
/// 実際のエンコード処理は行わず、`VideoFrame` の native `VideoFrameBuffer` から
/// 事前エンコード済みのサンプルを取り出して `EncodedImage` として WebRTC に渡す。
///
/// `has_trusted_rate_controller=true` を設定することで、
/// WebRTC のビットレート制御がこのエンコーダーに対して介入しないようにする。
/// パススルーなのでビットレートの調整は不可能であり、
/// 代わりに `--video-bit-rate` で十分な帯域を確保する必要がある。
struct Mp4PassthroughEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
}

impl VideoEncoderHandler for Mp4PassthroughEncoder {
    fn init_encode(
        &mut self,
        codec: VideoCodecRef<'_>,
        _settings: VideoEncoderSettingsRef<'_>,
    ) -> VideoCodecStatus {
        rtc_log_info!(
            "MP4Passthrough: init_encode() codec_type={:?} {}x{} bitrate={}kbps",
            codec.codec_type(),
            codec.width(),
            codec.height(),
            codec.start_bitrate_kbps()
        );
        VideoCodecStatus::Ok
    }

    fn encode(
        &mut self,
        frame: VideoFrameRef<'_>,
        _frame_types: Option<VideoFrameTypeVectorRef<'_>>,
    ) -> VideoCodecStatus {
        let callback = match self.callback {
            Some(callback) => callback,
            None => return VideoCodecStatus::Uninitialized,
        };

        let frame_buffer = frame.buffer();
        // 安全性: encode() 呼び出し中の参照のみを取得し、同一実体への同時アクセスは行わない。
        let sample = match unsafe { frame_buffer.as_native_ref::<Mp4EncodedSample>() } {
            Some(sample) => sample,
            None => {
                rtc_log_warning!(
                    "MP4Passthrough: failed to get Mp4EncodedSample from frame buffer"
                );
                return VideoCodecStatus::Error;
            }
        };

        rtc_log_info!(
            "MP4Passthrough: encode() keyframe={} size={} bytes",
            sample.is_keyframe,
            sample.data.len()
        );

        // EncodedImage を構築して WebRTC に渡す。
        let mut encoded_image = EncodedImage::new();
        let encoded_buffer = EncodedImageBuffer::from_bytes(&sample.data);
        encoded_image.set_encoded_data(&encoded_buffer);
        encoded_image.set_rtp_timestamp(frame.rtp_timestamp());
        encoded_image.set_encoded_width(sample.width);
        encoded_image.set_encoded_height(sample.height);
        encoded_image.set_frame_type(if sample.is_keyframe {
            VideoFrameType::Key
        } else {
            VideoFrameType::Delta
        });

        // H.264 の場合はパケタイゼーションモードと IDR フレームフラグを設定する。
        // H.265/VP8/VP9/AV1 はコーデック種別の設定のみで十分。
        let mut codec_specific_info = CodecSpecificInfo::new();
        codec_specific_info.set_codec_type(sample.codec_type);
        if sample.codec_type == VideoCodecType::H264 {
            codec_specific_info.set_h264_packetization_mode(H264PacketizationMode::NonInterleaved);
            codec_specific_info.set_h264_idr_frame(sample.is_keyframe);
        }

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
        rtc_log_info!("MP4Passthrough: release()");
        self.callback = None;
        VideoCodecStatus::Ok
    }

    /// WebRTC からのビットレート変更通知。
    /// パススルーエンコーダーではビットレートの調整はできないのでログ出力のみ行う。
    fn set_rates(&mut self, parameters: VideoEncoderRateControlParametersRef<'_>) {
        rtc_log_info!(
            "MP4Passthrough: set_rates() bitrate={}bps fps={}",
            parameters.bitrate_sum_bps(),
            parameters.framerate_fps()
        );
    }

    fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
        let mut info = VideoEncoderEncoderInfo::new();
        info.set_implementation_name("MP4Passthrough");
        info.set_is_hardware_accelerated(false);
        // has_trusted_rate_controller=true にすることで、WebRTC の帯域推定 (BWE) が
        // このエンコーダーにビットレート変更を要求しなくなる。
        // パススルーでは事前エンコード済みデータを送るため、レート制御は不可能。
        info.set_has_trusted_rate_controller(true);
        info
    }
}

/// MP4 パススルー用の VideoCodecCapability。
///
/// WebRTC のコーデックパイプラインにパススルーエンコーダーを登録するためのアダプター。
/// MP4 から検出されたコーデック種別のみをサポートし、デコーダーは提供しない (送信専用)。
pub struct Mp4PassthroughVideoCodecCapability {
    /// MP4 から検出されたコーデック種別。
    codec_type: VideoCodecType,
}

impl Mp4PassthroughVideoCodecCapability {
    pub fn new(codec_type: VideoCodecType) -> Self {
        Self { codec_type }
    }
}

impl VideoCodecCapability for Mp4PassthroughVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("mp4-passthrough", "MP4 Passthrough")
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        if direction != CodecDirection::Encoder {
            return Vec::new();
        }
        match self.codec_type {
            VideoCodecType::H264 => {
                let mut format = SdpVideoFormat::new("H264");
                format.parameters_mut().set("packetization-mode", "1");
                vec![format]
            }
            VideoCodecType::H265 => vec![SdpVideoFormat::new("H265")],
            VideoCodecType::Vp8 => vec![SdpVideoFormat::new("VP8")],
            VideoCodecType::Vp9 => vec![SdpVideoFormat::new("VP9")],
            VideoCodecType::Av1 => vec![SdpVideoFormat::new("AV1")],
            _ => Vec::new(),
        }
    }

    fn create_video_encoder(
        &self,
        _env: shiguredo_webrtc::EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        let Ok(format_name) = format.name() else {
            return None;
        };
        let Ok(format_codec_type) = VideoCodecType::try_from(format_name.as_str()) else {
            return None;
        };
        if format_codec_type != self.codec_type {
            return None;
        }
        Some(VideoEncoder::new_with_handler(Box::new(
            Mp4PassthroughEncoder { callback: None },
        )))
    }

    /// デコーダーは提供しない (パススルーは送信専用)。
    fn create_video_decoder(
        &self,
        _env: shiguredo_webrtc::EnvironmentRef<'_>,
        _format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        None
    }
}

/// MP4 ファイルからビデオフレームを送信するキャプチャラー。
///
/// 専用スレッドで MP4 のフレームタイミングに従ってサンプルを供給する。
/// WebRTC のエンコーダーパイプラインは `VideoTrackSource::on_frame()` を起点に
/// `encode()` を呼び出すため、サンプルを内包した native `VideoFrameBuffer` を送る。
///
/// フレームペーシングは `Instant` ベースの絶対時刻待ちで行い、
/// 処理時間の累積ドリフトを防止する。
/// MP4 の末尾に到達すると先頭に戻りループ再生する。
pub struct Mp4VideoCapturer {
    video_source: VideoTrackSource,
    /// フィーダースレッドへの停止シグナル。
    stop: Arc<AtomicBool>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl Mp4VideoCapturer {
    pub fn new(reader: Mp4SampleReader) -> Result<Self> {
        let width = reader.track_info.width as i32;
        let height = reader.track_info.height as i32;

        let source = AdaptedVideoTrackSource::new();
        let video_source = source.cast_to_video_track_source();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();

        let thread_handle = thread::spawn(move || {
            let mut source = source;
            let mut aligner = TimestampAligner::new();

            loop {
                // ループ再生の先頭で基準時刻を記録する。
                // 各フレームの送信タイミングはこの基準時刻からの累積オフセットで決まる。
                let loop_start = std::time::Instant::now();

                for i in 0..reader.len() {
                    if stop_clone.load(Ordering::Acquire) {
                        return;
                    }

                    // サンプルを内包した native VideoFrameBuffer を送信して encode() をトリガーする。
                    // adapt_frame() は WebRTC のフレームアダプター (解像度/フレームレート調整) を通す。
                    // applied=false の場合はフレームドロッパーがスキップを指示している。
                    let timestamp_us = shiguredo_webrtc::time_millis() * 1000;
                    let AdaptFrameResult { applied, .. } =
                        source.adapt_frame(width, height, timestamp_us);
                    if applied {
                        let sample = reader.get_sample(i);
                        let frame_buffer = VideoFrameBuffer::new_with_handler(Box::new(sample));
                        let ts =
                            aligner.translate(timestamp_us, shiguredo_webrtc::time_millis() * 1000);
                        let video_frame = VideoFrame::builder(&frame_buffer)
                            .set_timestamp_us(ts)
                            .set_rtp_timestamp(0)
                            .build();
                        source.on_frame(&video_frame);
                    }

                    // 次のフレームの絶対送信時刻まで待機する。
                    // cumulative_duration_us(i+1) は「フレーム 0 から i までの合計再生時間」を返す。
                    // loop_start からのオフセットとして使うことで、累積ドリフトを防止する。
                    let next_frame_time_us = reader.cumulative_duration_us(i + 1);
                    let target = loop_start + std::time::Duration::from_micros(next_frame_time_us);
                    let now = std::time::Instant::now();
                    if target > now {
                        thread::sleep(target - now);
                    }
                }

                rtc_log_info!("MP4 reached end of file, looping back to the beginning");
            }
        });

        Ok(Self {
            video_source,
            stop,
            thread_handle: Some(thread_handle),
        })
    }

    pub fn video_source(&self) -> VideoTrackSource {
        self.video_source.clone()
    }
}

impl Drop for Mp4VideoCapturer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_capability_supports_only_selected_encoder_codec() {
        let capability = Mp4PassthroughVideoCodecCapability::new(VideoCodecType::H264);
        assert_eq!(capability.get_implementation().name(), "mp4-passthrough");
        assert!(capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264));
        assert!(!capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp9));
        assert!(!capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264));

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
                .create_video_encoder(
                    shiguredo_webrtc::Environment::new().as_ref(),
                    SdpVideoFormat::new("VP9").as_ref(),
                )
                .is_none()
        );
        assert!(
            capability
                .create_video_decoder(
                    shiguredo_webrtc::Environment::new().as_ref(),
                    SdpVideoFormat::new("H264").as_ref(),
                )
                .is_none()
        );

        let resolved = capability.resolve_sdp_format(
            CodecDirection::Encoder,
            SdpVideoFormat::new("H264").as_ref(),
        );
        assert!(resolved.is_some());

        let unresolved = capability
            .resolve_sdp_format(CodecDirection::Encoder, SdpVideoFormat::new("VP8").as_ref());
        assert!(unresolved.is_none());
    }

    #[test]
    fn annex_b_conversion_converts_multiple_nalus() {
        let input = [
            0x00, 0x00, 0x00, 0x02, 0x11, 0x22, 0x00, 0x00, 0x00, 0x03, 0x33, 0x44, 0x55,
        ];
        let output = length_prefixed_nalu_to_annex_b(&input);
        assert_eq!(
            output,
            vec![
                0x00, 0x00, 0x00, 0x01, 0x11, 0x22, 0x00, 0x00, 0x00, 0x01, 0x33, 0x44, 0x55,
            ]
        );
    }

    #[test]
    fn annex_b_conversion_ignores_truncated_nalu() {
        let input = [0x00, 0x00, 0x00, 0x05, 0x11, 0x22, 0x33];
        let output = length_prefixed_nalu_to_annex_b(&input);
        assert!(output.is_empty());
    }

    #[test]
    fn sample_reader_reads_fixture_h264_mp4() {
        let fixture = include_bytes!("testdata/archive-red-320x320-h264.mp4");
        let tmp_name = format!(
            "sora-sdk-mp4-test-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after UNIX_EPOCH")
                .as_nanos()
        );
        let tmp_path = std::env::temp_dir().join(tmp_name);

        std::fs::write(&tmp_path, fixture).expect("failed to write temporary fixture");

        let reader = Mp4SampleReader::new(tmp_path.to_str().expect("path should be valid utf-8"));

        let _ = std::fs::remove_file(&tmp_path);

        let reader = reader.expect("failed to parse fixture MP4");
        assert_eq!(reader.codec_type(), VideoCodecType::H264);
        assert!(!reader.is_empty());
    }
}
