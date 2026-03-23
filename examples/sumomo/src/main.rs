use rustls_pki_types::pem::PemObject;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::io;
use std::io::Write as IoWrite;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

#[cfg(feature = "media-device")]
use shiguredo_webrtc::VideoFrame as WebrtcVideoFrame;
use shiguredo_webrtc::{
    AdaptFrameResult, AdaptedVideoTrackSource, CodecSpecificInfo, EncodedImage, EncodedImageBuffer,
    H264PacketizationMode, I420Buffer, LibyuvFourcc, SdpVideoFormat, TimestampAligner,
    VideoCodecRef, VideoCodecStatus, VideoCodecType as WebrtcVideoCodecType,
    VideoEncoderEncodedImageCallbackPtr, VideoEncoderEncodedImageCallbackRef,
    VideoEncoderEncodedImageCallbackResultError, VideoEncoderEncoderInfo, VideoEncoderHandler,
    VideoEncoderRateControlParametersRef, VideoEncoderSettingsRef, VideoFrame, VideoFrameRef,
    VideoFrameType, VideoFrameTypeVectorRef, VideoSink, VideoSinkHandler, VideoSinkWants,
    VideoTrackSource, convert_from_i420, log, rtc_log_info, rtc_log_warning,
};
use sora_sdk::{
    CodecDirection, Role, SoraClient, SoraClientContext, SoraClientContextConfig,
    VideoCodecImplementation,
};
#[cfg(feature = "nvcodec")]
use sora_sdk::{NvCodecVideoCodecCapability, VideoCodecCapability, VideoCodecPreference};
#[cfg(not(feature = "nvcodec"))]
use sora_sdk::{VideoCodecCapability, VideoCodecPreference};
#[cfg(feature = "media-device")]
use std::sync::Mutex;
use std::sync::mpsc as std_mpsc;
use tokio::sync::mpsc;

struct Args {
    signaling_urls: Vec<String>,
    channel_id: String,
    role: Role,
    audio: Option<bool>,
    video: Option<bool>,
    video_codec_type: Option<String>,
    input_mp4: Option<String>,
    data_channel_signaling: Option<bool>,
    ignore_disconnect_websocket: Option<bool>,
    simulcast: Option<bool>,
    insecure: bool,
    client_cert: Option<String>,
    client_key: Option<String>,
    ca_cert: Option<String>,
    duration: Option<u64>,
    turn_tls_insecure: bool,
    turn_tls_ca_cert: Option<String>,
    #[cfg(feature = "raw-player")]
    use_raw_player: bool,
    #[cfg(feature = "media-device")]
    video_input_device: Option<String>,
    #[cfg(feature = "media-device")]
    audio_input_device: Option<String>,
    #[cfg(feature = "media-device")]
    list_devices: bool,
}

enum AppEvent {
    Notify(String),
    Push(String),
    OnTrack(shiguredo_webrtc::RtpTransceiver),
    OnRemoveTrack(shiguredo_webrtc::RtpReceiver),
}

#[derive(Debug, Clone)]
struct ErrorMessage {
    message: String,
}

impl ErrorMessage {
    #[allow(dead_code)]
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug)]
enum AppError {
    Args(noargs::Error),
    Sora(sora_sdk::Error),
    #[cfg(feature = "media-device")]
    Video(shiguredo_video_device::Error),
    #[cfg(feature = "media-device")]
    Audio(shiguredo_audio_device::Error),
    #[cfg(feature = "raw-player")]
    RawPlayer(raw_player::Error),
    Message(ErrorMessage),
    Io(std::io::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Args(err) => write!(f, "{err:?}"),
            AppError::Sora(err) => write!(f, "AppError::Sora: {err}"),
            AppError::Message(err) => write!(f, "AppError::Message: {err}"),
            #[cfg(feature = "media-device")]
            AppError::Video(err) => write!(f, "AppError::Video: {err}"),
            #[cfg(feature = "media-device")]
            AppError::Audio(err) => write!(f, "AppError::Audio: {err}"),
            #[cfg(feature = "raw-player")]
            AppError::RawPlayer(err) => write!(f, "AppError::RawPlayer: {err}"),
            AppError::Io(err) => write!(f, "AppError::Io: {err}"),
        }
    }
}

impl std::fmt::Display for ErrorMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Sora(err) => Some(err),
            _ => None,
        }
    }
}

impl From<noargs::Error> for AppError {
    fn from(err: noargs::Error) -> Self {
        AppError::Args(err)
    }
}

impl From<sora_sdk::Error> for AppError {
    fn from(err: sora_sdk::Error) -> Self {
        AppError::Sora(err)
    }
}

impl From<ErrorMessage> for AppError {
    fn from(err: ErrorMessage) -> Self {
        AppError::Message(err)
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err)
    }
}

#[cfg(feature = "raw-player")]
impl From<raw_player::Error> for AppError {
    fn from(err: raw_player::Error) -> Self {
        AppError::RawPlayer(err)
    }
}

#[cfg(feature = "media-device")]
impl From<shiguredo_video_device::Error> for AppError {
    fn from(err: shiguredo_video_device::Error) -> Self {
        AppError::Video(err)
    }
}

#[cfg(feature = "media-device")]
impl From<shiguredo_audio_device::Error> for AppError {
    fn from(err: shiguredo_audio_device::Error) -> Self {
        AppError::Audio(err)
    }
}

type Result<T> = std::result::Result<T, AppError>;

fn parse_args() -> Result<Args> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = "Sora WebSocket シグナリングの最小サンプル";

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        rtc_log_info!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    noargs::HELP_FLAG.take_help(&mut args);

    // --list-devices は他のオプションなしで使用できる
    #[cfg(feature = "media-device")]
    let list_devices = noargs::flag("list-devices")
        .doc("利用可能なデバイス一覧を表示して終了する")
        .take(&mut args)
        .is_present();

    #[cfg(feature = "media-device")]
    if list_devices {
        // list-devices モードでは他のオプションは不要
        let _ = args.finish();
        return Ok(Args {
            signaling_urls: Vec::new(),
            channel_id: String::new(),
            role: Role::RecvOnly,
            audio: None,
            video: None,
            video_codec_type: None,
            data_channel_signaling: None,
            ignore_disconnect_websocket: None,
            simulcast: None,
            insecure: false,
            client_cert: None,
            client_key: None,
            ca_cert: None,
            duration: None,
            turn_tls_insecure: false,
            turn_tls_ca_cert: None,
            input_mp4: None,
            #[cfg(feature = "raw-player")]
            use_raw_player: false,
            video_input_device: None,
            audio_input_device: None,
            list_devices: true,
        });
    }

    let signaling_urls: Vec<String> = noargs::opt("signaling-url")
        .doc("Sora の WebSocket シグナリング URL (カンマ区切りで複数指定可)")
        .example("wss://sora.example.com/signaling")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().split(',').map(|s| s.trim().to_string()).collect()))?;

    let channel_id: String = noargs::opt("channel-id")
        .doc("Sora のチャネル ID")
        .example("sora")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let role: String = noargs::opt("role")
        .doc("Sora のロール (sendonly, recvonly, sendrecv)")
        .example("recvonly")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let audio: Option<bool> = noargs::opt("audio")
        .doc("音声の有効/無効 (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("audio は true または false で指定してください"),
        })?;

    let video: Option<bool> = noargs::opt("video")
        .doc("映像の有効/無効 (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("video は true または false で指定してください"),
        })?;

    let video_codec_type: Option<String> = noargs::opt("video-codec-type")
        .doc("映像コーデック (vp8/vp9/av1/h264/h265)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "vp8" | "vp9" | "av1" | "h264" | "h265" => Ok(o.value().to_string()),
            _ => Err("video-codec-type は vp8/vp9/av1/h264/h265 で指定してください"),
        })?;

    let input_mp4: Option<String> = noargs::opt("input-mp4")
        .doc("MP4 ファイルからエンコード済み映像をそのまま送信する")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let data_channel_signaling: Option<bool> = noargs::opt("data-channel-signaling")
        .doc("DataChannel 経由でシグナリングを行う (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("data-channel-signaling は true または false で指定してください"),
        })?;

    let ignore_disconnect_websocket: Option<bool> = noargs::opt("ignore-disconnect-websocket")
        .doc("DataChannel 使用時に WebSocket 切断を無視する (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("ignore-disconnect-websocket は true または false で指定してください"),
        })?;

    let simulcast: Option<bool> = noargs::opt("simulcast")
        .doc("サイマルキャストを有効にする (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("simulcast は true または false で指定してください"),
        })?;

    let insecure = noargs::flag("insecure")
        .doc("サーバー証明書の検証をスキップする")
        .take(&mut args)
        .is_present();

    let client_cert: Option<String> = noargs::opt("client-cert")
        .doc("クライアント証明書の PEM ファイルパス")
        .take(&mut args)
        .present_and_then(|o| {
            std::fs::read_to_string(o.value())
                .map_err(|e| format!("クライアント証明書の読み込みに失敗しました: {e}"))
        })?;

    let client_key: Option<String> = noargs::opt("client-key")
        .doc("クライアント秘密鍵の PEM ファイルパス")
        .take(&mut args)
        .present_and_then(|o| {
            std::fs::read_to_string(o.value())
                .map_err(|e| format!("クライアント秘密鍵の読み込みに失敗しました: {e}"))
        })?;

    let ca_cert: Option<String> = noargs::opt("ca-cert")
        .doc("CA 証明書の PEM ファイルパス")
        .take(&mut args)
        .present_and_then(|o| {
            std::fs::read_to_string(o.value())
                .map_err(|e| format!("CA 証明書の読み込みに失敗しました: {e}"))
        })?;

    let duration: Option<u64> = noargs::opt("duration")
        .doc("接続を維持する秒数 (省略時は無制限)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<u64>())?;

    let turn_tls_insecure = noargs::flag("turn-tls-insecure")
        .doc("TURN-TLS の証明書検証をスキップする")
        .take(&mut args)
        .is_present();

    let turn_tls_ca_cert: Option<String> = noargs::opt("turn-tls-ca-cert")
        .doc("TURN-TLS の CA 証明書ファイル (PEM 形式)")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    #[cfg(feature = "raw-player")]
    let use_raw_player = noargs::flag("raw-player")
        .doc("raw-player でビデオを表示する")
        .take(&mut args)
        .is_present();

    #[cfg(feature = "media-device")]
    let video_input_device: Option<String> = noargs::opt("video-input-device")
        .doc("使用するビデオ入力デバイスの ID（省略時は FakeVideoCapturer を使用）")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    #[cfg(feature = "media-device")]
    let audio_input_device: Option<String> = noargs::opt("audio-input-device")
        .doc("使用するオーディオ入力デバイスの名前または ID")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    if let Some(help) = args.finish()? {
        print!("{}", help);
        std::process::exit(0);
    }

    let role = Role::parse(&role)?;

    Ok(Args {
        signaling_urls,
        channel_id,
        role,
        audio,
        video,
        video_codec_type,
        input_mp4,
        data_channel_signaling,
        ignore_disconnect_websocket,
        simulcast,
        insecure,
        client_cert,
        client_key,
        ca_cert,
        duration,
        turn_tls_insecure,
        turn_tls_ca_cert,
        #[cfg(feature = "raw-player")]
        use_raw_player,
        #[cfg(feature = "media-device")]
        video_input_device,
        #[cfg(feature = "media-device")]
        audio_input_device,
        #[cfg(feature = "media-device")]
        list_devices: false,
    })
}

/// ANSI 描画用の簡易レンダラー。
struct AnsiRenderer {
    width: i32,
    height: i32,
}

impl AnsiRenderer {
    fn new() -> Self {
        Self {
            width: 80,
            height: 45,
        }
    }

    fn render(&self, frame: VideoFrameRef) {
        render_frame(frame, self.width, self.height);
    }
}

#[cfg(feature = "raw-player")]
struct I420Frame {
    width: i32,
    height: i32,
    y_data: Vec<u8>,
    y_stride: i32,
    u_data: Vec<u8>,
    u_stride: i32,
    v_data: Vec<u8>,
    v_stride: i32,
}

#[cfg(feature = "raw-player")]
impl I420Frame {
    fn from_buffer(buffer: &I420Buffer) -> Self {
        Self {
            width: buffer.width(),
            height: buffer.height(),
            y_data: buffer.y_data().to_vec(),
            y_stride: buffer.stride_y(),
            u_data: buffer.u_data().to_vec(),
            u_stride: buffer.stride_u(),
            v_data: buffer.v_data().to_vec(),
            v_stride: buffer.stride_v(),
        }
    }
}

#[cfg(feature = "raw-player")]
struct RawPlayerRenderer {
    window: raw_player::Window,
    renderer: raw_player::Renderer,
    texture: Option<raw_player::Texture>,
    running: bool,
}

#[cfg(feature = "raw-player")]
impl RawPlayerRenderer {
    fn new(title: &str, width: i32, height: i32) -> Result<Self> {
        raw_player::init()?;
        let window = raw_player::Window::new(title, width, height)?;
        let renderer = raw_player::Renderer::new(&window)?;
        Ok(Self {
            window,
            renderer,
            texture: None,
            running: true,
        })
    }

    fn render(&mut self, frame: &I420Frame) {
        let width = frame.width;
        let height = frame.height;

        let needs_recreate = self
            .texture
            .as_ref()
            .map_or(true, |t| t.width() != width || t.height() != height);
        if needs_recreate {
            let _ = self.window.set_size(width, height);
            self.texture = raw_player::Texture::new_yuv(&self.renderer, width, height).ok();
        }

        if let Some(ref mut texture) = self.texture {
            let _ = texture.update_yuv(
                &frame.y_data,
                frame.y_stride,
                &frame.u_data,
                frame.u_stride,
                &frame.v_data,
                frame.v_stride,
            );

            let _ = self.renderer.set_draw_color(0, 0, 0, 255);
            let _ = self.renderer.clear();
            let _ = self.renderer.copy(texture);
            let _ = self.renderer.present();
        }
    }

    fn poll_events(&mut self) {
        while let Some(event) = raw_player::poll_event() {
            match event {
                raw_player::Event::Quit | raw_player::Event::WindowClose => {
                    self.running = false;
                }
                raw_player::Event::KeyDown { keycode } if keycode == raw_player::KEYCODE_ESCAPE => {
                    self.running = false;
                }
                _ => {}
            }
        }
    }

    fn is_running(&self) -> bool {
        self.running
    }
}

struct TrackEntry {
    sink: VideoSink,
}

#[cfg(feature = "raw-player")]
struct RawPlayerTrackSinkHandler {
    frame_tx: std::sync::mpsc::SyncSender<I420Frame>,
    first_frame: Arc<AtomicBool>,
    track_id_for_log: String,
}

#[cfg(feature = "raw-player")]
impl VideoSinkHandler for RawPlayerTrackSinkHandler {
    fn on_frame(&mut self, frame: VideoFrameRef<'_>) {
        if !self.first_frame.swap(true, Ordering::Relaxed) {
            rtc_log_info!(
                "ビデオ フレームを受信しました: track_id={}",
                self.track_id_for_log
            );
        }
        let buffer = frame.buffer();
        let i420_frame = I420Frame::from_buffer(&buffer);
        let _ = self.frame_tx.try_send(i420_frame);
    }
}

struct AnsiTrackSinkHandler {
    renderer: Arc<AnsiRenderer>,
    first_frame: Arc<AtomicBool>,
    track_id_for_log: String,
}

impl VideoSinkHandler for AnsiTrackSinkHandler {
    fn on_frame(&mut self, frame: VideoFrameRef<'_>) {
        if !self.first_frame.swap(true, Ordering::Relaxed) {
            rtc_log_info!(
                "ビデオ フレームを受信しました: track_id={}",
                self.track_id_for_log
            );
        }
        self.renderer.render(frame);
    }
}

fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> i32 {
    let r6 = (r as i32 * 5) / 255;
    let g6 = (g as i32 * 5) / 255;
    let b6 = (b as i32 * 5) / 255;
    16 + (r6 * 36) + (g6 * 6) + b6
}

fn render_frame(frame: VideoFrameRef, width: i32, height: i32) {
    let src = frame.buffer();
    let mut scaled = I420Buffer::new(width, height);
    scaled.scale_from(&src);

    let image = match convert_from_i420(&scaled, LibyuvFourcc::Argb) {
        Some(image) => image,
        None => return,
    };
    let width_u = width.max(0) as usize;
    let height_u = height.max(0) as usize;
    let capacity = width_u.saturating_mul(height_u).saturating_mul(20);
    let mut output = String::with_capacity(capacity);
    output.push_str("\x1b[H");

    // 2x1 ピクセルを 1 文字で表現する。
    for y in (0..height_u).step_by(2) {
        output.push_str("\x1b[2K");
        for x in 0..width_u {
            let upper_offset = (y * width_u + x) * 4;
            let upper_r = image[upper_offset + 2];
            let upper_g = image[upper_offset + 1];
            let upper_b = image[upper_offset];

            let (lower_r, lower_g, lower_b) = if y + 1 < height_u {
                let lower_offset = ((y + 1) * width_u + x) * 4;
                let lower_r = image[lower_offset + 2];
                let lower_g = image[lower_offset + 1];
                let lower_b = image[lower_offset];
                (lower_r, lower_g, lower_b)
            } else {
                (upper_r, upper_g, upper_b)
            };
            let upper_color = rgb_to_ansi256(upper_r, upper_g, upper_b);
            let lower_color = rgb_to_ansi256(lower_r, lower_g, lower_b);
            let _ = write!(
                output,
                "\x1b[38;5;{}m\x1b[48;5;{}m▀",
                upper_color, lower_color
            );
        }
        output.push_str("\x1b[0m\n");
    }

    let mut stdout = io::stdout();
    let _ = stdout.write_all(output.as_bytes());
    let _ = stdout.flush();
}

/// u32 のスライスを読み取り専用の u8 スライスとして扱う。
fn u32_slice_as_u8_slice(data: &[u32]) -> &[u8] {
    let len = std::mem::size_of_val(data);
    let ptr = data.as_ptr() as *const u8;
    // 安全性: u32 の連続領域を読み取り専用の u8 スライスとして扱う。
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// on_tick は 1 秒ごとに、1 秒の先頭で生成されるフレームだけで呼ばれる。
#[derive(Clone)]
struct FakeVideoCapturerConfig {
    width: i32,
    height: i32,
    fps: i32,
}

impl Default for FakeVideoCapturerConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 480,
            fps: 30,
        }
    }
}

struct FakeVideoCapturer {
    source: AdaptedVideoTrackSource,
    timestamp_aligner: Option<shiguredo_webrtc::TimestampAligner>,
    image: Vec<u32>,
    width: i32,
    height: i32,
    fps: i32,
    start_time_ms: i64,
    video_source: VideoTrackSource,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FakeVideoCapturer {
    fn new(config: FakeVideoCapturerConfig) -> Result<Self> {
        let width = if config.width > 0 { config.width } else { 640 };
        let height = if config.height > 0 {
            config.height
        } else {
            480
        };
        let fps = if config.fps > 0 { config.fps } else { 30 };
        let source = AdaptedVideoTrackSource::new();
        let timestamp_aligner = shiguredo_webrtc::TimestampAligner::new();
        let video_source = source.cast_to_video_track_source();
        Ok(Self {
            image: vec![0u32; (width * height) as usize],
            width,
            height,
            fps,
            start_time_ms: shiguredo_webrtc::time_millis(),
            video_source,
            source,
            timestamp_aligner: Some(timestamp_aligner),
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
        })
    }

    fn video_source(&self) -> VideoTrackSource {
        self.video_source.clone()
    }

    fn start(&mut self) -> Result<()> {
        if self.handle.is_some() {
            return Ok(());
        }
        let mut source = self.source.clone();
        let mut timestamp_aligner = match self.timestamp_aligner.take() {
            Some(t) => t,
            None => return Ok(()),
        };
        let mut image = std::mem::take(&mut self.image);
        let width = self.width;
        let height = self.height;
        let fps = self.fps.max(1);
        let start_time_ms = self.start_time_ms;
        let stop = self.stop.clone();
        let handle = thread::Builder::new()
            .name("fake-video-capturer".to_string())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    tick_once(
                        &mut source,
                        &mut timestamp_aligner,
                        &mut image,
                        width,
                        height,
                        start_time_ms,
                        fps,
                    );
                    let sleep_ms = (1000 / fps).saturating_sub(2).max(1);
                    shiguredo_webrtc::thread_sleep_ms(sleep_ms);
                }
            })?;
        self.handle = Some(handle);
        Ok(())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FakeVideoCapturer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn tick_once(
    source: &mut AdaptedVideoTrackSource,
    timestamp_aligner: &mut shiguredo_webrtc::TimestampAligner,
    image: &mut [u32],
    width: i32,
    height: i32,
    start_time_ms: i64,
    _fps: i32,
) {
    let elapsed_ms = shiguredo_webrtc::time_millis() - start_time_ms;
    let radius = (width.min(height)) / 4;
    let center_x = width / 2;
    let center_y = height / 2;
    let angle = 2.0 * std::f64::consts::PI * (elapsed_ms % 1000) as f64 / 1000.0;
    let circle_x = center_x + (radius as f64 * angle.cos()) as i32;
    let circle_y = center_y + (radius as f64 * angle.sin()) as i32;
    let circle_radius = 100;

    image.fill(0);
    for y in -circle_radius..=circle_radius {
        for x in -circle_radius..=circle_radius {
            if x * x + y * y <= circle_radius * circle_radius {
                let draw_x = circle_x + x;
                let draw_y = circle_y + y;
                if draw_x >= 0 && draw_x < width && draw_y >= 0 && draw_y < height {
                    let mut color = 0xFF00_0000u32;
                    color |= (((elapsed_ms / 10) % 256) as u32) << 16;
                    color |= (((elapsed_ms / 5) % 256) as u32) << 8;
                    color |= (elapsed_ms % 256) as u32;
                    image[(draw_y * width + draw_x) as usize] = color;
                }
            }
        }
    }

    if let Some(buffer) =
        shiguredo_webrtc::abgr_to_i420(u32_slice_as_u8_slice(image), width, height)
    {
        let timestamp_us = elapsed_ms * 1000;
        let frame = shiguredo_webrtc::VideoFrame::from_i420(&buffer, timestamp_us, 0);
        let AdaptFrameResult { applied, size } = source.adapt_frame(width, height, timestamp_us);
        let frame = if applied
            && (size.adapted_width != frame.width() || size.adapted_height != frame.height())
        {
            let mut scaled =
                shiguredo_webrtc::I420Buffer::new(size.adapted_width, size.adapted_height);
            scaled.scale_from(&buffer);
            shiguredo_webrtc::VideoFrame::from_i420(
                &scaled,
                timestamp_aligner.translate(timestamp_us, shiguredo_webrtc::time_millis() * 1000),
                0,
            )
        } else {
            shiguredo_webrtc::VideoFrame::from_i420(
                &buffer,
                timestamp_aligner.translate(timestamp_us, shiguredo_webrtc::time_millis() * 1000),
                0,
            )
        };
        source.on_frame(&frame);
    }
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

    let signaling_urls = args.signaling_urls.clone();
    let channel_id = args.channel_id.clone();
    let role = args.role;
    let data_channel_signaling = args.data_channel_signaling;
    let simulcast = args.simulcast;
    let insecure = args.insecure;
    let client_cert = args.client_cert.clone();
    let client_key = args.client_key.clone();
    let ca_cert = args.ca_cert.clone();

    let handle = thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Tokio ランタイムの作成に失敗しました");

        let _ = rt.block_on(async {
            let context = SoraClientContext::new()?;
            let mut builder =
                SoraClient::builder(context.clone(), signaling_urls, channel_id, role)
                .on_notify({
                    let event_tx = event_tx.clone();
                    move |text| {
                        let _ = event_tx.send(AppEvent::Notify(text.to_string()));
                    }
                })
                .on_push({
                    let event_tx = event_tx.clone();
                    move |text| {
                        let _ = event_tx.send(AppEvent::Push(text.to_string()));
                    }
                })
                .on_track({
                    let event_tx = event_tx.clone();
                    move |transceiver| {
                        rtc_log_info!("on_track コールバックが呼ばれました");
                        let _ = event_tx.send(AppEvent::OnTrack(transceiver));
                    }
                })
                .on_remove_track({
                    let event_tx = event_tx.clone();
                    move |receiver| {
                        let _ = event_tx.send(AppEvent::OnRemoveTrack(receiver));
                    }
                });

            let mut _video_capturer: Option<VideoCapturerHolder> = None;
            if role.wants_send() {
                let mut capturer = VideoCapturerHolder::Fake(
                    FakeVideoCapturer::new(FakeVideoCapturerConfig::default())?
                );
                capturer.start()?;
                let video_track = context.create_video_track(&capturer.video_source())?;
                builder = builder.sender_video_track(video_track);
                _video_capturer = Some(capturer);
            }
            if role.wants_send() {
                let audio_source = context.create_audio_source()?;
                let audio_track = context.create_audio_track(&audio_source)?;
                builder = builder.sender_audio_track(audio_track);
            }

            if let Some(data_channel_signaling) = data_channel_signaling {
                builder = builder.data_channel_signaling(data_channel_signaling);
            }
            if let Some(simulcast) = simulcast {
                builder = builder.simulcast(simulcast);
            }
            builder = builder.insecure(insecure);
            if let (Some(cert), Some(key)) = (client_cert, client_key) {
                builder = builder.client_cert(cert, key);
            }
            if let Some(ca) = ca_cert {
                builder = builder.ca_cert(ca);
            }

            let (client, _handle) = builder.build()?;
            let mut tracks: HashMap<String, TrackEntry> = HashMap::new();
            let mut run = Box::pin(client.run());

            loop {
                if stop_for_thread.load(Ordering::Relaxed) {
                    break;
                }

                tokio::select! {
                    result = &mut run => {
                        return result.map_err(AppError::Sora);
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
                                    let receiver = transceiver.receiver();
                                    let track = receiver.track();
                                    let kind = match track.kind() {
                                        Ok(kind) => kind,
                                        Err(_) => "unknown".to_string(),
                                    };
                                    if kind != "video" {
                                        rtc_log_warning!("ビデオ以外のトラックを受信しました: kind={}", kind);
                                        continue;
                                    }
                                    let track_id = match track.id() {
                                        Ok(id) => id,
                                        Err(_) => {
                                            rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
                                            continue;
                                        }
                                    };
                                    let mut video_track = track.cast_to_video_track();
                                    if let Some(old_entry) = tracks.remove(&track_id) {
                                        rtc_log_info!("既存のトラックを削除します: track_id={}", track_id);
                                        video_track.remove_sink(&old_entry.sink);
                                    }
                                    rtc_log_info!("ビデオ トラックが追加されました: track_id={}", track_id);
                                    let frame_tx = frame_tx.clone();
                                    let first_frame = Arc::new(AtomicBool::new(false));
                                    let sink = VideoSink::new_with_handler(Box::new(
                                        RawPlayerTrackSinkHandler {
                                            frame_tx,
                                            first_frame,
                                            track_id_for_log: track_id.clone(),
                                        },
                                    ));
                                    let wants = VideoSinkWants::new();
                                    video_track.add_or_update_sink(&sink, &wants);
                                    tracks.insert(track_id, TrackEntry { sink });
                                }
                                AppEvent::OnRemoveTrack(receiver) => {
                                    let track = receiver.track();
                                    let track_id = match track.id() {
                                        Ok(id) => id,
                                        Err(_) => {
                                            rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
                                            continue;
                                        }
                                    };
                                    let kind = match track.kind() {
                                        Ok(kind) => kind,
                                        Err(_) => "unknown".to_string(),
                                    };
                                    if kind != "video" {
                                        rtc_log_warning!("ビデオ以外のトラックが削除されました: kind={}", kind);
                                        continue;
                                    }
                                    let mut video_track = track.cast_to_video_track();
                                    if let Some(entry) = tracks.remove(&track_id) {
                                        video_track.remove_sink(&entry.sink);
                                    }
                                    rtc_log_info!("ビデオ トラックが削除されました: track_id={}", track_id);
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
    while raw_player_renderer.is_running() {
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

        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    stop.store(true, Ordering::Relaxed);
    let _ = handle.join();
    raw_player::quit();
    Ok(())
}

#[cfg(feature = "media-device")]
fn list_devices() -> Result<()> {
    let video_device_list = shiguredo_video_device::VideoDeviceList::enumerate()?;
    let audio_device_list = shiguredo_audio_device::AudioDeviceList::enumerate_input()?;

    let json = nojson::json(|f| {
        f.set_indent_size(2);
        f.set_spacing(true);
        f.object(|f| {
            f.member(
                "video_devices",
                nojson::array(|f| {
                    for device in video_device_list.devices() {
                        let name = device.name().unwrap_or_else(|_| String::new());
                        let unique_id = device.unique_id().unwrap_or_else(|_| String::new());
                        let formats = device.formats();
                        f.element(nojson::object(|f| {
                            f.member("name", name.as_str())?;
                            f.member("unique_id", unique_id.as_str())?;
                            f.member(
                                "formats",
                                nojson::array(|f| {
                                    for format in &formats {
                                        f.element(nojson::object(|f| {
                                            f.member("width", format.width)?;
                                            f.member("height", format.height)?;
                                            f.member("min_fps", format.min_fps)?;
                                            f.member("max_fps", format.max_fps)?;
                                            f.member("pixel_format", format.pixel_format.name())
                                        }))?;
                                    }
                                    Ok(())
                                }),
                            )
                        }))?;
                    }
                    Ok(())
                }),
            )?;
            f.member(
                "audio_devices",
                nojson::array(|f| {
                    for device in audio_device_list.devices() {
                        let name = device.name().unwrap_or_else(|_| String::new());
                        let unique_id = device.unique_id().unwrap_or_else(|_| String::new());
                        let channels = device.channels();
                        let sample_rate = device.sample_rate();
                        f.element(nojson::object(|f| {
                            f.member("name", name.as_str())?;
                            f.member("unique_id", unique_id.as_str())?;
                            f.member("channels", channels)?;
                            f.member("sample_rate", sample_rate)
                        }))?;
                    }
                    Ok(())
                }),
            )
        })
    });

    rtc_log_info!("{}", json);
    Ok(())
}

// --- MP4 パススルー送信 ---

/// MP4 から抽出したエンコード済みビデオサンプル
struct EncodedSample {
    data: Vec<u8>,
    is_keyframe: bool,
    width: u32,
    height: u32,
    codec_type: WebrtcVideoCodecType,
}

/// MP4 のビデオトラック情報
struct Mp4VideoTrackInfo {
    codec_type: WebrtcVideoCodecType,
    width: u16,
    height: u16,
    timescale: u32,
    /// H.264 の場合の SPS/PPS (Annex B 形式)
    h264_parameter_sets: Option<Vec<u8>>,
}

/// MP4 ファイルからビデオサンプルを読み出す
struct Mp4SampleReader {
    file_data: Vec<u8>,
    track_info: Mp4VideoTrackInfo,
    /// (data_offset, data_size, keyframe, timestamp, duration) のリスト
    samples: Vec<(u64, usize, bool, u64, u32)>,
}

impl Mp4SampleReader {
    fn new(path: &str) -> std::result::Result<Self, String> {
        use shiguredo_mp4::demux::{Input, Mp4FileDemuxer};

        let file_data = std::fs::read(path)
            .map_err(|e| format!("MP4 ファイルの読み込みに失敗しました: {e}"))?;

        let mut demuxer = Mp4FileDemuxer::new();

        // ファイル全体を入力として渡す
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

        let tracks = demuxer
            .tracks()
            .map_err(|e| format!("MP4 トラック情報の取得に失敗しました: {e}"))?;

        // ビデオトラックを探す
        let video_track = tracks
            .iter()
            .find(|t| t.kind == shiguredo_mp4::TrackKind::Video)
            .ok_or("MP4 にビデオトラックが見つかりません")?;

        let video_track_id = video_track.track_id;
        let timescale = video_track.timescale.get();

        // サンプルを全て読み出してトラック情報を取得する
        let mut track_info: Option<Mp4VideoTrackInfo> = None;
        let mut samples = Vec::new();

        loop {
            match demuxer.next_sample() {
                Ok(Some(sample)) => {
                    if sample.track.track_id != video_track_id {
                        continue;
                    }

                    // 最初のサンプルからコーデック情報を取得する
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
                Ok(None) => break,
                Err(e) => return Err(format!("MP4 サンプルの読み出しに失敗しました: {e}")),
            }
        }

        let track_info = track_info.ok_or("MP4 にビデオサンプルが見つかりません")?;

        if samples.is_empty() {
            return Err("MP4 にビデオサンプルが見つかりません".to_string());
        }

        Ok(Self {
            file_data,
            track_info,
            samples,
        })
    }

    fn extract_track_info(
        entry: &shiguredo_mp4::boxes::SampleEntry,
        timescale: u32,
    ) -> std::result::Result<Mp4VideoTrackInfo, String> {
        use shiguredo_mp4::boxes::SampleEntry;

        match entry {
            SampleEntry::Avc1(avc1) => {
                let (width, height) = (avc1.visual.width, avc1.visual.height);
                // SPS/PPS を Annex B 形式で結合する
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
                    codec_type: WebrtcVideoCodecType::H264,
                    width,
                    height,
                    timescale,
                    h264_parameter_sets: Some(parameter_sets),
                })
            }
            SampleEntry::Vp08(vp08) => Ok(Mp4VideoTrackInfo {
                codec_type: WebrtcVideoCodecType::Vp8,
                width: vp08.visual.width,
                height: vp08.visual.height,
                timescale,
                h264_parameter_sets: None,
            }),
            SampleEntry::Vp09(vp09) => Ok(Mp4VideoTrackInfo {
                codec_type: WebrtcVideoCodecType::Vp9,
                width: vp09.visual.width,
                height: vp09.visual.height,
                timescale,
                h264_parameter_sets: None,
            }),
            _ => Err("MP4 のビデオコーデックが H.264, VP8, VP9 のいずれでもありません".to_string()),
        }
    }

    /// 指定インデックスのサンプルデータを取得する
    fn get_sample(&self, index: usize) -> EncodedSample {
        let (data_offset, data_size, keyframe, _, _) = self.samples[index];
        let raw_data = &self.file_data[data_offset as usize..data_offset as usize + data_size];

        let data = if self.track_info.codec_type == WebrtcVideoCodecType::H264 {
            let mut annex_b = Vec::new();
            // キーフレームの場合は SPS/PPS を先頭に付与する
            if keyframe && let Some(ref ps) = self.track_info.h264_parameter_sets {
                annex_b.extend_from_slice(ps);
            }
            // AVCC → Annex B 変換
            annex_b.extend_from_slice(&avcc_to_annex_b(raw_data));
            annex_b
        } else {
            raw_data.to_vec()
        };

        EncodedSample {
            data,
            is_keyframe: keyframe,
            width: self.track_info.width as u32,
            height: self.track_info.height as u32,
            codec_type: self.track_info.codec_type,
        }
    }

    /// サンプル数を返す
    fn len(&self) -> usize {
        self.samples.len()
    }

    /// フレーム間隔をマイクロ秒で返す
    fn frame_duration_us(&self, index: usize) -> u64 {
        let (_, _, _, _, duration) = self.samples[index];
        (duration as u64 * 1_000_000) / self.track_info.timescale as u64
    }
}

/// AVCC フォーマット (length-prefixed NAL units) を Annex B フォーマット (start code prefixed) に変換する
fn avcc_to_annex_b(data: &[u8]) -> Vec<u8> {
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

/// MP4 パススルーエンコーダー
///
/// encode() が呼ばれるたびにチャネルからエンコード済みサンプルを取り出し、
/// EncodedImage として WebRTC に渡す。
struct Mp4PassthroughEncoder {
    callback: Option<VideoEncoderEncodedImageCallbackPtr>,
    sample_rx: std_mpsc::Receiver<EncodedSample>,
}

impl VideoEncoderHandler for Mp4PassthroughEncoder {
    fn init_encode(
        &mut self,
        _codec: VideoCodecRef<'_>,
        _settings: VideoEncoderSettingsRef<'_>,
    ) -> VideoCodecStatus {
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

        let sample = match self.sample_rx.try_recv() {
            Ok(sample) => sample,
            Err(_) => return VideoCodecStatus::NoOutput,
        };

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

        let mut codec_specific_info = CodecSpecificInfo::new();
        codec_specific_info.set_codec_type(sample.codec_type);
        if sample.codec_type == WebrtcVideoCodecType::H264 {
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
        self.callback = None;
        VideoCodecStatus::Ok
    }

    fn set_rates(&mut self, _parameters: VideoEncoderRateControlParametersRef<'_>) {
        // パススルーなのでビットレート制御は無視する
    }

    fn get_encoder_info(&mut self) -> VideoEncoderEncoderInfo {
        let mut info = VideoEncoderEncoderInfo::new();
        info.set_implementation_name("MP4Passthrough");
        info.set_is_hardware_accelerated(false);
        info
    }
}

/// MP4 パススルー用の VideoCodecCapability
struct Mp4PassthroughVideoCodecCapability {
    codec_type: WebrtcVideoCodecType,
    sample_rx: std::sync::Mutex<Option<std_mpsc::Receiver<EncodedSample>>>,
}

impl Mp4PassthroughVideoCodecCapability {
    fn new(codec_type: WebrtcVideoCodecType, sample_rx: std_mpsc::Receiver<EncodedSample>) -> Self {
        Self {
            codec_type,
            sample_rx: std::sync::Mutex::new(Some(sample_rx)),
        }
    }
}

impl VideoCodecCapability for Mp4PassthroughVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("mp4-passthrough", "MP4 Passthrough")
    }

    fn is_supported(&self, direction: CodecDirection, codec_type: WebrtcVideoCodecType) -> bool {
        direction == CodecDirection::Encoder && codec_type == self.codec_type
    }

    fn resolve_sdp_format(
        &self,
        direction: CodecDirection,
        codec_type: WebrtcVideoCodecType,
        _parameters: &HashMap<String, String>,
        _scalability_mode: Option<&str>,
    ) -> Option<SdpVideoFormat> {
        if !self.is_supported(direction, codec_type) {
            return None;
        }
        match codec_type {
            WebrtcVideoCodecType::H264 => {
                let mut format = SdpVideoFormat::new("H264");
                format.parameters_mut().set("packetization-mode", "1");
                Some(format)
            }
            WebrtcVideoCodecType::Vp8 => Some(SdpVideoFormat::new("VP8")),
            WebrtcVideoCodecType::Vp9 => Some(SdpVideoFormat::new("VP9")),
            _ => None,
        }
    }

    fn create_video_encoder(
        &self,
        _format: &SdpVideoFormat,
    ) -> Option<Box<dyn VideoEncoderHandler>> {
        let rx = self.sample_rx.lock().ok()?.take()?;
        Some(Box::new(Mp4PassthroughEncoder {
            callback: None,
            sample_rx: rx,
        }))
    }

    fn create_video_decoder(
        &self,
        _format: &SdpVideoFormat,
    ) -> Option<Box<dyn shiguredo_webrtc::VideoDecoderHandler>> {
        None
    }
}

/// MP4 ファイルからビデオフレームを送信するキャプチャラー
struct Mp4VideoCapturer {
    video_source: VideoTrackSource,
    stop: Arc<AtomicBool>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl Mp4VideoCapturer {
    fn new(
        reader: Mp4SampleReader,
        sample_tx: std_mpsc::SyncSender<EncodedSample>,
    ) -> std::result::Result<Self, String> {
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
                for i in 0..reader.len() {
                    if stop_clone.load(Ordering::Acquire) {
                        return;
                    }

                    let sample = reader.get_sample(i);
                    // エンコード済みデータをエンコーダーに送信する
                    if sample_tx.send(sample).is_err() {
                        return;
                    }

                    // ダミー I420 フレームを送信して encode() をトリガーする
                    let timestamp_us = shiguredo_webrtc::time_millis() * 1000;
                    let AdaptFrameResult { applied, .. } =
                        source.adapt_frame(width, height, timestamp_us);
                    if applied {
                        let buffer = I420Buffer::new(width, height);
                        let ts =
                            aligner.translate(timestamp_us, shiguredo_webrtc::time_millis() * 1000);
                        let video_frame = VideoFrame::from_i420(&buffer, ts, 0);
                        source.on_frame(&video_frame);
                    }

                    // フレーム間隔だけ待機する
                    let duration_us = reader.frame_duration_us(i);
                    if duration_us > 0 {
                        thread::sleep(std::time::Duration::from_micros(duration_us));
                    }
                }
                // ループ再生: 先頭に戻る
                rtc_log_info!("MP4 の末尾に到達しました。先頭に戻りループ再生します");
            }
        });

        Ok(Self {
            video_source,
            stop,
            thread_handle: Some(thread_handle),
        })
    }

    fn video_source(&self) -> VideoTrackSource {
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

// --- MP4 パススルー送信ここまで ---

#[cfg(feature = "media-device")]
struct VideoDeviceCapturer {
    capture: shiguredo_video_device::VideoCapture,
    video_source: VideoTrackSource,
}

#[cfg(feature = "media-device")]
impl VideoDeviceCapturer {
    fn new(device_id: Option<String>) -> Result<Self> {
        let source = AdaptedVideoTrackSource::new();
        let video_source = source.cast_to_video_track_source();
        let timestamp_aligner = shiguredo_webrtc::TimestampAligner::new();

        let shared = Arc::new(std::sync::Mutex::new((source, timestamp_aligner)));

        let config = shiguredo_video_device::VideoCaptureConfig {
            device_id,
            width: 640,
            height: 480,
            fps: 30,
        };

        let shared_clone = shared.clone();
        let capture = shiguredo_video_device::VideoCapture::new(config, move |frame| {
            let i420 = match frame.pixel_format {
                shiguredo_video_device::PixelFormat::Nv12 => {
                    let uv = frame.uv_data.unwrap_or(&[]);
                    shiguredo_webrtc::nv12_to_i420(
                        frame.data,
                        frame.stride,
                        uv,
                        frame.stride_uv,
                        frame.width,
                        frame.height,
                    )
                }
                shiguredo_video_device::PixelFormat::Yuy2 => shiguredo_webrtc::yuy2_to_i420(
                    frame.data,
                    frame.stride,
                    frame.width,
                    frame.height,
                ),
                _ => None,
            };

            let Some(buffer) = i420 else { return };
            let Ok(mut guard) = shared_clone.lock() else {
                return;
            };
            let (ref mut source, ref mut aligner) = *guard;

            let AdaptFrameResult { applied, size } =
                source.adapt_frame(frame.width, frame.height, frame.timestamp_us);
            if !applied {
                return;
            }

            let ts = aligner.translate(frame.timestamp_us, shiguredo_webrtc::time_millis() * 1000);

            let video_frame =
                if size.adapted_width != frame.width || size.adapted_height != frame.height {
                    let mut scaled = I420Buffer::new(size.adapted_width, size.adapted_height);
                    scaled.scale_from(&buffer);
                    WebrtcVideoFrame::from_i420(&scaled, ts, 0)
                } else {
                    WebrtcVideoFrame::from_i420(&buffer, ts, 0)
                };

            source.on_frame(&video_frame);
        })?;

        Ok(Self {
            capture,
            video_source,
        })
    }

    fn start(&mut self) -> Result<()> {
        self.capture.start()?;
        Ok(())
    }

    fn video_source(&self) -> VideoTrackSource {
        self.video_source.clone()
    }
}

enum VideoCapturerHolder {
    Mp4(Mp4VideoCapturer),
    Fake(FakeVideoCapturer),
    #[cfg(feature = "media-device")]
    Device(VideoDeviceCapturer),
}

impl VideoCapturerHolder {
    fn start(&mut self) -> Result<()> {
        match self {
            VideoCapturerHolder::Mp4(_) => {
                // Mp4VideoCapturer はコンストラクタでスレッドを開始済み
            }
            VideoCapturerHolder::Fake(capturer) => capturer.start()?,
            #[cfg(feature = "media-device")]
            VideoCapturerHolder::Device(capturer) => capturer.start()?,
        }
        Ok(())
    }

    fn video_source(&self) -> VideoTrackSource {
        match self {
            VideoCapturerHolder::Mp4(capturer) => capturer.video_source(),
            VideoCapturerHolder::Fake(capturer) => capturer.video_source(),
            #[cfg(feature = "media-device")]
            VideoCapturerHolder::Device(capturer) => capturer.video_source(),
        }
    }
}

#[cfg(feature = "media-device")]
struct AudioDeviceCapturer {
    capture: shiguredo_audio_device::AudioCapture,
}

#[cfg(feature = "media-device")]
#[derive(Clone)]
struct SumomoAdmState {
    recording: Arc<AtomicBool>,
    audio_transport: Arc<Mutex<Option<shiguredo_webrtc::AudioTransportRef>>>,
}

#[cfg(feature = "media-device")]
impl SumomoAdmState {
    fn on_recorded_data(
        &self,
        audio_data: *const u8,
        n_samples: usize,
        n_bytes_per_sample: usize,
        n_channels: usize,
        samples_per_sec: u32,
    ) {
        if !self.recording.load(Ordering::SeqCst) {
            return;
        }
        let transport = {
            let stored = self.audio_transport.lock().unwrap();
            *stored
        };
        let transport = match transport {
            Some(transport) => transport,
            None => return,
        };
        let mut new_mic_level = 0;
        let _ = unsafe {
            transport.recorded_data_is_available(
                audio_data,
                n_samples,
                n_bytes_per_sample,
                n_channels,
                samples_per_sec,
                0,
                0,
                0,
                false,
                &mut new_mic_level,
                None,
            )
        };
    }
}

#[cfg(feature = "media-device")]
#[derive(Clone)]
struct SumomoAdm {
    adm: shiguredo_webrtc::AudioDeviceModule,
    state: SumomoAdmState,
}

#[cfg(feature = "media-device")]
struct SumomoAdmHandler {
    recording: Arc<AtomicBool>,
    audio_transport: Arc<Mutex<Option<shiguredo_webrtc::AudioTransportRef>>>,
}

#[cfg(feature = "media-device")]
impl shiguredo_webrtc::AudioDeviceModuleHandler for SumomoAdmHandler {
    fn register_audio_callback(
        &self,
        transport: Option<shiguredo_webrtc::AudioTransportRef>,
    ) -> i32 {
        let mut stored = self.audio_transport.lock().unwrap();
        *stored = transport;
        0
    }

    fn init(&self) -> i32 {
        0
    }

    fn terminate(&self) -> i32 {
        0
    }

    fn initialized(&self) -> bool {
        true
    }

    fn recording_devices(&self) -> i16 {
        1
    }

    fn recording_device_name(&self, index: u16) -> Option<(String, String)> {
        if index == 0 {
            Some((
                "External Recording".to_string(),
                "external-recording".to_string(),
            ))
        } else {
            None
        }
    }

    fn recording_is_available(&self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn init_recording(&self) -> i32 {
        0
    }

    fn recording_is_initialized(&self) -> bool {
        true
    }

    fn start_recording(&self) -> i32 {
        self.recording.store(true, Ordering::SeqCst);
        0
    }

    fn stop_recording(&self) -> i32 {
        self.recording.store(false, Ordering::SeqCst);
        0
    }

    fn recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst)
    }
}

#[cfg(feature = "media-device")]
impl SumomoAdm {
    fn new() -> Self {
        let state = SumomoAdmState {
            recording: Arc::new(AtomicBool::new(false)),
            audio_transport: Arc::new(Mutex::new(None)),
        };
        let adm =
            shiguredo_webrtc::AudioDeviceModule::new_with_handler(Box::new(SumomoAdmHandler {
                recording: Arc::clone(&state.recording),
                audio_transport: Arc::clone(&state.audio_transport),
            }));
        Self { adm, state }
    }

    fn audio_device_module(&self) -> shiguredo_webrtc::AudioDeviceModule {
        self.adm.clone()
    }

    fn state(&self) -> SumomoAdmState {
        self.state.clone()
    }
}

#[cfg(feature = "media-device")]
impl AudioDeviceCapturer {
    fn new(device_id: Option<String>, external_state: SumomoAdmState) -> Result<Self> {
        let config = shiguredo_audio_device::AudioCaptureConfig {
            device_id,
            ..Default::default()
        };

        let capture = shiguredo_audio_device::AudioCapture::new(config, move |frame| {
            let state = &external_state;
            let n_channels = frame.channels as usize;
            let samples_per_sec = frame.sample_rate as u32;
            match frame.format {
                shiguredo_audio_device::AudioFormat::S16 => {
                    let n_samples = frame.frames as usize;
                    let n_bytes_per_sample = 2 * n_channels;
                    state.on_recorded_data(
                        frame.data.as_ptr(),
                        n_samples,
                        n_bytes_per_sample,
                        n_channels,
                        samples_per_sec,
                    );
                }
                shiguredo_audio_device::AudioFormat::F32 => {
                    // WebRTC の RecordedDataIsAvailable は S16 を期待するため、
                    // F32 から S16 に変換する
                    if let Some(f32_data) = frame.as_f32() {
                        let s16_data: Vec<i16> = f32_data
                            .iter()
                            .map(|&s| {
                                let clamped = s.clamp(-1.0, 1.0);
                                (clamped * i16::MAX as f32) as i16
                            })
                            .collect();
                        let n_samples = frame.frames as usize;
                        let n_bytes_per_sample = 2 * n_channels;
                        state.on_recorded_data(
                            s16_data.as_ptr() as *const u8,
                            n_samples,
                            n_bytes_per_sample,
                            n_channels,
                            samples_per_sec,
                        );
                    }
                }
            }
        })?;

        Ok(Self { capture })
    }

    fn start(&mut self) -> Result<()> {
        self.capture.start()?;
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    log::log_to_debug(log::Severity::Info);
    log::enable_timestamps();
    log::enable_threads();

    let args = parse_args()?;

    #[cfg(feature = "media-device")]
    if args.list_devices {
        return list_devices();
    }

    let video_enabled = args.video.unwrap_or(true);
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
    let mp4_state = if let Some(ref mp4_path) = args.input_mp4 {
        let reader = Mp4SampleReader::new(mp4_path).map_err(ErrorMessage::new)?;
        let codec_type = reader.track_info.codec_type;
        let (sample_tx, sample_rx) = std_mpsc::sync_channel::<EncodedSample>(2);
        Some((reader, sample_tx, sample_rx, codec_type))
    } else {
        None
    };

    let mut context_config = SoraClientContextConfig {
        #[cfg(feature = "media-device")]
        adm_config: if external_adm.is_some() {
            sora_sdk::AdmConfig::UseExternal(external_adm.as_ref().unwrap().audio_device_module())
        } else {
            sora_sdk::AdmConfig::NoAudioDevice
        },
        #[cfg(not(feature = "media-device"))]
        adm_config: sora_sdk::AdmConfig::NoAudioDevice,
        ..Default::default()
    };

    // --input-mp4 が指定されている場合はパススルー capability を追加する
    let mp4_state = if let Some((reader, sample_tx, sample_rx, codec_type)) = mp4_state {
        let passthrough_capability: Box<dyn VideoCodecCapability> = Box::new(
            Mp4PassthroughVideoCodecCapability::new(codec_type, sample_rx),
        );
        let passthrough_preference =
            VideoCodecPreference::new_from_capability(passthrough_capability.as_ref());
        context_config
            .video_codec_preference
            .merge(&passthrough_preference);
        context_config
            .video_codec_capabilities
            .push(passthrough_capability);
        Some((reader, sample_tx, codec_type))
    } else {
        None
    };

    #[cfg(feature = "nvcodec")]
    {
        let nvcodec_capability: Box<dyn VideoCodecCapability> =
            Box::new(NvCodecVideoCodecCapability::new());
        let nvcodec_preference =
            VideoCodecPreference::new_from_capability(nvcodec_capability.as_ref());
        context_config
            .video_codec_preference
            .merge(&nvcodec_preference);
        context_config
            .video_codec_capabilities
            .push(nvcodec_capability);
    }

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

    let mut builder = SoraClient::builder(
        context.clone(),
        args.signaling_urls,
        args.channel_id,
        args.role,
    )
    .on_notify({
        let event_tx = event_tx.clone();
        move |text| {
            let _ = event_tx.try_send(AppEvent::Notify(text.to_string()));
        }
    })
    .on_push({
        let event_tx = event_tx.clone();
        move |text| {
            let _ = event_tx.try_send(AppEvent::Push(text.to_string()));
        }
    })
    .on_track({
        let event_tx = event_tx.clone();
        move |transceiver| {
            let _ = event_tx.try_send(AppEvent::OnTrack(transceiver));
        }
    })
    .on_remove_track({
        let event_tx = event_tx.clone();
        move |receiver| {
            let _ = event_tx.try_send(AppEvent::OnRemoveTrack(receiver));
        }
    });

    if let Some(audio) = args.audio {
        builder = builder.audio(sora_sdk::Audio::new_bool(audio));
    }

    if let Some(video) = args.video {
        if video {
            let video_setting = match args.video_codec_type.as_deref() {
                Some("vp8") => sora_sdk::Video::new_vp8(None),
                Some("vp9") => sora_sdk::Video::new_vp9(None, None),
                Some("av1") => sora_sdk::Video::new_av1(None, None),
                Some("h264") => sora_sdk::Video::new_h264(None, None),
                Some("h265") => sora_sdk::Video::new_h265(None, None),
                None => sora_sdk::Video::new_bool(true),
                _ => sora_sdk::Video::new_bool(true),
            };
            builder = builder.video(video_setting);
        } else {
            builder = builder.video(sora_sdk::Video::new_bool(false));
        }
    } else if let Some(ref codec) = args.video_codec_type {
        let video_setting = match codec.as_str() {
            "vp8" => sora_sdk::Video::new_vp8(None),
            "vp9" => sora_sdk::Video::new_vp9(None, None),
            "av1" => sora_sdk::Video::new_av1(None, None),
            "h264" => sora_sdk::Video::new_h264(None, None),
            "h265" => sora_sdk::Video::new_h265(None, None),
            _ => sora_sdk::Video::new_bool(true),
        };
        builder = builder.video(video_setting);
    }

    let mut _video_capturer: Option<VideoCapturerHolder> = None;
    if args.role.wants_send() && video_enabled {
        let mut capturer = if let Some((reader, sample_tx, _codec_type)) = mp4_state {
            // --input-mp4 が最優先
            let mp4_capturer =
                Mp4VideoCapturer::new(reader, sample_tx).map_err(ErrorMessage::new)?;
            VideoCapturerHolder::Mp4(mp4_capturer)
        } else {
            #[cfg(feature = "media-device")]
            {
                if let Some(ref device_id) = args.video_input_device {
                    VideoCapturerHolder::Device(VideoDeviceCapturer::new(Some(device_id.clone()))?)
                } else {
                    let fake = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())?;
                    VideoCapturerHolder::Fake(fake)
                }
            }
            #[cfg(not(feature = "media-device"))]
            {
                let fake = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())?;
                VideoCapturerHolder::Fake(fake)
            }
        };
        capturer.start()?;
        let video_track = context.create_video_track(&capturer.video_source())?;
        builder = builder.sender_video_track(video_track);
        _video_capturer = Some(capturer);
    }

    if args.role.wants_send() {
        let audio_source = context.create_audio_source()?;
        let audio_track = context.create_audio_track(&audio_source)?;
        builder = builder.sender_audio_track(audio_track);
    }

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
    if let (Some(cert), Some(key)) = (args.client_cert, args.client_key) {
        builder = builder.client_cert(cert, key);
    }
    if let Some(ca) = args.ca_cert {
        builder = builder.ca_cert(ca);
    }

    if args.turn_tls_insecure {
        builder = builder.turn_tls_insecure(true);
    }
    if let Some(ca_cert_path) = args.turn_tls_ca_cert {
        let pem_data = std::fs::read(&ca_cert_path)?;
        let cert = rustls_pki_types::CertificateDer::from_pem_slice(&pem_data)
            .map_err(|e| ErrorMessage::new(format!("CA 証明書の読み込みに失敗しました: {e}")))?;
        builder = builder.turn_tls_ca_cert(cert.to_vec());
    }

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
                        let receiver = transceiver.receiver();
                        let track = receiver.track();
                        let kind = match track.kind() {
                            Ok(kind) => kind,
                            Err(_) => "unknown".to_string(),
                        };
                        if kind != "video" {
                            rtc_log_warning!("ビデオ以外のトラックを受信しました: kind={}", kind);
                            continue;
                        }
                        let track_id = match track.id() {
                            Ok(id) => id,
                            Err(_) => {
                                rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
                                continue;
                            }
                        };
                        let mut video_track = track.cast_to_video_track();
                        if tracks.contains_key(&track_id) {
                            continue;
                        }
                        rtc_log_info!("ビデオ トラックが追加されました: track_id={}", track_id);
                        let first_frame = Arc::new(AtomicBool::new(false));
                        let sink = VideoSink::new_with_handler(Box::new(AnsiTrackSinkHandler {
                            renderer: renderer_for_events.clone(),
                            first_frame,
                            track_id_for_log: track_id.clone(),
                        }));
                        let wants = VideoSinkWants::new();
                        video_track.add_or_update_sink(&sink, &wants);
                        tracks.insert(track_id, TrackEntry { sink });
                    }
                    AppEvent::OnRemoveTrack(receiver) => {
                        let track = receiver.track();
                        let track_id = match track.id() {
                            Ok(id) => id,
                            Err(_) => {
                                rtc_log_warning!("MediaStreamTrack の id が取得できませんでした");
                                continue;
                            }
                        };
                        let kind = match track.kind() {
                            Ok(kind) => kind,
                            Err(_) => "unknown".to_string(),
                        };
                        if kind != "video" {
                            rtc_log_warning!("ビデオ以外のトラックが削除されました: kind={}", kind);
                            continue;
                        }
                        let mut video_track = track.cast_to_video_track();
                        if let Some(entry) = tracks.remove(&track_id) {
                            video_track.remove_sink(&entry.sink);
                        }
                        rtc_log_info!("ビデオ トラックが削除されました: track_id={}", track_id);
                    }
                }
            }
        }
    }
    run.await?;
    Ok(())
}
