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
mod video_codec_list;
mod video_device;

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(feature = "raw-player")]
use std::sync::atomic::Ordering;
#[cfg(feature = "raw-player")]
use std::thread;

#[cfg(feature = "media-device")]
use adm::SumomoAdm;
use ansi_renderer::{AnsiRenderer, AnsiTrackSinkHandler};
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
use raw_player_renderer::{I420Frame, RawPlayerRenderer, RawPlayerTrackSinkHandler};
use rustls_pki_types::pem::PemObject;
use shiguredo_webrtc::{
    VideoCodecType, VideoSink, VideoSinkWants, log, rtc_log_info, rtc_log_warning,
};
#[cfg(feature = "amf")]
use sora_sdk::AmfVideoCodecCapability;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use sora_sdk::InternalAppleVideoCodecCapability;
#[cfg(feature = "nvcodec")]
use sora_sdk::NvCodecVideoCodecCapability;
#[cfg(feature = "vpl")]
use sora_sdk::VplVideoCodecCapability;
use sora_sdk::{
    InternalVideoCodecCapability, Mp4PassthroughVideoCodecCapability, Mp4SampleReader,
    Mp4VideoCapturer, Openh264VideoCodecCapability, SoraClient, SoraClientBuilder,
    SoraClientContext, SoraClientContextConfig, VideoCodecCapability, VideoCodecPreference,
};
use tokio::sync::mpsc;
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

trait AppEventSender: Clone + Send + Sync + 'static {
    fn send_event(&self, event: AppEvent);
}

impl AppEventSender for mpsc::Sender<AppEvent> {
    fn send_event(&self, event: AppEvent) {
        let _ = self.try_send(event);
    }
}

#[cfg(feature = "raw-player")]
impl AppEventSender for std::sync::mpsc::Sender<AppEvent> {
    fn send_event(&self, event: AppEvent) {
        let _ = self.send(event);
    }
}

fn add_video_codec_capability(
    context_config: &mut SoraClientContextConfig,
    capability: Box<dyn VideoCodecCapability>,
) {
    let preference = VideoCodecPreference::new_from_capability(capability.as_ref());
    context_config.video_codec_preference.merge(&preference);
    context_config.video_codec_capabilities.push(capability);
}

fn build_context_config(
    adm_config: sora_sdk::AdmConfig,
    mp4_codec_type: Option<VideoCodecType>,
    openh264_path: Option<&str>,
    video_codec_implementation: VideoCodecImplementationSelections,
) -> Result<SoraClientContextConfig> {
    let mut context_config = match video_codec_implementation {
        VideoCodecImplementationSelections::Auto => SoraClientContextConfig {
            adm_config,
            ..Default::default()
        },
        VideoCodecImplementationSelections::Manual(_) => SoraClientContextConfig {
            adm_config,
            video_codec_preference: VideoCodecPreference::default(),
            video_codec_capabilities: Vec::new(),
        },
    };

    if let Some(codec_type) = mp4_codec_type {
        let passthrough_capability: Box<dyn VideoCodecCapability> =
            Box::new(Mp4PassthroughVideoCodecCapability::new(codec_type));
        add_video_codec_capability(&mut context_config, passthrough_capability);
    }

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
                                Box::new(NvCodecVideoCodecCapability::new());
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

    Ok(context_config)
}

struct TrackEntry {
    sink: VideoSink,
}

fn prepare_mp4_state(args: &Args) -> Result<Option<(Mp4SampleReader, VideoCodecType)>> {
    if let Some(ref mp4_path) = args.input_mp4 {
        let reader = Mp4SampleReader::new(mp4_path)?;
        let codec_type = reader.codec_type();
        Ok(Some((reader, codec_type)))
    } else {
        Ok(None)
    }
}

fn apply_video_options(mut builder: SoraClientBuilder, args: &Args) -> SoraClientBuilder {
    let video_bit_rate = args.video_bit_rate;
    if let Some(video) = args.video {
        if video {
            let video_setting = match args.video_codec_type.as_deref() {
                Some("vp8") => sora_sdk::Video::new_vp8(video_bit_rate),
                Some("vp9") => sora_sdk::Video::new_vp9(video_bit_rate, None),
                Some("av1") => sora_sdk::Video::new_av1(video_bit_rate, None),
                Some("h264") => sora_sdk::Video::new_h264(video_bit_rate, None),
                Some("h265") => sora_sdk::Video::new_h265(video_bit_rate, None),
                None => sora_sdk::Video::new_bool(true),
                _ => sora_sdk::Video::new_bool(true),
            };
            builder = builder.video(video_setting);
        } else {
            builder = builder.video(sora_sdk::Video::new_bool(false));
        }
    } else if let Some(ref codec) = args.video_codec_type {
        let video_setting = match codec.as_str() {
            "vp8" => sora_sdk::Video::new_vp8(video_bit_rate),
            "vp9" => sora_sdk::Video::new_vp9(video_bit_rate, None),
            "av1" => sora_sdk::Video::new_av1(video_bit_rate, None),
            "h264" => sora_sdk::Video::new_h264(video_bit_rate, None),
            "h265" => sora_sdk::Video::new_h265(video_bit_rate, None),
            _ => sora_sdk::Video::new_bool(true),
        };
        builder = builder.video(video_setting);
    }
    builder
}

fn apply_common_builder_options(
    mut builder: SoraClientBuilder,
    args: &Args,
) -> Result<SoraClientBuilder> {
    if let Some(audio) = args.audio {
        builder = builder.audio(sora_sdk::Audio::new_bool(audio));
    }
    builder = apply_video_options(builder, args);
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

fn build_client_builder<T>(
    context: Arc<SoraClientContext>,
    args: &Args,
    event_tx: T,
) -> Result<SoraClientBuilder>
where
    T: AppEventSender,
{
    let builder = SoraClient::builder(
        context,
        args.signaling_urls.clone(),
        args.channel_id.clone(),
        args.role,
    )
    .on_notify({
        let event_tx = event_tx.clone();
        move |text| {
            event_tx.send_event(AppEvent::Notify(text.to_string()));
        }
    })
    .on_push({
        let event_tx = event_tx.clone();
        move |text| {
            event_tx.send_event(AppEvent::Push(text.to_string()));
        }
    })
    .on_track({
        let event_tx = event_tx.clone();
        move |transceiver| {
            event_tx.send_event(AppEvent::OnTrack(transceiver));
        }
    })
    .on_remove_track({
        move |receiver| {
            event_tx.send_event(AppEvent::OnRemoveTrack(receiver));
        }
    });
    apply_common_builder_options(builder, args)
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
                .height(480);
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
    mut builder: SoraClientBuilder,
    context: &Arc<SoraClientContext>,
    args: &Args,
    mp4_reader: Option<Mp4SampleReader>,
) -> Result<(SoraClientBuilder, Option<VideoCapturerHolder>)> {
    let video_enabled = args.video.unwrap_or(true);
    let mut video_capturer = None;
    if args.role.wants_send() && video_enabled {
        let mut capturer = create_video_capturer(args, mp4_reader)?;
        capturer.start()?;
        let video_track = context.create_video_track(&capturer.video_source())?;
        builder = builder.sender_video_track(video_track);
        video_capturer = Some(capturer);
    }

    if args.role.wants_send() {
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
        rtc_log_warning!("ビデオ以外のトラックを受信しました: kind={}", kind);
        return;
    }
    let track_id = match track.id() {
        Ok(id) => id,
        Err(_) => {
            rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
            return;
        }
    };

    let mut video_track = track.cast_to_video_track();
    if let Some(old_entry) = tracks.remove(&track_id) {
        if !replace_existing {
            tracks.insert(track_id, old_entry);
            return;
        }
        rtc_log_info!("既存のトラックを削除します: track_id={}", track_id);
        video_track.remove_sink(&old_entry.sink);
    }

    rtc_log_info!("ビデオ トラックが追加されました: track_id={}", track_id);
    let sink = build_sink(track_id.clone());
    let wants = VideoSinkWants::new();
    video_track.add_or_update_sink(&sink, &wants);
    tracks.insert(track_id, TrackEntry { sink });
}

fn handle_on_remove_track_event(
    tracks: &mut HashMap<String, TrackEntry>,
    receiver: shiguredo_webrtc::RtpReceiver,
) {
    let track = receiver.track();
    let track_id = match track.id() {
        Ok(id) => id,
        Err(_) => {
            rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
            return;
        }
    };
    let kind = match track.kind() {
        Ok(kind) => kind,
        Err(_) => "unknown".to_string(),
    };
    if kind != "video" {
        rtc_log_warning!("ビデオ以外のトラックが削除されました: kind={}", kind);
        return;
    }
    let mut video_track = track.cast_to_video_track();
    if let Some(entry) = tracks.remove(&track_id) {
        video_track.remove_sink(&entry.sink);
    }
    rtc_log_info!("ビデオ トラックが削除されました: track_id={}", track_id);
}

#[cfg(feature = "raw-player")]
fn run_with_raw_player(args: Args) -> Result<()> {
    log::log_to_debug(log::Severity::Warning);
    log::enable_timestamps();
    log::enable_threads();

    let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel::<I420Frame>(2);
    let (event_tx, event_rx) = std::sync::mpsc::channel::<AppEvent>();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    let stop_for_renderer = stop.clone();

    let handle = thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Tokio ランタイムの作成に失敗しました");

        let _ = rt.block_on(async move {
            let mp4_state = prepare_mp4_state(&args)?;

            #[cfg(feature = "media-device")]
            let external_adm = if args.audio_input_device.is_some() {
                Some(SumomoAdm::new())
            } else {
                None
            };
            #[cfg(feature = "media-device")]
            let adm_config = if external_adm.is_some() {
                sora_sdk::AdmConfig::UseExternal(external_adm.as_ref().unwrap().audio_device_module())
            } else {
                sora_sdk::AdmConfig::NoAudioDevice
            };
            #[cfg(not(feature = "media-device"))]
            let adm_config = sora_sdk::AdmConfig::NoAudioDevice;

            let context_config = build_context_config(
                adm_config,
                mp4_state.as_ref().map(|(_, codec_type)| *codec_type),
                args.openh264_path.as_deref(),
                args.video_codec_implementation.clone(),
            )?;
            let context = SoraClientContext::new_with_config(context_config)?;

            #[cfg(feature = "media-device")]
            let mut _audio_capturer = if let Some(ref device_id) = args.audio_input_device {
                let state = external_adm
                    .as_ref()
                    .expect("BUG: external_adm が None です")
                    .state();
                let mut capturer = AudioDeviceCapturer::new(Some(device_id.clone()), state)?;
                capturer.start()?;
                rtc_log_info!("オーディオ入力デバイスを開始しました: {}", device_id);
                Some(capturer)
            } else {
                None
            };

            let builder = build_client_builder(context.clone(), &args, event_tx.clone())?;
            let (builder, mut _video_capturer) =
                attach_sender_tracks(builder, &context, &args, mp4_state.map(|(reader, _)| reader))?;

            let (client, _handle) = builder.build()?;
            let mut tracks: HashMap<String, TrackEntry> = HashMap::new();
            let mut run = Box::pin(client.run());
            let duration_sleep = args.duration.map(|secs| {
                rtc_log_info!("{} 秒後に切断します", secs);
                tokio::time::sleep(std::time::Duration::from_secs(secs))
            });
            tokio::pin!(duration_sleep);

            loop {
                if stop_for_thread.load(Ordering::Relaxed) {
                    break;
                }

                tokio::select! {
                    result = &mut run => {
                        stop_for_thread.store(true, Ordering::Relaxed);
                        return result.map_err(AppError::Sora);
                    }
                    _ = async { duration_sleep.as_mut().as_pin_mut().unwrap().await }, if duration_sleep.is_some() => {
                        rtc_log_info!("指定された時間が経過しました。切断します");
                        stop_for_thread.store(true, Ordering::Relaxed);
                        break;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                        while let Ok(event) = event_rx.try_recv() {
                            match event {
                                AppEvent::Notify(text) => {
                                    rtc_log_info!("notify を受信しました: {}", text);
                                }
                                AppEvent::Push(text) => {
                                    rtc_log_info!("push を受信しました: {}", text);
                                }
                                AppEvent::OnTrack(transceiver) => {
                                    handle_on_track_event(&mut tracks, transceiver, true, |track_id| {
                                        let frame_tx = frame_tx.clone();
                                        let first_frame = Arc::new(AtomicBool::new(false));
                                        VideoSink::new_with_handler(Box::new(RawPlayerTrackSinkHandler {
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
                    }
                }
            }
            Ok(())
        });
    });

    let mut raw_player_renderer = RawPlayerRenderer::new("Sumomo - Video", 640, 480)?;

    let mut frame_count = 0u64;
    while raw_player_renderer.is_running() && !stop_for_renderer.load(Ordering::Relaxed) {
        raw_player_renderer.poll_events();

        while let Ok(frame) = frame_rx.try_recv() {
            frame_count += 1;
            if frame_count == 1 {
                rtc_log_info!(
                    "raw_player: 最初のフレームを受信しました: {}x{}",
                    frame.width,
                    frame.height
                );
            }
            raw_player_renderer.render(&frame);
        }

        thread::sleep(std::time::Duration::from_millis(1));
    }

    stop.store(true, Ordering::Relaxed);
    let _ = handle.join();
    raw_player::quit();
    Ok(())
}

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

    #[cfg(feature = "raw-player")]
    if args.use_raw_player {
        return run_with_raw_player(args);
    }

    let renderer = Arc::new(AnsiRenderer::new());
    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(32);

    // --audio-input-device が指定された場合は SumomoAdm を使用する
    #[cfg(feature = "media-device")]
    let external_adm = if args.audio_input_device.is_some() {
        Some(SumomoAdm::new())
    } else {
        None
    };

    // --input-mp4 が指定されている場合は MP4 を読み込んでパススルーの準備をする
    let mp4_state = prepare_mp4_state(&args)?;

    #[cfg(feature = "media-device")]
    let adm_config = if external_adm.is_some() {
        sora_sdk::AdmConfig::UseExternal(external_adm.as_ref().unwrap().audio_device_module())
    } else {
        sora_sdk::AdmConfig::NoAudioDevice
    };
    #[cfg(not(feature = "media-device"))]
    let adm_config = sora_sdk::AdmConfig::NoAudioDevice;

    let context_config = build_context_config(
        adm_config,
        mp4_state.as_ref().map(|(_, codec_type)| *codec_type),
        args.openh264_path.as_deref(),
        args.video_codec_implementation.clone(),
    )?;
    let context = SoraClientContext::new_with_config(context_config)?;

    // --audio-input-device が指定された場合は AudioDeviceCapturer を使用する
    #[cfg(feature = "media-device")]
    let mut _audio_capturer = if let Some(ref device_id) = args.audio_input_device {
        let state = external_adm
            .as_ref()
            .expect("BUG: external_adm が None です")
            .state();
        let mut capturer = AudioDeviceCapturer::new(Some(device_id.clone()), state)?;
        capturer.start()?;
        rtc_log_info!("オーディオ入力デバイスを開始しました: {}", device_id);
        Some(capturer)
    } else {
        None
    };

    let builder = build_client_builder(context.clone(), &args, event_tx.clone())?;
    let (builder, mut _video_capturer) = attach_sender_tracks(
        builder,
        &context,
        &args,
        mp4_state.map(|(reader, _)| reader),
    )?;

    let (client, _handle) = builder.build()?;
    let renderer_for_events = renderer.clone();
    let mut tracks: HashMap<String, TrackEntry> = HashMap::new();
    let mut run = Box::pin(client.run());

    // duration が指定されている場合はタイマーを設定
    let duration_sleep = args.duration.map(|secs| {
        rtc_log_info!("{} 秒後に切断します", secs);
        tokio::time::sleep(std::time::Duration::from_secs(secs))
    });
    tokio::pin!(duration_sleep);

    loop {
        tokio::select! {
            result = &mut run => {
                return result.map_err(AppError::Sora);
            }
            _ = async { duration_sleep.as_mut().as_pin_mut().unwrap().await }, if duration_sleep.is_some() => {
                rtc_log_info!("指定された時間が経過しました。切断します");
                break;
            }
            event = event_rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                match event {
                    AppEvent::Notify(text) => {
                        rtc_log_info!("notify を受信しました: {}", text);
                    }
                    AppEvent::Push(text) => {
                        rtc_log_info!("push を受信しました: {}", text);
                    }
                    AppEvent::OnTrack(transceiver) => {
                        handle_on_track_event(&mut tracks, transceiver, false, |track_id| {
                            let first_frame = Arc::new(AtomicBool::new(false));
                            VideoSink::new_with_handler(Box::new(AnsiTrackSinkHandler {
                                renderer: renderer_for_events.clone(),
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
        }
    }
    run.await?;
    Ok(())
}
