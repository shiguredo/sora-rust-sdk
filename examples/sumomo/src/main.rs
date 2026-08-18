#[cfg(feature = "media-device")]
mod adm;
mod ansi_renderer;
mod args;
#[cfg(feature = "media-device")]
mod audio_device;
mod error;
mod fake;
#[cfg(feature = "raw-player")]
mod raw_player_renderer;
#[cfg(test)]
mod tests;
mod video;
mod video_codec_list;
mod video_device;

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[cfg(feature = "media-device")]
use adm::SumomoAdm;
use ansi_renderer::AnsiRenderer;
use args::Args;
use args::{
    VideoCodecImplementationSelection, VideoCodecImplementationSelections, parse_args,
    validate_args,
};
#[cfg(feature = "media-device")]
use audio_device::AudioDeviceCapturer;
use error::{AppError, Result};
use fake::{FakeVideoCapturer, FakeVideoCapturerConfig};
#[cfg(feature = "raw-player")]
use raw_player_renderer::RawPlayerRenderer;
use rustls_pki_types::pem::PemObject;
use shiguredo_webrtc::{
    VideoCodecType, VideoSink, VideoSinkWants, VideoTrack, log, rtc_log_info, rtc_log_warning,
};
#[cfg(feature = "amf")]
use sora_sdk::AmfVideoCodecCapability;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use sora_sdk::InternalAppleVideoCodecCapability;
#[cfg(feature = "nvcodec")]
use sora_sdk::NvCodecVideoCodecCapability;
#[cfg(feature = "v4l2")]
use sora_sdk::V4l2VideoCodecCapability;
#[cfg(feature = "vpl")]
use sora_sdk::VplVideoCodecCapability;
use sora_sdk::{
    CodecDirection, InternalVideoCodecCapability, Mp4PassthroughVideoCodecCapability,
    Mp4SampleReader, Mp4VideoCapturer, Openh264VideoCodecCapability, PreferenceCodec,
    SoraConnection, SoraConnectionBuilder, SoraConnectionContext, SoraConnectionContextConfig,
    SoraConnectionEventHandler, SoraConnectionHandle, VideoCodecCapability, VideoCodecPreference,
};
use tokio::sync::mpsc;
use video::{I420Frame, VideoFrameSinkHandler, VideoRenderer};
use video_codec_list::run_video_codec_list;
#[cfg(test)]
pub(crate) use video_codec_list::{
    VideoCodecCapabilityReport, VideoCodecListReport, VideoCodecPreferenceReport,
    build_video_codec_list_report_text, collect_video_codec_list_report,
};
use video_device::VideoCapturerHolder;
#[cfg(feature = "media-device")]
use video_device::{VideoDeviceCapturer, list_devices};

enum AppEvent {
    Notify(String),
    Push(String),
    OnTrack(shiguredo_webrtc::RtpTransceiver),
    OnRemoveTrack(shiguredo_webrtc::RtpReceiver),
}

/// 終了処理全体に与える application-level timeout。
///
/// SDK の既定値である WebSocket close 3 秒と DataChannel close 5 秒の合計を収め、
/// 接続開始時の 30 秒 timeout より短く local 終了要求を bound する値。
const CONNECTION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

fn add_video_codec_capability(
    context_config: &mut SoraConnectionContextConfig,
    capability: Box<dyn VideoCodecCapability>,
) {
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());
    context_config.video_codec_preference.merge(&preference);
    context_config.video_codec_capabilities.push(capability);
}

fn build_context_config(
    adm_config: sora_sdk::AdmConfig,
    mp4_capability: Option<Mp4PassthroughVideoCodecCapability>,
    openh264_path: Option<&str>,
    video_codec_implementation: VideoCodecImplementationSelections,
) -> Result<SoraConnectionContextConfig> {
    let mut context_config = match video_codec_implementation {
        VideoCodecImplementationSelections::Auto => SoraConnectionContextConfig {
            adm_config,
            ..Default::default()
        },
        VideoCodecImplementationSelections::Manual(_) => SoraConnectionContextConfig {
            adm_config,
            video_codec_preference: VideoCodecPreference::default(),
            video_codec_capabilities: Vec::new(),
        },
    };

    match video_codec_implementation {
        VideoCodecImplementationSelections::Auto => {}
        VideoCodecImplementationSelections::Manual(selections) => {
            for selection in selections {
                match selection {
                    VideoCodecImplementationSelection::Internal => {
                        let internal_capability: Box<dyn VideoCodecCapability> =
                            Box::new(InternalVideoCodecCapability::new());
                        add_video_codec_capability(&mut context_config, internal_capability);
                    }
                    VideoCodecImplementationSelection::InternalApple => {
                        #[cfg(any(target_os = "macos", target_os = "ios"))]
                        {
                            let capability =
                                InternalAppleVideoCodecCapability::new().ok_or_else(|| {
                                    io::Error::other(
                                        "internal-apple is not available on this device",
                                    )
                                })?;
                            let capability: Box<dyn VideoCodecCapability> = Box::new(capability);
                            add_video_codec_capability(&mut context_config, capability);
                        }
                        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
                        {
                            return Err(io::Error::other(
                                "internal-apple is not supported on this platform",
                            )
                            .into());
                        }
                    }
                    VideoCodecImplementationSelection::Amf => {
                        #[cfg(feature = "amf")]
                        {
                            let amf_capability: Box<dyn VideoCodecCapability> =
                                Box::new(AmfVideoCodecCapability::new()?);
                            add_video_codec_capability(&mut context_config, amf_capability);
                        }
                        #[cfg(not(feature = "amf"))]
                        {
                            return Err(io::Error::other(
                                "AMF is not enabled in this build. Rebuild sumomo with --features amf",
                            )
                            .into());
                        }
                    }
                    VideoCodecImplementationSelection::Nvcodec => {
                        #[cfg(feature = "nvcodec")]
                        {
                            let nvcodec_capability: Box<dyn VideoCodecCapability> =
                                Box::new(NvCodecVideoCodecCapability::new()?);
                            add_video_codec_capability(&mut context_config, nvcodec_capability);
                        }
                        #[cfg(not(feature = "nvcodec"))]
                        {
                            return Err(io::Error::other(
                                "NVCodec is not enabled in this build. Rebuild sumomo with --features nvcodec",
                            )
                            .into());
                        }
                    }
                    VideoCodecImplementationSelection::Vpl => {
                        #[cfg(feature = "vpl")]
                        {
                            let vpl_capability: Box<dyn VideoCodecCapability> =
                                Box::new(VplVideoCodecCapability::new()?);
                            add_video_codec_capability(&mut context_config, vpl_capability);
                        }
                        #[cfg(not(feature = "vpl"))]
                        {
                            return Err(io::Error::other(
                                "VPL is not enabled in this build. Rebuild sumomo with --features vpl",
                            )
                            .into());
                        }
                    }
                    VideoCodecImplementationSelection::V4l2 => {
                        #[cfg(feature = "v4l2")]
                        {
                            let v4l2_capability: Box<dyn VideoCodecCapability> =
                                Box::new(V4l2VideoCodecCapability::new()?);
                            add_video_codec_capability(&mut context_config, v4l2_capability);
                        }
                        #[cfg(not(feature = "v4l2"))]
                        {
                            return Err(io::Error::other(
                                "V4L2 is not enabled in this build. Rebuild sumomo with --features v4l2",
                            )
                            .into());
                        }
                    }
                    VideoCodecImplementationSelection::Openh264 => {
                        let path = openh264_path.ok_or_else(|| {
                            io::Error::other(
                                "--video-codec-implementation openh264 requires --openh264-path",
                            )
                        })?;
                        let openh264_capability: Box<dyn VideoCodecCapability> =
                            Box::new(Openh264VideoCodecCapability::new(path)?);
                        add_video_codec_capability(&mut context_config, openh264_capability);
                    }
                }
            }
        }
    }

    // MP4 使用時は送信 (Encoder) に passthrough のみを使い、受信 (Decoder) は選択された実装を維持する。
    //
    // passthrough capability は必ず上の Manual / Auto の capability 追加より後に追加すること。
    // VideoCodecPreference::merge は後勝ち規則 (方向・codec が一致する既存エントリの
    // implementation を上書きする) のため、この順序が「MP4 の実 codec の Encoder が
    // passthrough になる」ことの不変条件になっている。順序が変わると下のフィルタで
    // Encoder エントリが 0 件になり、MP4 送信が静かに成立しなくなる。
    if let Some(capability) = mp4_capability {
        let passthrough_capability: Box<dyn VideoCodecCapability> = Box::new(capability);
        let passthrough_implementation = passthrough_capability.get_implementation();
        add_video_codec_capability(&mut context_config, passthrough_capability);

        // Encoder 方向は MP4 の実 codec の passthrough だけを残し、
        // 他の codec 実装の Encoder エントリが送信に使われないように除去する。
        // Decoder 方向は選択された実装をそのまま維持して受信デコードに使う。
        let codecs: Vec<PreferenceCodec> = context_config
            .video_codec_preference
            .codecs()
            .iter()
            .filter(|codec| {
                codec.direction() == CodecDirection::Decoder
                    || codec.implementation() == &passthrough_implementation
            })
            .cloned()
            .collect();
        context_config.video_codec_preference = VideoCodecPreference::new(codecs);
    }

    Ok(context_config)
}

struct TrackEntry {
    sink: VideoSink,
    video_track: VideoTrack,
}

fn prepare_mp4_state(args: &Args) -> Result<Option<Mp4SampleReader>> {
    if let Some(ref mp4_path) = args.input_mp4 {
        Ok(Some(Mp4SampleReader::new(mp4_path)?))
    } else {
        Ok(None)
    }
}

/// [VideoCodecType] からシグナリング用の [sora_sdk::Video] を生成する。
///
/// [VideoCodecType::Generic] や [VideoCodecType::Unknown] の場合はエラーになる
fn video_from_codec_type(
    codec_type: VideoCodecType,
    bit_rate: Option<u32>,
) -> Result<sora_sdk::Video> {
    match codec_type {
        VideoCodecType::Vp8 => Ok(sora_sdk::Video::new_vp8(bit_rate)),
        VideoCodecType::Vp9 => Ok(sora_sdk::Video::new_vp9(bit_rate, None)),
        VideoCodecType::Av1 => Ok(sora_sdk::Video::new_av1(bit_rate, None)),
        VideoCodecType::H264 => Ok(sora_sdk::Video::new_h264(bit_rate, None)),
        VideoCodecType::H265 => Ok(sora_sdk::Video::new_h265(bit_rate, None)),
        VideoCodecType::Generic | VideoCodecType::Unknown(_) => {
            Err(io::Error::other(format!("unsupported video codec type: {codec_type:?}")).into())
        }
    }
}

fn apply_video_options(
    mut builder: SoraConnectionBuilder,
    args: &Args,
    mp4_codec_type: Option<VideoCodecType>,
) -> Result<SoraConnectionBuilder> {
    // MP4 使用時は MP4 から検出した実際のコーデックを使う (--video-codec-type とは併用不可)。
    // ただし、受信専用 (RecvOnly) では MP4 のコーデックを利用せず、--video-codec-type に従う。
    // `--input-mp4` は送信専用のオプションであるため、RecvOnly 時の `video` の設定へ波及させない。
    let video_codec_type = if args.role.wants_send() {
        mp4_codec_type.or(args.video_codec_type)
    } else {
        args.video_codec_type
    };

    if let Some(video) = args.video {
        if video {
            let video_setting = match video_codec_type {
                Some(codec_type) => video_from_codec_type(codec_type, args.video_bit_rate)?,
                None => sora_sdk::Video::new_bool(true),
            };
            builder = builder.video(video_setting);
        } else {
            builder = builder.video(sora_sdk::Video::new_bool(false));
        }
    } else if let Some(codec_type) = video_codec_type {
        builder = builder.video(video_from_codec_type(codec_type, args.video_bit_rate)?);
    }
    Ok(builder)
}

// SoraConnectionEventHandler は同期トレイトであるため、
// チャンネルがフルになった時に待つことが出来ない。
// そのためイベントチャネルは unbounded にする。
struct AppEventHandler {
    event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl SoraConnectionEventHandler for AppEventHandler {
    fn on_notify(&mut self, text: &str) {
        let _ = self.event_tx.send(AppEvent::Notify(text.to_string()));
    }

    fn on_push(&mut self, text: &str) {
        let _ = self.event_tx.send(AppEvent::Push(text.to_string()));
    }

    fn on_track(&mut self, transceiver: shiguredo_webrtc::RtpTransceiver) {
        let _ = self.event_tx.send(AppEvent::OnTrack(transceiver));
    }

    fn on_remove_track(&mut self, receiver: shiguredo_webrtc::RtpReceiver) {
        let _ = self.event_tx.send(AppEvent::OnRemoveTrack(receiver));
    }
}

fn build_connection_builder(
    context: Arc<SoraConnectionContext>,
    args: &Args,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    mp4_codec_type: Option<VideoCodecType>,
) -> Result<SoraConnectionBuilder> {
    let mut builder = SoraConnection::builder(
        context,
        args.signaling_urls.clone(),
        args.channel_id.clone(),
        args.role,
        AppEventHandler { event_tx },
    );
    if let Some(metadata) = &args.metadata {
        // --metadata は JSON 文字列を受け取り、Sora の認証メタデータとして送信する。
        let metadata = metadata.parse::<sora_sdk::JsonString>()?;
        builder = builder.metadata(metadata);
    }
    if let Some(audio) = args.audio {
        builder = builder.audio(sora_sdk::Audio::new_bool(audio));
    }
    builder = apply_video_options(builder, args, mp4_codec_type)?;
    if let Some(data_channel_signaling) = args.data_channel_signaling {
        builder = builder.data_channel_signaling(data_channel_signaling);
    }
    if let Some(ignore_disconnect_websocket) = args.ignore_disconnect_websocket {
        builder = builder.ignore_disconnect_websocket(ignore_disconnect_websocket);
    }
    if let Some(simulcast) = args.simulcast {
        builder = builder.simulcast(simulcast);
    }
    builder = builder.insecure(args.insecure);
    if let (Some(cert), Some(key)) = (args.client_cert.clone(), args.client_key.clone()) {
        builder = builder.client_cert(cert, key);
    }
    if let Some(ca) = args.ca_cert.clone() {
        builder = builder.ca_cert(ca);
    }
    if args.turn_tls_insecure {
        builder = builder.turn_tls_insecure(true);
    }
    if let Some(ref ca_cert_path) = args.turn_tls_ca_cert {
        let pem_data = std::fs::read(ca_cert_path)?;
        let cert = rustls_pki_types::CertificateDer::from_pem_slice(&pem_data)?;
        builder = builder.turn_tls_ca_cert(cert.to_vec());
    }
    Ok(builder)
}

fn create_video_capturer(
    args: &Args,
    mp4_reader: Option<Mp4SampleReader>,
) -> Result<VideoCapturerHolder> {
    if let Some(reader) = mp4_reader {
        let mp4_capturer = Mp4VideoCapturer::new(reader)?;
        return Ok(VideoCapturerHolder::Mp4(mp4_capturer));
    }

    #[cfg(feature = "libcamera")]
    {
        if args.use_libcamera {
            let mut builder = sora_sdk::LibcameraVideoCapturer::builder()
                .width(640)
                .height(480)
                .native_frame_output(args.use_libcamera_native);
            for (key, value) in &args.libcamera_controls {
                builder = builder.control(key.clone(), value.clone());
            }
            return Ok(VideoCapturerHolder::Libcamera(builder.build()?));
        }
    }

    #[cfg(not(feature = "libcamera"))]
    {
        if args.use_libcamera {
            return Err(io::Error::other(
                "libcamera is not enabled in this build. Rebuild sumomo with --features libcamera",
            )
            .into());
        }
    }

    #[cfg(feature = "media-device")]
    {
        if let Some(ref device_id) = args.video_input_device {
            return Ok(VideoCapturerHolder::Device(VideoDeviceCapturer::new(
                Some(device_id.clone()),
            )?));
        }
    }

    let fake = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())?;
    Ok(VideoCapturerHolder::Fake(fake))
}

fn attach_sender_tracks(
    mut builder: SoraConnectionBuilder,
    context: &Arc<SoraConnectionContext>,
    args: &Args,
    mp4_reader: Option<Mp4SampleReader>,
) -> Result<(SoraConnectionBuilder, Option<VideoCapturerHolder>)> {
    let video_enabled = args.video_enabled();
    let audio_enabled = args.audio_enabled();
    let mut video_capturer = None;
    if args.role.wants_send() && video_enabled {
        let mut capturer = create_video_capturer(args, mp4_reader)?;
        capturer.start()?;
        let video_track = context.create_video_track(&capturer.video_source())?;
        builder = builder.sender_video_track(video_track);
        video_capturer = Some(capturer);
    }

    if args.role.wants_send() && audio_enabled {
        let audio_source = context.create_audio_source()?;
        let audio_track = context.create_audio_track(&audio_source)?;
        builder = builder.sender_audio_track(audio_track);
    }

    Ok((builder, video_capturer))
}

fn handle_on_track_event<F>(
    tracks: &mut HashMap<String, TrackEntry>,
    transceiver: shiguredo_webrtc::RtpTransceiver,
    replace_existing: bool,
    build_sink: F,
) where
    F: FnOnce(String) -> VideoSink,
{
    let receiver = transceiver.receiver();
    let track = receiver.track();
    let kind = match track.kind() {
        Ok(kind) => kind,
        Err(_) => "unknown".to_string(),
    };
    if kind != "video" {
        rtc_log_warning!("Received non-video track: kind={}", kind);
        return;
    }
    let track_id = match track.id() {
        Ok(id) => id,
        Err(_) => {
            rtc_log_warning!("Failed to get MediaStreamTrack id");
            return;
        }
    };

    let mut video_track = track.cast_to_video_track();
    if let Some(old_entry) = tracks.remove(&track_id) {
        if !replace_existing {
            tracks.insert(track_id, old_entry);
            return;
        }
        rtc_log_info!("Removing existing track: track_id={}", track_id);
        video_track.remove_sink(&old_entry.sink);
    }

    rtc_log_info!("Video track added: track_id={}", track_id);
    let sink = build_sink(track_id.clone());
    let wants = VideoSinkWants::new();
    video_track.add_or_update_sink(&sink, &wants);
    tracks.insert(track_id, TrackEntry { sink, video_track });
}

fn handle_on_remove_track_event(
    tracks: &mut HashMap<String, TrackEntry>,
    receiver: shiguredo_webrtc::RtpReceiver,
) {
    let track = receiver.track();
    let track_id = match track.id() {
        Ok(id) => id,
        Err(_) => {
            rtc_log_warning!("Failed to get MediaStreamTrack id");
            return;
        }
    };
    let kind = match track.kind() {
        Ok(kind) => kind,
        Err(_) => "unknown".to_string(),
    };
    if kind != "video" {
        rtc_log_warning!("Non-video track removed: kind={}", kind);
        return;
    }
    let mut video_track = track.cast_to_video_track();
    if let Some(entry) = tracks.remove(&track_id) {
        video_track.remove_sink(&entry.sink);
    }
    rtc_log_info!("Video track removed: track_id={}", track_id);
}

/// 受信 track の sink を外して callback source を停止する。
///
/// connection を構築し、`run()` を別タスクとして開始する。
///
/// capturer は run タスク内で保持され、run 完了時に drop される。
/// disconnect に使う [SoraConnectionHandle] と、run の完了を待つ `JoinHandle` を返す。
fn build_and_run_connection(
    args: &Args,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) -> Result<(
    SoraConnectionHandle,
    tokio::task::JoinHandle<sora_sdk::Result<()>>,
)> {
    // 送信ロールかつ音声が有効で、--audio-input-device が指定された場合は SumomoAdm を使用する。
    #[cfg(feature = "media-device")]
    let external_adm =
        if args.role.wants_send() && args.audio_enabled() && args.audio_input_device.is_some() {
            Some(SumomoAdm::new())
        } else {
            None
        };

    // --input-mp4 が指定されている場合は MP4 を読み込んでパススルーの準備をする
    let mp4_state = prepare_mp4_state(args)?;
    let mp4_codec_type = mp4_state.as_ref().map(|reader| reader.codec_type());

    #[cfg(feature = "media-device")]
    let adm_config = if let Some(external_adm) = &external_adm {
        sora_sdk::AdmConfig::UseExternal(external_adm.audio_device_module())
    } else {
        sora_sdk::AdmConfig::NoAudioDevice
    };
    #[cfg(not(feature = "media-device"))]
    let adm_config = sora_sdk::AdmConfig::NoAudioDevice;

    let context_config = build_context_config(
        adm_config,
        mp4_state
            .as_ref()
            .map(|reader| reader.passthrough_capability()),
        args.openh264_path.as_deref(),
        args.video_codec_implementation.clone(),
    )?;
    let context = SoraConnectionContext::new_with_config(context_config)?;

    // 送信ロールかつ音声が有効で、--audio-input-device が指定された場合は AudioDeviceCapturer を使用する。
    #[cfg(feature = "media-device")]
    let audio_capturer = if args.role.wants_send() && args.audio_enabled() {
        if let Some(ref device_id) = args.audio_input_device {
            let state = external_adm
                .as_ref()
                .expect("BUG: external_adm が None です")
                .state();
            let mut capturer = AudioDeviceCapturer::new(Some(device_id.clone()), state)?;
            capturer.start()?;
            rtc_log_info!("Started audio input device: {}", device_id);
            Some(capturer)
        } else {
            None
        }
    } else {
        None
    };

    let builder = build_connection_builder(context.clone(), args, event_tx, mp4_codec_type)?;
    let (builder, video_capturer) = attach_sender_tracks(builder, &context, args, mp4_state)?;

    let (connection, handle) = builder.build()?;
    let run_handle = tokio::spawn(async move {
        // capturer は run タスク内で保持し、run 完了時に drop する。
        #[cfg(feature = "media-device")]
        let _audio_capturer = audio_capturer;
        let _video_capturer = video_capturer;
        connection.run().await
    });
    Ok((handle, run_handle))
}

/// connection を終了する。
///
/// `run()` が先に完了している場合は disconnect を送らず、その結果を返す。
/// それ以外は disconnect command を送って `run()` の完了を、`deadline` の
/// 内側で待つ。`run()` は別タスクで動いているため、disconnect を先に await しても
/// command が処理されて deadlock しない。
async fn shutdown_connection(
    handle: SoraConnectionHandle,
    run_handle: tokio::task::JoinHandle<sora_sdk::Result<()>>,
    deadline: tokio::time::Instant,
) -> Result<()> {
    if run_handle.is_finished() {
        let result = run_handle.await.map_err(|_| AppError::WorkerPanic)?;
        return result.map_err(AppError::Sora);
    }

    tokio::time::timeout_at(deadline, async {
        handle.disconnect().await?;
        let result = run_handle.await.map_err(|_| AppError::WorkerPanic)?;
        result.map_err(AppError::Sora)
    })
    .await
    .map_err(|_| AppError::ConnectionShutdownTimeout)?
}

/// duration が指定されている場合はタイマーを設定する。
fn create_duration_sleep(args: &Args) -> Option<tokio::time::Sleep> {
    args.duration.map(|secs| {
        rtc_log_info!("Will disconnect after {} seconds", secs);
        tokio::time::sleep(Duration::from_secs(secs))
    })
}

/// 受信 track の sink を外してから、最初に観測した renderer error を primary として
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = parse_args(noargs::raw_args())?;

    // codec list モードは接続処理を行わず早期終了する。
    if args.video_codec_list {
        return run_video_codec_list(&args);
    }

    log::log_to_debug(log::Severity::Info);
    log::enable_timestamps();
    log::enable_threads();

    validate_args(&args)?;

    #[cfg(feature = "media-device")]
    if args.list_devices {
        return list_devices();
    }

    // レンダラーを生成する。raw-player は SDL を、それ以外は ANSI を使う。
    #[cfg(feature = "raw-player")]
    let mut renderer = if args.use_raw_player {
        VideoRenderer::RawPlayer(RawPlayerRenderer::new("Sumomo - Video", 640, 480)?)
    } else {
        VideoRenderer::Ansi(AnsiRenderer::new())
    };
    #[cfg(not(feature = "raw-player"))]
    let mut renderer = VideoRenderer::Ansi(AnsiRenderer::new());

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (frame_tx, mut frame_rx) = mpsc::channel::<I420Frame>(2);

    let (handle, mut run_handle) = build_and_run_connection(&args, event_tx.clone())?;
    let mut tracks: HashMap<String, TrackEntry> = HashMap::new();

    // duration が指定されている場合はタイマーを設定
    let duration_sleep = create_duration_sleep(&args);
    tokio::pin!(duration_sleep);

    // run 完了 (server 起因)、duration 経過、event channel close、renderer error の
    // いずれかを検出したらループを抜ける。
    let mut renderer_error: Option<AppError> = None;
    let mut frame_count = 0u64;
    loop {
        tokio::select! {
            _ = &mut run_handle => {
                // server 起因で run() が先に完了した場合も shutdown_connection が結果を返す。
                break;
            }
            _ = async { duration_sleep.as_mut().as_pin_mut().unwrap().await }, if duration_sleep.is_some() && renderer_error.is_none() => {
                rtc_log_info!("Specified duration elapsed, disconnecting");
                break;
            }
            event = event_rx.recv() => {
                let Some(event) = event else {
                    // 通常の event channel が閉じた場合は終了処理へ進む
                    break;
                };
                match event {
                    AppEvent::Notify(text) => {
                        rtc_log_info!("Received notify: {}", text);
                    }
                    AppEvent::Push(text) => {
                        rtc_log_info!("Received push: {}", text);
                    }
                    AppEvent::OnTrack(transceiver) => {
                        handle_on_track_event(&mut tracks, transceiver, false, |track_id| {
                            let frame_tx = frame_tx.clone();
                            let first_frame = Arc::new(AtomicBool::new(false));
                            VideoSink::new_with_handler(Box::new(VideoFrameSinkHandler {
                                frame_tx,
                                first_frame,
                                track_id_for_log: track_id,
                            }))
                        });
                    }
                    AppEvent::OnRemoveTrack(receiver) => {
                        handle_on_remove_track_event(&mut tracks, receiver);
                    }
                }
            }
            frame = frame_rx.recv() => {
                let Some(frame) = frame else {
                    // frame channel が閉じた場合は終了処理へ進む
                    break;
                };
                frame_count += 1;
                if frame_count == 1 {
                    rtc_log_info!(
                        "first frame received: {}x{}",
                        frame.width,
                        frame.height
                    );
                }
                if let Err(err) = renderer.render_frame(&frame) {
                    // renderer の render error を main loop で検出し、primary error として保持する。
                    renderer_error = Some(err);
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                // raw-player では SDL の window close / Escape を検出する。
                renderer.poll_events();
                if !renderer.is_running() {
                    break;
                }
            }
        }
    }

    let deadline = tokio::time::Instant::now() + CONNECTION_SHUTDOWN_TIMEOUT;
    let shutdown_result = shutdown_connection(handle, run_handle, deadline).await;

    // 受信 track の sink を外して callback source を停止する。
    for entry in tracks.values_mut() {
        entry.video_track.remove_sink(&entry.sink);
    }

    // 最初に観測した renderer error を primary として返す。
    match renderer_error {
        Some(err) => Err(err),
        None => shutdown_result,
    }
}
