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
//! Mp4PassthroughVideoCodecCapability は [`Mp4SampleReader::passthrough_capability`]
//! で reader から直接生成する。
//!
//! データフロー:
//!   Mp4SampleReader --[Mp4EncodedSample を内包した VideoFrame]--> Mp4PassthroughEncoder --> WebRTC RTP
//!                                         (native VideoFrameBuffer)
//!
//! `Mp4SampleReader` は `clone()` で共有でき、demux とファイル I/O を reader 1 つに集約する。
//! ファイル読み出しは reader 内部の I/O スレッド 1 本に直列化され、
//! 複数の `Mp4VideoCapturer` が同一ファイルを同時に読める。
//!
//! 対応コーデック: H.264, H.265, VP8, VP9, AV1
use std::io::{self, BufReader, Read, Seek};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;

use shiguredo_webrtc::{
    AdaptFrameResult, AdaptedVideoTrackSource, CodecSpecificInfo, EncodedImage, EncodedImageBuffer,
    H264PacketizationMode, I420Buffer, SdpVideoFormat, SdpVideoFormatRef, TimestampAligner,
    VideoCodecRef, VideoCodecStatus, VideoCodecType, VideoEncoder,
    VideoEncoderEncodedImageCallbackPtr, VideoEncoderEncodedImageCallbackRef,
    VideoEncoderEncodedImageCallbackResultError, VideoEncoderEncoderInfo, VideoEncoderHandler,
    VideoEncoderRateControlParametersRef, VideoEncoderSettingsRef, VideoFrame, VideoFrameBuffer,
    VideoFrameBufferHandler, VideoFrameRef, VideoFrameType, VideoFrameTypeVectorRef,
    VideoTrackSource, rtc_log_error, rtc_log_info, rtc_log_verbose, rtc_log_warning,
};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

/// MP4 ファイル処理中に発生するエラー。
#[derive(Debug)]
pub enum Mp4Error {
    /// I/O エラー。
    Io(io::Error),
    /// デマルチプレクスエラー。
    Demux(shiguredo_mp4::demux::DemuxError),
    /// 映像トラックが存在しない。
    NoVideoTrack,
    /// 映像サンプルが存在しない。
    NoVideoSamples,
    /// 未対応の映像コーデック。
    UnsupportedVideoCodec,
    /// NAL 長プレフィックスのバイト数が不正。
    /// ISO/IEC 14496-15 では nal_length_size は 1/2/4 のみ有効 (lengthSizeMinusOne 0/1/3)。
    InvalidNalLengthSize(u8),
    /// デマルチプレクサが要求する入力位置がファイルサイズ範囲外。
    InputPositionOutOfRange {
        /// デマルチプレクサが要求したファイル内位置。
        position: u64,
        /// 実際のファイルサイズ (バイト単位)。
        file_size: u64,
    },
    /// サンプルテーブル (stsz / stco / co64) に不整合があり、
    /// サンプルのオフセットがファイル範囲外になっている。
    InconsistentSampleTable {
        /// 問題のサンプルインデックス。
        index: usize,
        /// サンプルのファイル内オフセット。
        offset: u64,
        /// サンプルのデータサイズ。
        size: usize,
        /// 実際のファイルサイズ (バイト単位)。
        file_size: u64,
    },
    /// 非ゼロの composition time offset を含むビデオサンプルが存在する。
    ///
    /// B フレーム等のデコード順と表示順が異なる映像をデコード順のまま送信すると、
    /// 受信側の表示順序が壊れるため、受理しない。
    UnsupportedCompositionTimeOffset {
        /// 問題のサンプルインデックス (0 始まり、ビデオサンプル内の連番)。
        index: usize,
        /// ビデオコーデック種別。
        codec_type: VideoCodecType,
    },
    /// 2 個目以降のサンプルエントリーが最初の設定と一致しない。
    ///
    /// コーデックや解像度などが途中で変わるサンプルエントリーは受理しない。
    InconsistentSampleDescription {
        /// 相違が検出されたビデオサンプルの 0 始まりインデックス。
        index: usize,
        /// 相違したフィールド名。
        fields: Vec<&'static str>,
    },
}

impl std::fmt::Display for Mp4Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "読み込みに失敗しました: {err}"),
            Self::Demux(err) => write!(f, "デマルチプレクスに失敗しました: {err}"),
            Self::NoVideoTrack => f.write_str("映像トラックがありません"),
            Self::NoVideoSamples => f.write_str("映像サンプルがありません"),
            Self::UnsupportedVideoCodec => {
                f.write_str("映像コーデックが未対応です (H.264, H.265, VP8, VP9, AV1 のみ対応)")
            }
            Self::InvalidNalLengthSize(size) => {
                write!(
                    f,
                    "NAL 長プレフィックスのバイト数が不正です: {size} (1, 2, 4 のみ有効)"
                )
            }
            Self::InputPositionOutOfRange {
                position,
                file_size,
            } => {
                write!(
                    f,
                    "入力位置がファイルサイズ範囲外です: position={position}, file_size={file_size}"
                )
            }
            Self::InconsistentSampleTable {
                index,
                offset,
                size,
                file_size,
            } => {
                write!(
                    f,
                    "サンプルテーブルに不整合があります: sample={index} offset={offset} size={size} file_size={file_size}"
                )
            }
            Self::UnsupportedCompositionTimeOffset { index, codec_type } => {
                write!(
                    f,
                    "サンプルの composition time offset が非ゼロです: sample={index} codec={codec_type:?} (B フレームには未対応)"
                )
            }
            Self::InconsistentSampleDescription { index, fields } => {
                write!(
                    f,
                    "サンプルエントリーが最初の設定と一致しません: sample={index} fields={fields:?}"
                )
            }
        }
    }
}

impl std::error::Error for Mp4Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Demux(err) => Some(err),
            Self::NoVideoTrack
            | Self::NoVideoSamples
            | Self::UnsupportedVideoCodec
            | Self::InvalidNalLengthSize(_)
            | Self::InputPositionOutOfRange { .. }
            | Self::InconsistentSampleTable { .. }
            | Self::UnsupportedCompositionTimeOffset { .. }
            | Self::InconsistentSampleDescription { .. } => None,
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
pub(crate) struct Mp4EncodedSample {
    /// エンコード済みフレームデータ。
    /// H.264/H.265 の場合は Annex B 形式に変換済み。
    /// VP8/VP9/AV1 の場合は MP4 から抽出したそのまま。
    pub data: Vec<u8>,
    /// キーフレームかどうか。
    pub is_keyframe: bool,
    /// 映像の幅。
    pub width: u32,
    /// 映像の高さ。
    pub height: u32,
    /// ビデオコーデック種別。
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
#[derive(Clone)]
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
    /// NAL 長プレフィックスのバイト数 (length_size_minus_one + 1)。
    /// H.264/H.265 では 1/2/4 のいずれか。VP8/VP9/AV1 ではデフォルト値 4。
    nal_length_size: u8,
}

/// MP4 のビデオサンプルのメタデータ。
struct Mp4SampleMeta {
    /// サンプルデータのファイル内オフセット。
    data_offset: u64,
    /// サンプルデータのサイズ。
    data_size: usize,
    /// キーフレームかどうか。
    is_keyframe: bool,
    /// サンプルの長さ (MP4 のタイムスケール単位)。
    duration: u32,
}

/// MP4 のタイムスケールで表した再生時刻。
///
/// マイクロ秒へ事前変換せず、タイムスケール単位 (tick) のまま保持する。
/// 必要な時点で [`Mp4Timestamp::to_duration`] により [`std::time::Duration`] へ変換する。
struct Mp4Timestamp {
    /// タイムスケール単位の再生時刻。
    ticks: u64,
    /// 1 秒あたりのタイムスタンプ単位数。
    timescale: u32,
}

impl Mp4Timestamp {
    /// [`std::time::Duration`] に変換する。
    ///
    /// 商と剰余に分けて変換するため overflow しない:
    /// - 秒: `ticks / timescale` は `u64` に収まる
    /// - ナノ秒: `(ticks % timescale) * 1_000_000_000 / timescale` は
    ///   分子が `timescale` 未満 * 1_000_000_000 のため 1_000_000_000 未満になる
    fn to_duration(&self) -> std::time::Duration {
        let secs = self.ticks / self.timescale as u64;
        let nanos = (self.ticks % self.timescale as u64) * 1_000_000_000 / self.timescale as u64;
        std::time::Duration::new(secs, nanos as u32)
    }
}

/// MP4 ファイルからビデオサンプルを読み出すリーダー。
///
/// コンストラクタでファイルを開き、全サンプルのメタデータを事前解析する。
/// `clone()` で安価に共有でき、複数の [`Mp4VideoCapturer`] から同時にサンプルを読み出せる。
/// demux とファイル I/O は reader 1 つにつき 1 回・1 スレッドに集約される。
/// 再生位置や再生時計はクローン側に持たないため、各 [`Mp4VideoCapturer`] が独立して再生できる。
///
/// ファイルデータはメモリに保持せず、必要に応じて内部の I/O スレッドがファイルから読み込む。
/// 最後のクローンがドロップされると I/O スレッドを停止する。
///
/// [`Mp4VideoCapturer::video_source`] を複数の encoder に渡せない制約と回避策は、
/// `docs/INPUT_MP4.md` の「SDK での利用」を参照すること。
#[derive(Clone)]
pub struct Mp4SampleReader {
    /// 全クローンで共有する内部状態。
    inner: Arc<Mp4SampleReaderInner>,
}

/// `Mp4SampleReader` のクローン間で共有される内部状態。
struct Mp4SampleReaderInner {
    track_info: Mp4VideoTrackInfo,
    /// サンプル数。サンプルメタデータ自体は I/O スレッド側が保持する
    /// (二重保持を避けるため、`len()` / `is_empty()` に必要な数だけを残す)。
    sample_count: usize,
    /// 各フレームの累積再生時刻。
    /// cumulative[0] = 0, cumulative[i] = フレーム 0..i の合計再生時間。
    /// 長さは samples.len() + 1 で、末尾が動画全体の長さ。
    /// フレームペーシングで絶対時刻ベースの待機に使用する。
    cumulative: Vec<Mp4Timestamp>,
    /// ファイル I/O を担う内部スレッドの制御ハンドル。
    io: Mp4SampleReaderIo,
}

/// ファイル I/O スレッドへのサンプル読み出し依頼。
struct Mp4SampleIoRequest {
    /// 読み出すサンプルのインデックス。
    index: usize,
    /// 読み出し結果を受け取るチャネルの送信側。
    response: Sender<Result<Mp4EncodedSample>>,
}

/// ファイル I/O スレッドの制御ハンドル。
///
/// 終了メカニズム: 最後の `Mp4SampleReader` クローンがドロップされると
/// `Mp4SampleReaderInner` が drop され、この型の `Drop` が実行される。
/// `Drop` は sender を `take()` してチャネルを閉じ、I/O スレッドの `recv()` を
/// `Err` で終了させてから、thread を `take()` して join する。
/// フィールドが `Option` なのは、`&mut self` から `take()` により
/// 所有権ごと取り出すためである。
struct Mp4SampleReaderIo {
    /// 依頼を送るチャネルの送信側。`Drop` で先に閉じて I/O スレッドの `recv()` を終了させる。
    sender: Option<Sender<Mp4SampleIoRequest>>,
    /// I/O スレッドの join ハンドル。
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Mp4SampleReaderIo {
    fn drop(&mut self) {
        // チャネルを閉じてから join する (仕組みは構造体の doc コメント参照)。
        self.sender.take();
        if let Some(handle) = self.thread.take() {
            // チャネルが閉じられると I/O スレッドは進行中の読み出し (最大 1 回) の
            // 完了後に終了するため、join はすぐ完了する。
            if let Err(payload) = handle.join() {
                // I/O スレッドが panic した場合 (実装バグ) は、ここでもログに残す。
                rtc_log_error!(
                    "MP4: I/O thread panicked: {:?}",
                    payload.downcast_ref::<&str>()
                );
            }
        }
    }
}

impl Mp4SampleReader {
    /// MP4 ファイルを読み込み、ビデオトラックの全サンプルを事前解析する。
    pub fn new<P: AsRef<Path>>(path: P) -> crate::error::Result<Self> {
        Self::new_inner(path.as_ref()).map_err(crate::error::Error::from)
    }

    fn new_inner(path: &Path) -> Result<Self> {
        use shiguredo_mp4::demux::{Input, Mp4FileDemuxer};

        let mut file = BufReader::new(std::fs::File::open(path)?);
        let file_size = file.get_ref().metadata()?.len();

        let mut demuxer = Mp4FileDemuxer::new();

        // shiguredo_mp4 のデマルチプレクサにファイルデータを供給する。
        // required_input() が要求する範囲のデータを順次渡すことで、
        // ボックス構造の解析が進む。
        while let Some(required) = demuxer.required_input() {
            if required.position > file_size {
                return Err(Mp4Error::InputPositionOutOfRange {
                    position: required.position,
                    file_size,
                });
            }
            // size が None ならファイル末尾までの範囲を要求する。
            let remaining = file_size - required.position;
            let size = usize::try_from(
                required
                    .size
                    .map_or(remaining, |size| (size as u64).min(remaining)),
            )
            .map_err(|_| io::Error::other("required input size exceeds usize"))?;
            let data = read_bytes_at(&mut file, required.position, size)?;
            demuxer.handle_input(Input {
                position: required.position,
                data: &data,
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
        // 最初のサンプルの sample_entry からコーデック情報 (解像度、parameter sets 等) を取得し、
        // 以後の Some(sample_entry) も extract_track_info に通して、
        // codec_type / width / height / nal_length_size / parameter_sets の
        // 値等値を検証する。サンプルエントリーが途中で切り替わる MP4 を
        // 気付かれないまま最初の設定のまま送出しないためのゲート。
        let mut track_info: Option<Mp4VideoTrackInfo> = None;
        let mut samples = Vec::new();

        while let Some(sample) = demuxer.next_sample()? {
            // 音声など他トラックのサンプルはスキップする。
            if sample.track.track_id != video_track_id {
                continue;
            }

            // sample_entry が付与されたサンプルではコーデック情報を確定または一貫性検証する。
            //
            // shiguredo_mp4 が前のサンプルと構造的に等値なサンプルエントリーを `None` に
            // 正規化するため、後発の `Some(sample_entry)` は必ず何らかの相違を持つ。
            // 本 SDK が抽出する 5 フィールドの一致で `mismatched` が空になる場合、その相違は
            // 本 SDK の抽出範囲外（`avcC` header や補助 box）にある。codec 固有フィールドの
            // 検証は、将来 `Mp4VideoTrackInfo` を拡張する形で加える。
            if let Some(entry) = sample.sample_entry {
                let info = Self::extract_track_info(entry, timescale)?;
                if let Some(ref first) = track_info {
                    let mismatched = Self::collect_mismatched_track_info_fields(first, &info);
                    if !mismatched.is_empty() {
                        return Err(Mp4Error::InconsistentSampleDescription {
                            index: samples.len(),
                            fields: mismatched,
                        });
                    }
                } else {
                    track_info = Some(info);
                }
            }

            // 非ゼロの composition time offset はデコード順と表示時刻が一致しないため拒否する。
            if sample.composition_time_offset.unwrap_or(0) != 0 {
                return Err(Mp4Error::UnsupportedCompositionTimeOffset {
                    index: samples.len(),
                    // 通常はビデオトラックの最初のサンプルでサンプルエントリーから確定済みだが、
                    // サンプルエントリーが付与されない異常入力に備えて Generic を返す。
                    codec_type: track_info
                        .as_ref()
                        .map(|info| info.codec_type)
                        .unwrap_or(VideoCodecType::Generic),
                });
            }

            samples.push(Mp4SampleMeta {
                data_offset: sample.data_offset,
                data_size: sample.data_size,
                is_keyframe: sample.keyframe,
                duration: sample.duration,
            });
        }

        let track_info = track_info.ok_or(Mp4Error::NoVideoSamples)?;

        if samples.is_empty() {
            return Err(Mp4Error::NoVideoSamples);
        }

        for (index, sample) in samples.iter().enumerate() {
            let data_size_u64 = sample.data_size as u64;
            if sample
                .data_offset
                .checked_add(data_size_u64)
                .is_none_or(|end| end > file_size)
            {
                return Err(Mp4Error::InconsistentSampleTable {
                    index,
                    offset: sample.data_offset,
                    size: sample.data_size,
                    file_size,
                });
            }
        }

        // 累積再生時刻テーブルを事前計算する。
        // フレームペーシングで「次のフレームをいつ送るべきか」を O(1) で求めるため。
        // thread::sleep の相対待ちでは処理時間の累積ドリフトが発生するが、
        // このテーブルを使って Instant ベースの絶対時刻待ちを行うことで防止する。
        // 時刻はマイクロ秒へ事前変換せず、タイムスケール単位 (tick) のまま保持する。
        let timescale = track_info.timescale;
        let mut cumulative = Vec::new();
        let mut acc: u64 = 0;
        cumulative.push(Mp4Timestamp {
            ticks: 0,
            timescale,
        });
        for sample in &samples {
            // 加算は shiguredo_mp4 が検証する invariant
            // (Σ sample count <= u32::MAX なら総 duration < u64::MAX) により overflow しない。
            acc += sample.duration as u64;
            cumulative.push(Mp4Timestamp {
                ticks: acc,
                timescale,
            });
        }

        // ファイル I/O を内部スレッド 1 本に集約する。
        // 複数の Mp4VideoCapturer が同一 reader の clone から同時に get_sample を呼んでも、
        // スレッドが要求を直列化して処理するため BufReader + seek を奪い合わない。
        // サンプル数は Inner 側の len() / is_empty() でも使うため、move の前に控えておく。
        let sample_count = samples.len();
        let (io_sender, io_receiver) = mpsc::channel::<Mp4SampleIoRequest>();
        // I/O スレッドは Mp4SampleReaderInner を Arc で共有しない。
        // 共有すると最後のクローンが drop されても refcount が 0 にならず、
        // Mp4SampleReaderIo の Drop (構造体 doc 参照) による終了・ join が走らない。
        // samples は全体をスレッドへ move し、Inner にはサンプル数だけを残す。
        let io_track_info = track_info.clone();
        let io_thread = thread::spawn(move || {
            while let Ok(request) = io_receiver.recv() {
                let result = read_sample(&mut file, &io_track_info, &samples, request.index);
                let _ = request.response.send(result);
            }
        });

        Ok(Self {
            inner: Arc::new(Mp4SampleReaderInner {
                track_info,
                sample_count,
                cumulative,
                io: Mp4SampleReaderIo {
                    sender: Some(io_sender),
                    thread: Some(io_thread),
                },
            }),
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
                let nal_length_size =
                    Self::validated_nal_length_size(avc1.avcc_box.length_size_minus_one.get())?;
                Ok(Mp4VideoTrackInfo {
                    codec_type: VideoCodecType::H264,
                    width,
                    height,
                    timescale,
                    parameter_sets: Some(parameter_sets),
                    nal_length_size,
                })
            }
            // H.265 には hev1 と hvc1 の 2 種類の SampleEntry がある。
            // hev1: parameter sets がサンプルデータ内にも含まれる (帯域内シグナリング)。
            // hvc1: parameter sets は SampleEntry のみに含まれる (帯域外シグナリング)。
            // どちらの場合も HvccBox から parameter sets を抽出して使用する。
            SampleEntry::Hev1(hev1) => {
                let (width, height) = (hev1.visual.width, hev1.visual.height);
                let parameter_sets = Self::extract_hevc_parameter_sets(&hev1.hvcc_box);
                let nal_length_size =
                    Self::validated_nal_length_size(hev1.hvcc_box.length_size_minus_one.get())?;
                Ok(Mp4VideoTrackInfo {
                    codec_type: VideoCodecType::H265,
                    width,
                    height,
                    timescale,
                    parameter_sets: Some(parameter_sets),
                    nal_length_size,
                })
            }
            SampleEntry::Hvc1(hvc1) => {
                let (width, height) = (hvc1.visual.width, hvc1.visual.height);
                let parameter_sets = Self::extract_hevc_parameter_sets(&hvc1.hvcc_box);
                let nal_length_size =
                    Self::validated_nal_length_size(hvc1.hvcc_box.length_size_minus_one.get())?;
                Ok(Mp4VideoTrackInfo {
                    codec_type: VideoCodecType::H265,
                    width,
                    height,
                    timescale,
                    parameter_sets: Some(parameter_sets),
                    nal_length_size,
                })
            }
            SampleEntry::Vp08(vp08) => Ok(Mp4VideoTrackInfo {
                codec_type: VideoCodecType::Vp8,
                width: vp08.visual.width,
                height: vp08.visual.height,
                timescale,
                parameter_sets: None,
                nal_length_size: 4,
            }),
            SampleEntry::Vp09(vp09) => Ok(Mp4VideoTrackInfo {
                codec_type: VideoCodecType::Vp9,
                width: vp09.visual.width,
                height: vp09.visual.height,
                timescale,
                parameter_sets: None,
                nal_length_size: 4,
            }),
            SampleEntry::Av01(av01) => Ok(Mp4VideoTrackInfo {
                codec_type: VideoCodecType::Av1,
                width: av01.visual.width,
                height: av01.visual.height,
                timescale,
                parameter_sets: None,
                nal_length_size: 4,
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

    /// length_size_minus_one から NAL 長プレフィックスのバイト数を検証付きで取得する。
    ///
    /// ISO/IEC 14496-15 では lengthSizeMinusOne は 0/1/3 (nal_length_size 1/2/4) のみ有効。
    /// 値 2 は reserved であり、nal_length_size=3 となるため拒否する。
    fn validated_nal_length_size(length_size_minus_one: u8) -> Result<u8> {
        match length_size_minus_one {
            0 => Ok(1),
            1 => Ok(2),
            3 => Ok(4),
            _ => Err(Mp4Error::InvalidNalLengthSize(
                length_size_minus_one.saturating_add(1),
            )),
        }
    }

    /// 2 個の `Mp4VideoTrackInfo` をフィールド単位で比較し、相違するフィールド名を返す。
    ///
    /// 検証対象は `codec_type` / `width` / `height` / `nal_length_size` /
    /// `parameter_sets` の 5 フィールド。
    /// `timescale` は `mdhd` の track 単位属性で `SampleEntry` からは抽出されず、
    /// `extract_track_info` にはループ外の同一 scalar が毎回渡されるため、
    /// サンプルエントリー間で変わり得ない値として比較対象に含めない。
    ///
    /// codec 固有フィールド（H.264 の profile-level-id、AV1 の av1C / configOBUs など）
    /// の bit-identical 検証は、各 codec 固有の別対応で `Mp4VideoTrackInfo` を
    /// 拡張する形で加える。
    ///
    /// `Mp4VideoTrackInfo` に新しいフィールドを追加した際にヘルパー未更新を
    /// compile error として検出するため、両側を exhaustive に destructure して
    /// 明示的に列挙する。比較対象外のフィールドは `_` に束縛する。
    fn collect_mismatched_track_info_fields(
        first: &Mp4VideoTrackInfo,
        current: &Mp4VideoTrackInfo,
    ) -> Vec<&'static str> {
        let Mp4VideoTrackInfo {
            codec_type: first_codec_type,
            width: first_width,
            height: first_height,
            timescale: _,
            parameter_sets: first_parameter_sets,
            nal_length_size: first_nal_length_size,
        } = first;
        let Mp4VideoTrackInfo {
            codec_type: current_codec_type,
            width: current_width,
            height: current_height,
            timescale: _,
            parameter_sets: current_parameter_sets,
            nal_length_size: current_nal_length_size,
        } = current;

        let mut mismatched = Vec::new();
        if first_codec_type != current_codec_type {
            mismatched.push("codec_type");
        }
        if first_width != current_width {
            mismatched.push("width");
        }
        if first_height != current_height {
            mismatched.push("height");
        }
        if first_nal_length_size != current_nal_length_size {
            mismatched.push("nal_length_size");
        }
        if first_parameter_sets != current_parameter_sets {
            mismatched.push("parameter_sets");
        }
        mismatched
    }

    /// サンプル数を返す。
    pub fn len(&self) -> usize {
        self.inner.sample_count
    }

    /// サンプルが 0 件かどうかを返す。
    pub fn is_empty(&self) -> bool {
        self.inner.sample_count == 0
    }

    /// この MP4 のビデオコーデック種別を返す。
    pub fn codec_type(&self) -> VideoCodecType {
        self.inner.track_info.codec_type
    }

    /// この reader から MP4 パススルー用の [`Mp4PassthroughVideoCodecCapability`] を生成する。
    ///
    /// ファイル I/O は行わず、reader 構築時に確定した値の cheap clone だけを渡す。
    /// 共有されたどのクローンからでも同じ capability を生成できる。
    /// この経路が capability の唯一の構築ルート。
    pub fn passthrough_capability(&self) -> Mp4PassthroughVideoCodecCapability {
        Mp4PassthroughVideoCodecCapability {
            codec_type: self.inner.track_info.codec_type,
            required_format: self.required_sdp_format(),
        }
    }

    /// この MP4 のビットストリーム実態から、パススルー送信に必要な [`SdpVideoFormat`] を返す。
    ///
    /// reader 構築時に確定するため、呼び出しごとに同じ内容を返す。
    /// 現在は各 codec について以下を返す:
    /// - H.264: `H264`, `packetization-mode=1`
    /// - H.265: `H265`
    /// - VP8: `VP8`
    /// - VP9: `VP9`
    /// - AV1: `AV1`
    ///
    /// コーデック固有のパラメータ（H.264 の profile-level-id、AV1 の
    /// profile / level / tier など）は、各コーデックの別対応で拡張する。
    fn required_sdp_format(&self) -> SdpVideoFormat {
        match self.inner.track_info.codec_type {
            VideoCodecType::H264 => {
                let mut format = SdpVideoFormat::new("H264");
                format.parameters_mut().set("packetization-mode", "1");
                format
            }
            VideoCodecType::H265 => SdpVideoFormat::new("H265"),
            VideoCodecType::Vp8 => SdpVideoFormat::new("VP8"),
            VideoCodecType::Vp9 => SdpVideoFormat::new("VP9"),
            VideoCodecType::Av1 => SdpVideoFormat::new("AV1"),
            // 未対応 codec は `new_inner` の `extract_track_info` が
            // `Mp4Error::UnsupportedVideoCodec` で拒否するためここには来ない。
            VideoCodecType::Generic | VideoCodecType::Unknown(_) => {
                unreachable!("unsupported codec is rejected in Mp4SampleReader::new_inner")
            }
        }
    }

    /// 指定インデックスのサンプルデータを取得する。
    ///
    /// サンプルデータは内部の I/O スレッドがファイルから読み込むため、I/O エラーを返し得る。
    /// 複数のクローンから同時に呼び出してよい (読み出しは I/O スレッドに直列化される)。
    ///
    /// 応答待ちの間も `stop` をポーリングし、設定されていたら `Ok(None)` を返す
    /// (共有 I/O スレッドが他 capturer の要求で塞がっていても、停止・ join を妨げない)。
    ///
    /// H.264/H.265 の場合:
    /// - MP4 内の AVCC/HVCC 形式 (4 バイト長さプレフィックス) を
    ///   Annex B 形式 (0x00000001 スタートコード) に変換する。
    /// - キーフレームの場合は先頭に parameter sets (SPS/PPS 等) を付与する。
    ///
    /// VP8/VP9/AV1 の場合:
    /// - MP4 から抽出したデータをそのまま使用する。
    fn get_sample(&self, index: usize, stop: &AtomicBool) -> Result<Option<Mp4EncodedSample>> {
        // 範囲外の index は内部の利用バグ。共有 I/O スレッド側の panic だと全 capturer の
        // 呼び出しが BUG panic 連鎖になるため、送信前に呼び出し側で明確に panic させる。
        assert!(
            index < self.inner.sample_count,
            "BUG: sample index {index} is out of range (sample_count={})",
            self.inner.sample_count
        );
        let (response_tx, response_rx) = mpsc::channel();
        let sender = self.inner.io.sender.as_ref().expect(
            "BUG: I/O thread sender must be available while an Mp4SampleReader clone is alive",
        );
        sender
            .send(Mp4SampleIoRequest {
                index,
                response: response_tx,
            })
            .expect("BUG: I/O thread receiver must not be closed while an Mp4SampleReader clone is alive");
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            match response_rx.recv_timeout(std::time::Duration::from_millis(1)) {
                Ok(result) => return result.map(Some),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                // 依頼を送信できているのに応答チャネルが閉じるのは、
                // I/O スレッドが応答前に panic した場合のみ (実装バグ)。
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("BUG: I/O thread must respond exactly once per request")
                }
            }
        }
    }

    /// 先頭からフレーム index までの累積再生時間を返す。
    /// index=0 なら 0、index=len() なら動画全体の長さ。
    fn cumulative_duration(&self, index: usize) -> std::time::Duration {
        self.inner.cumulative[index].to_duration()
    }
}

/// 指定インデックスのサンプルデータをファイルから読み出し、送信用に変換して返す。
///
/// I/O スレッド上で実行され、複数のキャプチャラーの要求を直列化する。
/// H.264/H.265 は AVCC/HVCC を Annex B に変換し、キーフレームには parameter sets を付与する。
/// VP8/VP9/AV1 は抽出したデータをそのまま返す。
fn read_sample(
    file: &mut BufReader<std::fs::File>,
    track_info: &Mp4VideoTrackInfo,
    samples: &[Mp4SampleMeta],
    index: usize,
) -> Result<Mp4EncodedSample> {
    let sample = &samples[index];
    let raw_data = read_bytes_at(file, sample.data_offset, sample.data_size)?;

    let data = match track_info.codec_type {
        VideoCodecType::H264 | VideoCodecType::H265 => {
            let mut annex_b = Vec::new();
            if sample.is_keyframe
                && let Some(ref ps) = track_info.parameter_sets
            {
                annex_b.extend_from_slice(ps);
            }
            annex_b.extend_from_slice(&length_prefixed_nalu_to_annex_b(
                &raw_data,
                track_info.nal_length_size,
            ));
            annex_b
        }
        _ => raw_data,
    };

    Ok(Mp4EncodedSample {
        data,
        is_keyframe: sample.is_keyframe,
        width: track_info.width as u32,
        height: track_info.height as u32,
        codec_type: track_info.codec_type,
    })
}

/// ファイルの指定位置から指定サイズのデータを読み込む。
///
/// ファイルベースの読み込みで seek + read_exact の組み合わせが必要な箇所は
/// すべてこの関数に集約する。read_exact は要求サイズの読み込みを保証するため、
/// ファイルが途中で縮小されている場合は I/O エラーを返す。
fn read_bytes_at(
    file: &mut BufReader<std::fs::File>,
    position: u64,
    size: usize,
) -> Result<Vec<u8>> {
    let mut data = vec![0; size];
    file.seek(std::io::SeekFrom::Start(position))?;
    file.read_exact(&mut data)?;
    Ok(data)
}

/// 長さプレフィックス付き NAL ユニットを Annex B 形式に変換する。
///
/// MP4 内の H.264/H.265 フレームデータは AVCC/HVCC 形式で格納されている:
///   [nal_length_size バイトの NAL 長][NAL データ][...]
///
/// WebRTC (RTP) では Annex B 形式が期待される:
///   [0x00 0x00 0x00 0x01][NAL データ][0x00 0x00 0x00 0x01][NAL データ]...
///
/// `nal_length_size` は NAL 長プレフィックスのバイト数 (1/2/4)。
/// 呼び出し元 (`extract_track_info`) で検証済みのため、
/// 1/2/4 以外の値は debug ビルドでのみ panic する。
fn length_prefixed_nalu_to_annex_b(data: &[u8], nal_length_size: u8) -> Vec<u8> {
    debug_assert!(
        nal_length_size == 1 || nal_length_size == 2 || nal_length_size == 4,
        "nal_length_size must be 1, 2, or 4"
    );
    let nal_length_size = nal_length_size as usize;
    let mut result = Vec::new();
    let mut offset = 0;
    while offset + nal_length_size <= data.len() {
        let nal_size = match nal_length_size {
            1 => data[offset] as usize,
            2 => u16::from_be_bytes([data[offset], data[offset + 1]]) as usize,
            4 => u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize,
            _ => unreachable!(),
        };
        offset += nal_length_size;
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

        rtc_log_verbose!(
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
            rtc_log_warning!(
                "MP4Passthrough: on_encoded_image returned non-Ok status; continue encoding to avoid libwebrtc crash"
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
///
/// [`Mp4SampleReader::passthrough_capability`] から生成する。
/// フィールドは private で、`codec_type` と `required_format` の内部一貫性
/// （format 名が codec_type と一致する）を構造的に守る。
pub struct Mp4PassthroughVideoCodecCapability {
    codec_type: VideoCodecType,
    required_format: SdpVideoFormat,
}

impl VideoCodecCapability for Mp4PassthroughVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("mp4-passthrough", "MP4 Passthrough")
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        if direction != CodecDirection::Encoder {
            return Vec::new();
        }
        // reader 由来の required format だけを広告する。コーデック固有の
        // 必須パラメータ（例: H.264 の profile-level-id）を持つ format も
        // ここから返せる。
        vec![self.required_format.clone()]
    }

    // デフォルト実装は bare `SdpVideoFormat` を組み立てて `resolve_sdp_format`
    // に通す形だが、reader 由来 required format がコーデック固有のパラメータを
    // 要求する場合に bare 入力が拒否され、false になってしまう。それを避けるため
    // override し、Encoder かつ reader の codec type と一致する場合のみ true を返す。
    // preference の可否判定はこの結果を source of truth として扱われる。
    fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
        direction == CodecDirection::Encoder && codec_type == self.codec_type
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
    /// フィーダースレッドへの停止フラグ。
    stop: Arc<AtomicBool>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

/// 停止フラグの確認間隔の上限。
///
/// フレーム間隔の待機をこの時間ずつに分割して `thread::sleep` し、
/// 停止フラグの確認を挟むことで、停止までの最大遅延をこの値に制限する。
/// 値自体に特別な意味はなく、停止までの最大遅延が実用上十分に小さく
/// （100ms は体感できない程度）、かつ通常のフレーム間隔 (30 fps で約 33ms) の
/// 待機を分割しない程度の値として選んだ。
const MAX_SLEEP_DURATION: std::time::Duration = std::time::Duration::from_millis(100);

/// 停止フラグを確認しながら deadline まで待機する。
///
/// 停止フラグが設定されたら `true`、deadline に到達したら `false` を返す。
fn wait_until_or_stop(stop: &AtomicBool, deadline: std::time::Instant) -> bool {
    loop {
        if stop.load(Ordering::Acquire) {
            return true;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        thread::sleep(remaining.min(MAX_SLEEP_DURATION));
    }
}

impl Mp4VideoCapturer {
    /// [Mp4SampleReader] から動画データを読み込み、新しい `Mp4VideoCapturer` を生成する。
    ///
    /// 生成と同時に専用スレッドを起動し、MP4 のフレームタイミングに従って
    /// 映像フレームを WebRTC に供給する。動画末尾に達すると先頭に戻りループ再生する。
    ///
    /// 1 つの reader を複数の capturer で共有するには `reader.clone()` を渡す。
    /// 各 capturer は独立した feeder スレッド・再生時計を持つ。
    pub fn new(reader: Mp4SampleReader) -> crate::error::Result<Self> {
        let width = reader.inner.track_info.width as i32;
        let height = reader.inner.track_info.height as i32;

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
                        // サンプルデータの読み込みに失敗したらフィーダースレッドを終了する。
                        // 停止フラグが設定されて中断された場合 (Ok(None)) も終了する。
                        let sample = match reader.get_sample(i, &stop_clone) {
                            Ok(Some(sample)) => sample,
                            Ok(None) => return,
                            Err(err) => {
                                rtc_log_error!("MP4: failed to read sample: {err:?}");
                                return;
                            }
                        };
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
                    // cumulative_duration(i+1) は「フレーム 0 から i までの合計再生時間」を返す。
                    // loop_start からのオフセットとして使うことで、累積ドリフトを防止する。
                    let next_frame_time = reader.cumulative_duration(i + 1);
                    let Some(target) = loop_start.checked_add(next_frame_time) else {
                        // 累積再生時間が Instant の表現範囲を超えるのは、再生時間が極めて長い破損入力に限られる。
                        // 実用上は発生しないが、発生した場合はログを残してフィーダースレッドを終了する。
                        rtc_log_warning!("MP4: loop deadline overflow, stopping feeder thread");
                        return;
                    };
                    // 停止フラグが設定されたらフィーダースレッドを終了する。
                    if wait_until_or_stop(&stop_clone, target) {
                        return;
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

    /// WebRTC の [VideoTrackSource] を返す。
    ///
    /// 返された `VideoTrackSource` を複数の PeerConnection の映像 encoder に渡してはならない。
    /// 1 つの `VideoTrackSource` が運ぶ native `VideoFrameBuffer` を複数の encoder スレッドが
    /// 処理することになり、debug ビルドでは `video_frame_buffer callback called from
    /// multiple threads` で abort する。release ビルドでも同じ使い方は非対応である。
    ///
    /// 複数の PeerConnection に送る場合は、PeerConnection ごとに [`Mp4VideoCapturer`] を作り、
    /// 各 capturer の `video_source()` をその接続の encoder に渡すこと。
    /// 1 つの [`Mp4SampleReader`] は `clone()` して複数の [`Mp4VideoCapturer`] で共有できる。
    ///
    /// 詳細は `docs/INPUT_MP4.md` の「SDK での利用」を参照すること。
    pub fn video_source(&self) -> VideoTrackSource {
        self.video_source.clone()
    }
}

impl Drop for Mp4VideoCapturer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread_handle.take() {
            // フィーダースレッドは待機中 (最大 MAX_SLEEP_DURATION ごと) と
            // I/O 応答待ち中 (最大 1ms ごと) のどちらも停止フラグを確認するため、
            // join は停止フラグ設定後すぐに完了する。
            if let Err(payload) = handle.join() {
                // フィーダースレッドが panic した場合 (実装バグ) は、ここでもログに残す。
                rtc_log_error!(
                    "MP4: feeder thread panicked: {:?}",
                    payload.downcast_ref::<&str>()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use shiguredo_webrtc::{VideoFrameRef, VideoSink, VideoSinkHandler, VideoSinkWants};

    use super::*;
    use crate::video_codec_preference::VideoCodecPreference;

    /// テスト実行中だけ存在する一時 fixture ファイル。Drop で自動的に削除する。
    ///
    /// 各テストが個別に tmp ファイルを作ることで、テスト実行順序に依存しない。
    struct FixtureFile {
        path: PathBuf,
    }

    impl Drop for FixtureFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// H.264 の fixture MP4 を一時ファイルに書き出して `Mp4SampleReader` を生成する。
    /// 返される `FixtureFile` が drop されるまでファイルは残る。
    fn h264_reader_from_fixture(tag: &str) -> (Mp4SampleReader, FixtureFile) {
        let fixture = include_bytes!("../../testdata/red-320x320-h264.mp4");
        let tmp_name = format!(
            "sora-sdk-mp4-passthrough-{}-{}-{}.mp4",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("システム時刻は UNIX_EPOCH より後である必要があります")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(tmp_name);
        std::fs::write(&path, fixture).expect("一時 fixture の書き込みに失敗しました");
        let reader = Mp4SampleReader::new(&path).expect("fixture MP4 のパースに失敗しました");
        (reader, FixtureFile { path })
    }

    #[test]
    fn passthrough_capability_advertises_only_reader_required_format() {
        let (reader, _fixture) = h264_reader_from_fixture("required-format");
        let capability = reader.passthrough_capability();

        assert_eq!(capability.get_implementation().name(), "mp4-passthrough");

        // Encoder 側の広告フォーマットは reader の required_sdp_format() と同じ 1 件だけ。
        let encoder_formats = capability.get_supported_formats(CodecDirection::Encoder);
        assert_eq!(
            encoder_formats.len(),
            1,
            "Encoder 側は required_sdp_format() の 1 件だけを広告するはずです"
        );
        let required = reader.required_sdp_format();
        assert_eq!(
            encoder_formats[0]
                .name()
                .expect("format 名を取得できるはず"),
            required.name().expect("required の name を取得できるはず")
        );
        let mut owned = encoder_formats[0].clone();
        let params: std::collections::HashMap<String, String> =
            owned.parameters_mut().iter().collect();
        assert_eq!(
            params.get("packetization-mode").map(String::as_str),
            Some("1"),
            "H.264 required format は packetization-mode=1 を保持するはずです"
        );

        // Decoder 側はパススルー送信のみなので空。
        assert!(
            capability
                .get_supported_formats(CodecDirection::Decoder)
                .is_empty(),
            "Decoder 方向は空を返すはずです"
        );
    }

    #[test]
    fn passthrough_capability_is_supported_only_for_encoder_and_reader_codec_type() {
        let (reader, _fixture) = h264_reader_from_fixture("is-supported");
        let capability = reader.passthrough_capability();

        // Encoder かつ reader の codec type (H.264) と一致する場合だけ true。
        assert!(
            capability.is_supported(CodecDirection::Encoder, VideoCodecType::H264),
            "Encoder かつ H.264 は true を返すはずです"
        );
        assert!(
            !capability.is_supported(CodecDirection::Encoder, VideoCodecType::Vp9),
            "Encoder でも別 codec type は false を返すはずです"
        );
        assert!(
            !capability.is_supported(CodecDirection::Decoder, VideoCodecType::H264),
            "Decoder 方向は同じ codec type でも false を返すはずです"
        );
    }

    #[test]
    fn passthrough_capability_creates_encoder_only_for_reader_codec_type() {
        let (reader, _fixture) = h264_reader_from_fixture("create-encoder");
        let capability = reader.passthrough_capability();
        let env = shiguredo_webrtc::Environment::new();

        // reader の codec type と一致する H.264 では encoder が生成される。
        assert!(
            capability
                .create_video_encoder(env.as_ref(), SdpVideoFormat::new("H264").as_ref())
                .is_some(),
            "reader の codec type と一致する H.264 は encoder を生成できるはずです"
        );
        // 別の codec type ではエンコーダーは生成されない。
        assert!(
            capability
                .create_video_encoder(env.as_ref(), SdpVideoFormat::new("VP9").as_ref())
                .is_none(),
            "別の codec type の format は encoder を生成しないはずです"
        );
        // Decoder は常に生成しない (send only)。
        assert!(
            capability
                .create_video_decoder(env.as_ref(), SdpVideoFormat::new("H264").as_ref())
                .is_none(),
            "Decoder は生成しないはずです (send only)"
        );
    }

    #[test]
    fn passthrough_capability_preference_registers_encoder_entry() {
        let (reader, _fixture) = h264_reader_from_fixture("preference");
        let capability = reader.passthrough_capability();

        // is_supported override の結果が VideoCodecPreference::new_from_capability に
        // そのまま反映されることを確認する。
        let preference = VideoCodecPreference::new_from_capability(&capability);
        let codecs = preference.codecs();

        // is_supported override が Encoder + reader の codec_type にだけ true を返す
        // ことを保証する目的で、preference のエントリ数までを直接検証する。
        // 他 codec type や Decoder 方向で is_supported が誤って true を返すと、
        // ここで余計なエントリが載って failure になる。
        assert_eq!(
            codecs.len(),
            1,
            "preference は Encoder + H.264 のエントリを 1 件だけ持つはずです"
        );
        let entry = &codecs[0];
        assert_eq!(entry.direction(), CodecDirection::Encoder);
        assert_eq!(entry.codec_type(), VideoCodecType::H264);
        assert_eq!(
            entry.implementation(),
            &capability.get_implementation(),
            "エントリの implementation は passthrough capability のものと一致するはずです"
        );
    }

    /// テスト用の基準 `Mp4VideoTrackInfo` を返す。
    ///
    /// `parameter_sets` は非空の H.264 SPS 相当のバイト列にしてあり、
    /// `Some` → `None` / `Some(bytes)` → `Some(別 bytes)` の遷移を検証しやすくしている。
    fn base_track_info_for_consistency_test() -> Mp4VideoTrackInfo {
        Mp4VideoTrackInfo {
            codec_type: VideoCodecType::H264,
            width: 640,
            height: 360,
            timescale: 1000,
            parameter_sets: Some(vec![0x00, 0x00, 0x00, 0x01, 0x67]),
            nal_length_size: 4,
        }
    }

    #[test]
    fn sample_description_consistency_check_reports_field_mismatches() {
        // 実際に sample_entry の切り替わりが起きる合成 MP4 を用意するのが難しいため、
        // 内部ヘルパー collect_mismatched_track_info_fields を単独で検証する。
        let base = base_track_info_for_consistency_test();

        // 完全一致は相違なし。
        assert!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &base).is_empty(),
            "完全一致では相違が報告されないはずです"
        );

        // codec_type だけ変えると codec_type が相違として報告される。
        let mut modified = Mp4VideoTrackInfo {
            codec_type: VideoCodecType::H265,
            width: base.width,
            height: base.height,
            timescale: base.timescale,
            parameter_sets: base.parameter_sets.clone(),
            nal_length_size: base.nal_length_size,
        };
        assert_eq!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &modified),
            vec!["codec_type"],
            "codec_type だけの相違は codec_type のみを返すはずです"
        );

        // width だけ変えると width が相違として報告される。
        modified = Mp4VideoTrackInfo {
            codec_type: base.codec_type,
            width: 1280,
            height: base.height,
            timescale: base.timescale,
            parameter_sets: base.parameter_sets.clone(),
            nal_length_size: base.nal_length_size,
        };
        assert_eq!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &modified),
            vec!["width"],
            "width だけの相違は width のみを返すはずです"
        );

        // height だけ変えると height が相違として報告される。
        modified = Mp4VideoTrackInfo {
            codec_type: base.codec_type,
            width: base.width,
            height: 720,
            timescale: base.timescale,
            parameter_sets: base.parameter_sets.clone(),
            nal_length_size: base.nal_length_size,
        };
        assert_eq!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &modified),
            vec!["height"],
            "height だけの相違は height のみを返すはずです"
        );

        // nal_length_size だけ変えると nal_length_size が相違として報告される。
        modified = Mp4VideoTrackInfo {
            codec_type: base.codec_type,
            width: base.width,
            height: base.height,
            timescale: base.timescale,
            parameter_sets: base.parameter_sets.clone(),
            nal_length_size: 2,
        };
        assert_eq!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &modified),
            vec!["nal_length_size"],
            "nal_length_size だけの相違は nal_length_size のみを返すはずです"
        );

        // parameter_sets だけを変えると parameter_sets が相違として報告される。
        modified = Mp4VideoTrackInfo {
            codec_type: base.codec_type,
            width: base.width,
            height: base.height,
            timescale: base.timescale,
            parameter_sets: Some(vec![0xff]),
            nal_length_size: base.nal_length_size,
        };
        assert_eq!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &modified),
            vec!["parameter_sets"],
            "parameter_sets の byte 列の相違は parameter_sets のみを返すはずです"
        );

        // parameter_sets の Some → None 単独遷移も parameter_sets の相違として検出される。
        modified = Mp4VideoTrackInfo {
            codec_type: base.codec_type,
            width: base.width,
            height: base.height,
            timescale: base.timescale,
            parameter_sets: None,
            nal_length_size: base.nal_length_size,
        };
        assert_eq!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &modified),
            vec!["parameter_sets"],
            "parameter_sets の Some から None への遷移は parameter_sets のみを返すはずです"
        );

        // 逆向きの None → Some 単独遷移も同じく検出される。
        let base_without_params = Mp4VideoTrackInfo {
            codec_type: base.codec_type,
            width: base.width,
            height: base.height,
            timescale: base.timescale,
            parameter_sets: None,
            nal_length_size: base.nal_length_size,
        };
        let modified_with_params = Mp4VideoTrackInfo {
            codec_type: base.codec_type,
            width: base.width,
            height: base.height,
            timescale: base.timescale,
            parameter_sets: Some(vec![0x00, 0x00, 0x00, 0x01, 0x67]),
            nal_length_size: base.nal_length_size,
        };
        assert_eq!(
            Mp4SampleReader::collect_mismatched_track_info_fields(
                &base_without_params,
                &modified_with_params
            ),
            vec!["parameter_sets"],
            "parameter_sets の None から Some への遷移は parameter_sets のみを返すはずです"
        );

        // codec_type / height / nal_length_size / parameter_sets を同時に変えると
        // 相違リストに全フィールドが設計方針の記載順で並ぶ。
        modified = Mp4VideoTrackInfo {
            codec_type: VideoCodecType::H265,
            width: base.width,
            height: 720,
            timescale: base.timescale,
            parameter_sets: None,
            nal_length_size: 2,
        };
        assert_eq!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &modified),
            vec!["codec_type", "height", "nal_length_size", "parameter_sets"],
            "複数フィールドの相違は codec_type -> width -> height -> nal_length_size -> parameter_sets の順で並ぶはずです"
        );

        // timescale だけを変えても比較対象外なので相違なし。
        modified = Mp4VideoTrackInfo {
            codec_type: base.codec_type,
            width: base.width,
            height: base.height,
            timescale: 90_000,
            parameter_sets: base.parameter_sets.clone(),
            nal_length_size: base.nal_length_size,
        };
        assert!(
            Mp4SampleReader::collect_mismatched_track_info_fields(&base, &modified).is_empty(),
            "timescale は比較対象外なので相違として報告されないはずです"
        );
    }

    #[test]
    fn inconsistent_sample_description_display_and_source() {
        // Display 出力に sample index と全ての相違フィールド名が含まれることを確認する。
        // issue 側の完了条件で「Display 実装が sample index と相違フィールド名を含む」と
        // 明示されているため、helper unit test とは別に error variant 側を直接検証する。
        let err = Mp4Error::InconsistentSampleDescription {
            index: 3,
            fields: vec!["codec_type", "width", "parameter_sets"],
        };
        let message = format!("{err}");
        assert!(
            message.contains("sample=3"),
            "sample index が Display 出力に含まれるはずです: {message}"
        );
        for expected_field in ["codec_type", "width", "parameter_sets"] {
            assert!(
                message.contains(expected_field),
                "相違したフィールド名 {expected_field} が Display 出力に含まれるはずです: {message}"
            );
        }

        // 本 variant は wrapping 元のエラーを持たないため source() は None を返す。
        // 将来 refactor で誤って Some(...) を返す分岐に追加された場合の regression を捕捉する。
        use std::error::Error as _;
        assert!(
            err.source().is_none(),
            "InconsistentSampleDescription は source を持たないはずです"
        );
    }

    #[test]
    fn annex_b_conversion_converts_multiple_nalus() {
        let input = [
            0x00, 0x00, 0x00, 0x02, 0x11, 0x22, 0x00, 0x00, 0x00, 0x03, 0x33, 0x44, 0x55,
        ];
        let output = length_prefixed_nalu_to_annex_b(&input, 4);
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
        let output = length_prefixed_nalu_to_annex_b(&input, 4);
        assert!(output.is_empty());
    }

    #[test]
    fn annex_b_conversion_1byte_nal_length_single_nalu() {
        let input = [0x03, 0x11, 0x22, 0x33];
        let output = length_prefixed_nalu_to_annex_b(&input, 1);
        assert_eq!(output, vec![0x00, 0x00, 0x00, 0x01, 0x11, 0x22, 0x33]);
    }

    #[test]
    fn annex_b_conversion_1byte_nal_length_multiple_nalus() {
        let input = [0x02, 0xAA, 0xBB, 0x03, 0xCC, 0xDD, 0xEE];
        let output = length_prefixed_nalu_to_annex_b(&input, 1);
        assert_eq!(
            output,
            vec![
                0x00, 0x00, 0x00, 0x01, 0xAA, 0xBB, 0x00, 0x00, 0x00, 0x01, 0xCC, 0xDD, 0xEE
            ]
        );
    }

    #[test]
    fn annex_b_conversion_1byte_nal_length_truncated_nalu() {
        let input = [0x05, 0x11, 0x22];
        let output = length_prefixed_nalu_to_annex_b(&input, 1);
        assert!(output.is_empty());
    }

    #[test]
    fn annex_b_conversion_2byte_nal_length_single_nalu() {
        let input = [0x00, 0x03, 0x11, 0x22, 0x33];
        let output = length_prefixed_nalu_to_annex_b(&input, 2);
        assert_eq!(output, vec![0x00, 0x00, 0x00, 0x01, 0x11, 0x22, 0x33]);
    }

    #[test]
    fn annex_b_conversion_2byte_nal_length_multiple_nalus() {
        let input = [0x00, 0x02, 0xAA, 0xBB, 0x00, 0x03, 0xCC, 0xDD, 0xEE];
        let output = length_prefixed_nalu_to_annex_b(&input, 2);
        assert_eq!(
            output,
            vec![
                0x00, 0x00, 0x00, 0x01, 0xAA, 0xBB, 0x00, 0x00, 0x00, 0x01, 0xCC, 0xDD, 0xEE
            ]
        );
    }

    #[test]
    fn annex_b_conversion_2byte_nal_length_truncated_nalu() {
        let input = [0x00, 0x05, 0x11, 0x22];
        let output = length_prefixed_nalu_to_annex_b(&input, 2);
        assert!(output.is_empty());
    }

    // ファイルベースのサンプル読み込みが、フィクスチャに記録された
    // オフセット・サイズで正しいデータを読み出せることを確認する。
    //
    // サンプル 0 はキーフレーム (offset=48, size=702) で、変換後データは
    // parameter sets (SPS/PPS) とサンプル NAL データの連結になる。
    // 期待値はフィクスチャの stco / stsz / avcC ボックスから直接計算するため、
    // フィクスチャ差し替え時にも自動的に追従する。
    #[test]
    fn sample_reader_reads_fixture_h264_mp4() {
        let fixture = include_bytes!("../../testdata/red-320x320-h264.mp4");
        let tmp_name = format!(
            "sora-sdk-mp4-test-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("システム時刻は UNIX_EPOCH より後である必要があります")
                .as_nanos()
        );
        let tmp_path = std::env::temp_dir().join(tmp_name);

        std::fs::write(&tmp_path, fixture).expect("一時フィクスチャの書き込みに失敗しました");

        let reader = Mp4SampleReader::new(
            tmp_path
                .to_str()
                .expect("パスは有効な UTF-8 である必要があります"),
        )
        .expect("フィクスチャ MP4 のパースに失敗しました");
        assert_eq!(reader.codec_type(), VideoCodecType::H264);
        assert!(!reader.is_empty());
        // 中断テストではないため、停止フラグは常に false のまま渡す。
        let stop = AtomicBool::new(false);
        let sample = reader
            .get_sample(0, &stop)
            .expect("サンプルデータの読み込みに失敗しました")
            .expect("停止フラグは未設定のため中断されないはずです");

        // サンプル 0 のファイル内オフセットは stco ボックスの先頭エントリから求める。
        // ボックス構造: size(4) + type(4) + version/flags(4) + entry_count(4) + [offset(4) * entry_count]
        let stco_offset = fixture
            .windows(4)
            .position(|w| w == b"stco")
            .expect("フィクスチャに stco ボックスが必要です");
        let stco_entry_count = u32::from_be_bytes(
            fixture[stco_offset + 8..stco_offset + 12]
                .try_into()
                .expect("stco の entry_count は 4 バイトで読める必要があります"),
        );
        assert_eq!(
            stco_entry_count, 1,
            "フィクスチャの stco エントリ数が移動しています"
        );
        let sample_offset = u32::from_be_bytes(
            fixture[stco_offset + 12..stco_offset + 16]
                .try_into()
                .expect("stco の先頭エントリは 4 バイトで読める必要があります"),
        );
        assert_eq!(
            sample_offset, 48,
            "フィクスチャのサンプル 0 のオフセットが移動しています"
        );

        // サンプル 0 のデータサイズは stsz ボックスの先頭エントリから求める。
        // ボックス構造: size(4) + type(4) + version/flags(4) + sample_size(4)
        //               + sample_count(4) + [entry_size(4) * sample_count]
        let stsz_offset = fixture
            .windows(4)
            .position(|w| w == b"stsz")
            .expect("フィクスチャに stsz ボックスが必要です");
        let sample_size = u32::from_be_bytes(
            fixture[stsz_offset + 16..stsz_offset + 20]
                .try_into()
                .expect("stsz の先頭エントリは 4 バイトで読める必要があります"),
        );
        assert_eq!(
            sample_size, 702,
            "フィクスチャのサンプル 0 のサイズが移動しています"
        );

        // parameter sets (SPS/PPS) は avcC ボックスから抽出する。
        // ボックス構造: size(4) + type(4) + version(1) + profile(1) + compat(1) + level(1)
        //               + length_size_minus_one(1) + num_of_sps(1) + [sps_length(2) + sps]
        //               + num_of_pps(1) + [pps_length(2) + pps]
        // num_of_sps の上位 3 ビットは reserved (111) なのでマスクする。
        let avcc_offset = fixture
            .windows(4)
            .position(|w| w == b"avcC")
            .expect("フィクスチャに avcC ボックスが必要です");
        let num_of_sps = (fixture[avcc_offset + 9] & 0x1f) as usize;
        let sps_length = u16::from_be_bytes(
            fixture[avcc_offset + 10..avcc_offset + 12]
                .try_into()
                .expect("sps_length は 2 バイトで読める必要があります"),
        ) as usize;
        let sps = &fixture[avcc_offset + 12..avcc_offset + 12 + sps_length];
        let num_of_pps = fixture[avcc_offset + 12 + sps_length] as usize;
        let pps_length = u16::from_be_bytes(
            fixture[avcc_offset + 13 + sps_length..avcc_offset + 15 + sps_length]
                .try_into()
                .expect("pps_length は 2 バイトで読める必要があります"),
        ) as usize;
        let pps =
            &fixture[avcc_offset + 15 + sps_length..avcc_offset + 15 + sps_length + pps_length];
        assert_eq!(num_of_sps, 1, "フィクスチャの SPS 数が移動しています");
        assert_eq!(num_of_pps, 1, "フィクスチャの PPS 数が移動しています");
        assert_eq!(
            sps[0], 0x67,
            "フィクスチャの SPS の先頭バイトが移動しています"
        );
        assert_eq!(
            pps[0], 0x68,
            "フィクスチャの PPS の先頭バイトが移動しています"
        );

        // 変換後データは [start code(4) + SPS][start code(4) + PPS][サンプル NAL データ] の形式になる。
        // サンプル NAL データは、stco/stsz が示す範囲の AVCC 形式データを
        // 長さプレフィックス変換したものと一致する必要がある (変換は長さ 1:1)。
        let sample_start = sample_offset as usize;
        let sample_end = sample_start + sample_size as usize;
        let expected_annex_b =
            length_prefixed_nalu_to_annex_b(&fixture[sample_start..sample_end], 4);
        let expected_len = 4 + sps_length + 4 + pps_length + expected_annex_b.len();
        assert_eq!(
            sample.data.len(),
            expected_len,
            "サンプル 0 のデータ長が期待値と異なります"
        );
        assert_eq!(
            &sample.data[0..4],
            &[0x00, 0x00, 0x00, 0x01],
            "SPS のスタートコードがありません"
        );
        assert_eq!(
            &sample.data[4..4 + sps_length],
            sps,
            "SPS が変換後データの先頭に現れるべきです"
        );
        assert_eq!(
            &sample.data[4 + sps_length..8 + sps_length],
            &[0x00, 0x00, 0x00, 0x01],
            "PPS のスタートコードがありません"
        );
        assert_eq!(
            &sample.data[8 + sps_length..8 + sps_length + pps_length],
            pps,
            "PPS が SPS の後に現れるべきです"
        );
        assert_eq!(
            &sample.data[8 + sps_length + pps_length..],
            expected_annex_b,
            "サンプル NAL データがファイルから正しく読み込まれていません"
        );

        // cumulative_duration の全値が従来どおり (i * 40000 µs) であることを確認する。
        // フィクスチャは 25 サンプル、duration 512、timescale 12800 のため、
        // 1 サンプルあたり 512 * 1_000_000 / 12800 = 40000 マイクロ秒になる。
        for i in 0..=reader.len() {
            assert_eq!(
                reader.cumulative_duration(i),
                std::time::Duration::from_micros(i as u64 * 40000),
                "cumulative_duration[{i}] が期待値と異なります"
            );
        }

        let _ = std::fs::remove_file(&tmp_path);
    }

    // Mp4Timestamp::to_duration が overflow せず正しい Duration を返すことを確認する。
    #[test]
    fn mp4_timestamp_converts_to_duration() {
        // ticks=0 は 0 秒。
        assert_eq!(
            Mp4Timestamp {
                ticks: 0,
                timescale: 12800
            }
            .to_duration(),
            std::time::Duration::ZERO
        );
        // ticks == timescale はちょうど 1 秒。
        assert_eq!(
            Mp4Timestamp {
                ticks: 12800,
                timescale: 12800
            }
            .to_duration(),
            std::time::Duration::from_secs(1)
        );
        // 割り切れない剰余: 1 tick = 1/12800 秒 = 78125 ナノ秒。
        assert_eq!(
            Mp4Timestamp {
                ticks: 1,
                timescale: 12800
            }
            .to_duration(),
            std::time::Duration::from_nanos(78125)
        );
        // 巨大な ticks でも overflow しない (timescale=1 なら tick がそのまま秒になる)。
        assert_eq!(
            Mp4Timestamp {
                ticks: u64::MAX,
                timescale: 1
            }
            .to_duration(),
            std::time::Duration::new(u64::MAX, 0)
        );
        // ナノ秒の乗算が最大になる境界 (timescale=u32::MAX、剰余=timescale-1) でも overflow しない。
        let max_mul = (u32::MAX as u64 - 1) * 1_000_000_000 / u32::MAX as u64;
        assert_eq!(
            Mp4Timestamp {
                ticks: u32::MAX as u64 - 1,
                timescale: u32::MAX
            }
            .to_duration(),
            std::time::Duration::from_nanos(max_mul)
        );
        // timescale ちょうど (剰余 0) は 1 秒。
        assert_eq!(
            Mp4Timestamp {
                ticks: u32::MAX as u64,
                timescale: u32::MAX
            }
            .to_duration(),
            std::time::Duration::from_secs(1)
        );
    }

    // リーダー構築後にファイルを 0 バイトへ縮小すると、
    // get_sample のファイル読み込みが I/O エラーとして失敗することを確認する。
    //
    // read_exact は要求サイズの読み込みを保証するため、
    // ファイルが縮小されていると UnexpectedEof が発生する。
    // 縮小は別ハンドルから行う (std::fs::File::open は共有モードで開くため、
    // Unix/Windows ともリーダーの保持するハンドルには影響しない)。
    #[test]
    fn sample_reader_get_sample_returns_io_error_after_file_truncation() {
        let fixture = include_bytes!("../../testdata/red-320x320-h264.mp4");
        let tmp_name = format!(
            "sora-sdk-mp4-test-truncate-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("システム時刻は UNIX_EPOCH より後である必要があります")
                .as_nanos()
        );
        let tmp_path = std::env::temp_dir().join(tmp_name);

        std::fs::write(&tmp_path, fixture).expect("一時フィクスチャの書き込みに失敗しました");

        let reader = Mp4SampleReader::new(
            tmp_path
                .to_str()
                .expect("パスは有効な UTF-8 である必要があります"),
        )
        .expect("フィクスチャ MP4 のパースに失敗しました");

        let file = std::fs::File::options()
            .write(true)
            .open(&tmp_path)
            .expect("縮小用ハンドルのオープンに失敗しました");
        file.set_len(0).expect("ファイルの縮小に失敗しました");
        drop(file);

        // 中断テストではないため、停止フラグは常に false のまま渡す。
        let stop = AtomicBool::new(false);
        let result = reader.get_sample(0, &stop);
        assert!(
            matches!(result, Err(Mp4Error::Io(_))),
            "縮小されたファイルからの読み込みは Io エラーになるべきです"
        );

        let _ = std::fs::remove_file(&tmp_path);
    }

    // validated_nal_length_size が有効な length_size_minus_one (0/1/3) を
    // それぞれ nal_length_size 1/2/4 に変換することを確認する。
    #[test]
    fn validated_nal_length_size_accepts_valid_values() {
        assert_eq!(
            Mp4SampleReader::validated_nal_length_size(0)
                .expect("length_size_minus_one=0 は受け入れられる必要があります"),
            1
        );
        assert_eq!(
            Mp4SampleReader::validated_nal_length_size(1)
                .expect("length_size_minus_one=1 は受け入れられる必要があります"),
            2
        );
        assert_eq!(
            Mp4SampleReader::validated_nal_length_size(3)
                .expect("length_size_minus_one=3 は受け入れられる必要があります"),
            4
        );
    }

    // validated_nal_length_size が reserved 値 (length_size_minus_one=2) を
    // 拒否して InvalidNalLengthSize エラーを返すことを確認する。
    #[test]
    fn validated_nal_length_size_rejects_reserved_value() {
        let result = Mp4SampleReader::validated_nal_length_size(2);
        assert!(result.is_err());
        assert!(matches!(
            result.expect_err("reserved 値はエラーになる必要があります"),
            Mp4Error::InvalidNalLengthSize(3)
        ));
    }

    // 不正な length_size_minus_one (reserved 値 2) を持つ MP4 を入力した場合に
    // Mp4SampleReader::new が panic せず Err を返すことを確認する。
    // 既存の H.264 フィクスチャの avcC ボックス内 lengthSizeMinusOne バイトを
    // 0xFF (値 3) から 0xFE (値 2) に書き換えて不正な MP4 を作成する。
    #[test]
    fn sample_reader_rejects_invalid_length_size_minus_one() {
        let fixture = include_bytes!("../../testdata/red-320x320-h264.mp4");
        let mut patched = fixture.to_vec();

        // avcC ボックスの lengthSizeMinusOne バイト (オフセット 0x6ea) を
        // 0xFF (lengthSizeMinusOne=3, nal_length_size=4) から
        // 0xFE (lengthSizeMinusOne=2, nal_length_size=3 = reserved) に書き換える。
        // フィクスチャ差し替え時にオフセットがズレていないことを確認する。
        assert_eq!(
            patched[0x6ea], 0xFF,
            "フィクスチャの lengthSizeMinusOne バイトが移動しています"
        );
        patched[0x6ea] = 0xFE;

        let tmp_name = format!(
            "sora-sdk-mp4-test-invalid-nal-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("システム時刻は UNIX_EPOCH より後である必要があります")
                .as_nanos()
        );
        let tmp_path = std::env::temp_dir().join(tmp_name);

        std::fs::write(&tmp_path, &patched).expect("一時フィクスチャの書き込みに失敗しました");

        let result = Mp4SampleReader::new(
            tmp_path
                .to_str()
                .expect("パスは有効な UTF-8 である必要があります"),
        );

        let _ = std::fs::remove_file(&tmp_path);

        match result {
            Err(crate::error::Error::Mp4 { source }) => {
                assert!(
                    matches!(source, Mp4Error::InvalidNalLengthSize(_)),
                    "InvalidNalLengthSize エラーを期待しましたが、実際は: {source:?}"
                );
            }
            Err(e) => panic!("Mp4 エラーを期待しましたが、実際は: {e}"),
            Ok(_) => panic!("Err を期待しましたが、Ok でした"),
        }
    }

    // 切り詰められた MP4 ファイルを入力した場合に、
    // Mp4SampleReader::new が panic せず Err を返すことを確認する。
    #[test]
    fn sample_reader_rejects_truncated_mp4_with_oversized_input_position() {
        let fixture = include_bytes!("../../testdata/red-320x320-h264.mp4");
        let truncated = &fixture[..128];

        let tmp_name = format!(
            "sora-sdk-mp4-test-truncated-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("システム時刻は UNIX_EPOCH より後である必要があります")
                .as_nanos()
        );
        let tmp_path = std::env::temp_dir().join(tmp_name);

        std::fs::write(&tmp_path, truncated).expect("一時フィクスチャの書き込みに失敗しました");

        let result = Mp4SampleReader::new(
            tmp_path
                .to_str()
                .expect("パスは有効な UTF-8 である必要があります"),
        );

        let _ = std::fs::remove_file(&tmp_path);

        assert!(result.is_err(), "切り詰め MP4 は Err になるべきです");
    }

    #[test]
    fn sample_reader_rejects_inconsistent_sample_table_offset_exceeds_file_size() {
        let fixture = include_bytes!("../../testdata/red-320x320-h264.mp4");
        let mut patched = fixture.to_vec();
        let file_size = patched.len();

        let stco_offset = fixture
            .windows(4)
            .position(|w| w == b"stco")
            .expect("fixture に stco ボックスが必要です");

        let data_start = stco_offset + 8 + 4;
        let bad_offset = (file_size + 1) as u32;
        patched[data_start..data_start + 4].copy_from_slice(&bad_offset.to_be_bytes());

        let tmp_name = format!(
            "sora-sdk-mp4-test-stco-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("システム時刻は UNIX_EPOCH より後である必要があります")
                .as_nanos()
        );
        let tmp_path = std::env::temp_dir().join(tmp_name);

        std::fs::write(&tmp_path, &patched).expect("一時フィクスチャの書き込みに失敗しました");

        let result = Mp4SampleReader::new(
            tmp_path
                .to_str()
                .expect("パスは有効な UTF-8 である必要があります"),
        );

        let _ = std::fs::remove_file(&tmp_path);

        assert!(
            matches!(
                result,
                Err(crate::error::Error::Mp4 {
                    source: Mp4Error::InconsistentSampleTable { .. },
                })
            ),
            "不正な stco を持つ MP4 は InconsistentSampleTable エラーになるべきです"
        );
    }

    // 非ゼロの composition time offset (B フレーム) を含む MP4 を入力した場合に、
    // Mp4SampleReader::new が panic せず UnsupportedCompositionTimeOffset エラーを返すことを確認する。
    //
    // フィクスチャは次のコマンドで生成した (ffmpeg 7.1.1):
    //   ffmpeg -y -v error -f lavfi -i "color=red:size=320x320:rate=25:duration=2" \
    //     -c:v libx264 -preset medium -bf 2 -g 50 -b:v 50k -maxrate 50k -bufsize 100k \
    //     -pix_fmt yuv420p red-bframe-320x320-h264.mp4
    // H.264 High Profile Level 2.1、timescale=12800 で、B フレームの先頭 reorder により
    // 最初の sample (index 0) の composition time offset が 1024 になる。
    #[test]
    fn sample_reader_rejects_b_frame_fixture() {
        let fixture = include_bytes!("../../testdata/red-bframe-320x320-h264.mp4");

        let tmp_name = format!(
            "sora-sdk-mp4-test-bframe-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("システム時刻は UNIX_EPOCH より後である必要があります")
                .as_nanos()
        );
        let tmp_path = std::env::temp_dir().join(tmp_name);

        std::fs::write(&tmp_path, fixture).expect("一時フィクスチャの書き込みに失敗しました");

        let result = Mp4SampleReader::new(
            tmp_path
                .to_str()
                .expect("パスは有効な UTF-8 である必要があります"),
        );

        let _ = std::fs::remove_file(&tmp_path);

        match result {
            Err(crate::error::Error::Mp4 { source }) => {
                assert!(
                    matches!(
                        source,
                        Mp4Error::UnsupportedCompositionTimeOffset {
                            index: 0,
                            codec_type: VideoCodecType::H264,
                        }
                    ),
                    "UnsupportedCompositionTimeOffset エラーを期待しましたが、実際は: {source:?}"
                );
            }
            Err(e) => panic!("Mp4 エラーを期待しましたが、実際は: {e}"),
            Ok(_) => panic!("Err を期待しましたが、Ok でした"),
        }
    }

    // ctts ボックスが存在しても composition time offset が全て 0 の MP4 は受理されることを確認する。
    //
    // 判定は「offset の値」で行うため、ctts なし (None) と offset 0 (Some(0)) は同じように受理される。
    // フィクスチャ差し替えを検出するため、パッチ前に最初のエントリの offset が 1024 であることを確認する。
    #[test]
    fn sample_reader_accepts_zero_composition_time_offset_fixture() {
        let fixture = include_bytes!("../../testdata/red-bframe-320x320-h264.mp4");
        let mut patched = fixture.to_vec();

        // ctts ボックスの全エントリの sample_offset を 0 に書き換える。
        // ボックス構造: size(4) + type(4) + version/flags(4) + entry_count(4) + [sample_count(4) + sample_offset(4)] * entry_count
        let ctts_offset = patched
            .windows(4)
            .position(|w| w == b"ctts")
            .expect("fixture に ctts ボックスが必要です");
        // ctts の位置から type(4) と version/flags(4) を飛ばすと entry_count が読める。
        let entry_count_offset = ctts_offset + 8;
        let entry_count = u32::from_be_bytes(
            patched[entry_count_offset..entry_count_offset + 4]
                .try_into()
                .expect("entry_count は 4 バイトで読める必要があります"),
        );
        // 先頭エントリの sample_offset は entry_count(4) + sample_count(4) の後にある。
        assert_eq!(
            u32::from_be_bytes(
                patched[entry_count_offset + 8..entry_count_offset + 12]
                    .try_into()
                    .expect("先頭エントリの sample_offset は 4 バイトで読める必要があります")
            ),
            1024,
            "フィクスチャの先頭エントリの sample_offset が移動しています"
        );
        for i in 0..entry_count {
            let offset_pos = entry_count_offset + 4 + i as usize * 8 + 4;
            patched[offset_pos..offset_pos + 4].copy_from_slice(&0u32.to_be_bytes());
        }

        let tmp_name = format!(
            "sora-sdk-mp4-test-ctts-zero-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("システム時刻は UNIX_EPOCH より後である必要があります")
                .as_nanos()
        );
        let tmp_path = std::env::temp_dir().join(tmp_name);

        std::fs::write(&tmp_path, &patched).expect("一時フィクスチャの書き込みに失敗しました");

        let result = Mp4SampleReader::new(
            tmp_path
                .to_str()
                .expect("パスは有効な UTF-8 である必要があります"),
        );

        let _ = std::fs::remove_file(&tmp_path);

        match result {
            Ok(reader) => {
                assert_eq!(
                    reader.codec_type(),
                    VideoCodecType::H264,
                    "offset 0 の MP4 は H.264 reader として読み込めるべきです"
                );
            }
            Err(e) => panic!("offset 0 の MP4 は Ok を期待しましたが、実際は: {e}"),
        }
    }

    // deadline を 60 秒先に設定し、停止フラグ設定済みなら
    // sleep せずに即座に true を返すことを確認する。
    #[test]
    fn wait_until_or_stop_stops_immediately_when_stop_is_set() {
        let stop = AtomicBool::new(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        assert!(
            wait_until_or_stop(&stop, deadline),
            "停止フラグ設定済みなら即座に true を返すべきです"
        );
    }

    // deadline を 1 秒前に設定し、到達済みなら sleep せずに即座に false を返すことを確認する。
    #[test]
    fn wait_until_or_stop_returns_false_when_deadline_passed() {
        let stop = AtomicBool::new(false);
        let deadline = std::time::Instant::now() - std::time::Duration::from_secs(1);
        assert!(
            !wait_until_or_stop(&stop, deadline),
            "deadline 到達済みなら即座に false を返すべきです"
        );
    }

    // 実 thread で、sleep 中に stop フラグが設定された場合に
    // 最大 MAX_SLEEP_DURATION 以内で終了することを確認する。
    //
    // barrier でテストスレッドの wait_until_or_stop 呼び出し直前までを同期し、
    // 最初の stop チェックを通過して sleep に入るのを待ってから stop を設定する。
    // これにより、sleep 中に stop が設定される経路を確実に実行する。
    #[test]
    fn wait_until_or_stop_stops_within_sleep_limit() {
        let stop = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (done_tx, done_rx) = std::sync::mpsc::channel();

        let stop_clone = stop.clone();
        let barrier_clone = barrier.clone();
        thread::spawn(move || {
            barrier_clone.wait();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            let result = wait_until_or_stop(&stop_clone, deadline);
            done_tx.send(result).expect("終了通知の送信に失敗しました");
        });

        barrier.wait();
        // テストスレッドが最初の stop チェックを通過して sleep に入るのを待ってから
        // stop を設定する (sleep 中の stop 検出経路を確実に実行するためのタイミング調整)。
        thread::sleep(MAX_SLEEP_DURATION / 2);
        stop.store(true, Ordering::Release);

        let stopped = done_rx
            .recv_timeout(MAX_SLEEP_DURATION + std::time::Duration::from_millis(100))
            .expect("待機中のスレッドは停止フラグ設定から MAX_SLEEP_DURATION に余裕を加えた時間以内に終了するべきです");
        assert!(
            stopped,
            "stop による停止 (true) を期待しましたが、実際は: {stopped:?}"
        );
    }

    // 1 つの reader をクローンして複数スレッドから同時に get_sample を呼んでも、
    // 各スレッドが同じファイルから正しいサンプルを独立に読み出せることを確認する。
    //
    // I/O スレッドへの直列化と request / response の正しさの回帰テスト。
    // 全スレッドの同一 index サンプルがバイト列まで一致することを、
    // スレッド数 × サンプル数の組み合わせで検証する。
    #[test]
    fn shared_reader_clones_read_samples_concurrently() {
        let (reader, _fixture) = h264_reader_from_fixture("shared-reader");
        let sample_count = reader.len();
        let thread_count = 4;

        // 各スレッドが全サンプルを読み出す。reader は clone で共有される。
        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let reader = reader.clone();
                thread::spawn(move || {
                    // 中断テストではないため、停止フラグは常に false のまま渡す。
                    let stop = AtomicBool::new(false);
                    let mut samples = Vec::new();
                    for i in 0..sample_count {
                        let sample = reader
                            .get_sample(i, &stop)
                            .expect("共有 reader からのサンプル読み出しに失敗しました")
                            .expect("停止フラグは未設定のため中断されないはずです");
                        samples.push(sample);
                    }
                    samples
                })
            })
            .collect();

        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("共有 reader の読み出しスレッドが panic しました")
            })
            .collect();
        // 同一 index のサンプルは全スレッドで同一バイト列・同一フラグになるはず。
        for (i, reference) in results[0].iter().enumerate() {
            for other_results in results.iter().skip(1) {
                assert_eq!(
                    other_results[i].data, reference.data,
                    "index {i} のサンプルデータは全スレッドで一致するはずです"
                );
                assert_eq!(
                    other_results[i].is_keyframe, reference.is_keyframe,
                    "index {i} のキーフレーム判定は全スレッドで一致するはずです"
                );
            }
        }
        // 先頭サンプルはキーフレームで、H.264 の parameter sets (SPS/PPS) 付きであること。
        let first = &results[0][0];
        assert!(first.is_keyframe, "先頭サンプルはキーフレームのはずです");
        assert!(
            first.data.starts_with(&[0x00, 0x00, 0x00, 0x01]),
            "先頭サンプルは SPS/PPS のスタートコードで始まるはずです"
        );
        assert_eq!(first.width, 320, "先頭サンプルの幅は 320 のはずです");
        assert_eq!(first.height, 320, "先頭サンプルの高さは 320 のはずです");
        assert_eq!(
            first.codec_type,
            VideoCodecType::H264,
            "先頭サンプルのコーデックは H.264 のはずです"
        );
    }

    // 停止フラグが設定された状態で get_sample を呼ぶと、I/O 応答を待たずに
    // Ok(None) を返すことを確認する。
    //
    // 共有 I/O スレッドが他 capturer の要求で塞がっていても停止・ join を妨げない
    // という契約の回帰テスト。依頼は送信済みだが、応答を待つ前に stop を観測して
    // 中断する (中断された依頼は I/O スレッドが後で処理し、送信先が無いため破棄される)。
    #[test]
    fn get_sample_returns_none_when_stopped() {
        let (reader, _fixture) = h264_reader_from_fixture("get-sample-stopped");
        let stop = AtomicBool::new(true);
        let result = reader
            .get_sample(0, &stop)
            .expect("停止時の読み出しは I/O エラーにはならないはずです");
        assert!(
            result.is_none(),
            "停止フラグ設定時は応答を待たずに None を返すはずです"
        );
    }

    /// テスト用の `VideoSinkHandler`。
    ///
    /// 受信したフレームの情報を mpsc チャネルへ送り、テスト本体から観測できるようにする。
    struct TestVideoSink {
        tx: std::sync::mpsc::Sender<TestFrameInfo>,
    }

    /// テスト用に観測するフレーム情報。
    struct TestFrameInfo {
        width: i32,
        height: i32,
        codec_type: VideoCodecType,
    }

    impl VideoSinkHandler for TestVideoSink {
        fn on_frame(&mut self, frame: VideoFrameRef<'_>) {
            let buffer = frame.buffer();
            // 安全性: on_frame 内でのみ参照を取得する。各 capturer は専用の
            // VideoFrameBuffer を作るため、同一実体への同時アクセスは発生しない。
            let codec_type = unsafe { buffer.as_native_ref::<Mp4EncodedSample>() }
                .expect("パススルーサンプルは VideoFrameBuffer に内包されているはずです")
                .codec_type;
            let _ = self.tx.send(TestFrameInfo {
                width: buffer.width(),
                height: buffer.height(),
                codec_type,
            });
        }
    }

    // 1 つの reader を clone して複数の Mp4VideoCapturer を同時に動かし、
    // 各 video_source() がそれぞれ正常なフレームを供給し続けられることを確認する。
    #[test]
    fn multiple_capturers_share_single_reader() {
        // 実フレームを観測するため、実際の VideoTrack を生成できるコンテキストを使う。
        let context = crate::connection_context::SoraConnectionContext::new()
            .expect("SoraConnectionContext の生成に失敗しました");
        let (reader, _fixture) = h264_reader_from_fixture("shared-capturer");
        let capturer_count = 2;

        // reader.clone() を渡して capturer を複数作る。
        let mut capturers = Vec::new();
        let mut tracks = Vec::new();
        let mut sinks = Vec::new();
        for _ in 0..capturer_count {
            let capturer = Mp4VideoCapturer::new(reader.clone())
                .expect("共有 reader からの Mp4VideoCapturer 生成に失敗しました");
            let (tx, rx) = std::sync::mpsc::channel();
            let sink = VideoSink::new_with_handler(Box::new(TestVideoSink { tx }));
            let mut track = context
                .create_video_track(&capturer.video_source())
                .expect("VideoTrack の生成に失敗しました");
            track.add_or_update_sink(&sink, &VideoSinkWants::new());
            capturers.push(capturer);
            tracks.push(track);
            sinks.push((sink, rx));
        }

        // 各 capturer が 10 フレームを連続受信できることを、タイムアウト付きで待って確認する。
        // フィクスチャは 25fps のため 10 フレームは約 400ms 分に相当し、供給が継続している
        // ことを表す。固定 sleep は負荷時にフレークし得るため、recv_timeout で必要数を受信する。
        for (index, (_sink, rx)) in sinks.iter().enumerate() {
            // sink と track は保持し続ける必要がある (drop すると登録が解除される)。
            for _ in 0..10 {
                let frame = rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap_or_else(|_| {
                        panic!("capturer {index} は 5 秒以内に 10 フレーム供給できるはずです")
                    });
                assert_eq!(
                    frame.width, 320,
                    "capturer {index} のフレーム幅は 320 のはずです"
                );
                assert_eq!(
                    frame.height, 320,
                    "capturer {index} のフレーム高さは 320 のはずです"
                );
                assert_eq!(
                    frame.codec_type,
                    VideoCodecType::H264,
                    "capturer {index} のフレームコーデックは H.264 のはずです"
                );
            }
        }

        // feeder スレッドを先に停止してから sink の登録を解除する。
        // 登録されたまま sink を drop すると、feeder スレッドが解放済みの
        // sink へフレームを送り続け、解放済みメモリ参照 (SIGSEGV) の原因になる。
        drop(capturers);
        for (index, (sink, _rx)) in sinks.iter().enumerate() {
            tracks[index].remove_sink(sink);
        }
    }
}
