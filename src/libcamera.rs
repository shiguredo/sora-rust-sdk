//! libcamera を利用した映像キャプチャ機能。
//!
//! `#[cfg(feature = "libcamera")]` でのみコンパイルされる。

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use shiguredo_libcamera::{
    Camera, CameraManager, ConfigStatus, ControlId, ControlType, Direction, FrameBufferAllocator,
    FrameStatus, PixelFormat, Rectangle, RequestStatus, Size, StreamRole, core, draft, rpi,
};
use shiguredo_webrtc::{
    AdaptFrameResult, AdaptedVideoTrackSource, I420Buffer, NV12Buffer, TimestampAligner,
    VideoFrame, VideoFrameBuffer, VideoFrameBufferHandler, VideoTrackSource, i420_copy, nv12_copy,
    rtc_log_error, rtc_log_info, rtc_log_warning,
};

use crate::error::{Error, Result};

const DEFAULT_CAMERA_INDEX: u32 = 0;
const DEFAULT_WIDTH: i32 = 640;
const DEFAULT_HEIGHT: i32 = 480;
const YU12_FOURCC: u32 = u32::from_le_bytes(*b"YU12");
const NV12_FOURCC: u32 = u32::from_le_bytes(*b"NV12");

/// libcamera 映像キャプチャラのビルダー。
///
/// `#[cfg(feature = "libcamera")]` でのみ利用可能。
pub struct LibcameraVideoCapturerBuilder {
    camera_index: u32,
    width: i32,
    height: i32,
    native_frame_output: bool,
    controls: Vec<(String, String)>,
}

impl Default for LibcameraVideoCapturerBuilder {
    fn default() -> Self {
        Self {
            camera_index: DEFAULT_CAMERA_INDEX,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            native_frame_output: false,
            controls: Vec::new(),
        }
    }
}

impl LibcameraVideoCapturerBuilder {
    /// カメラインデックスを設定する。
    ///
    /// デフォルトは `0`。
    pub fn camera_index(mut self, camera_index: u32) -> Self {
        self.camera_index = camera_index;
        self
    }

    /// 映像の幅を設定する。
    ///
    /// デフォルトは `640`。
    pub fn width(mut self, width: i32) -> Self {
        self.width = width;
        self
    }

    /// 映像の高さを設定する。
    ///
    /// デフォルトは `480`。
    pub fn height(mut self, height: i32) -> Self {
        self.height = height;
        self
    }

    /// ネイティブフレーム出力を設定する。
    ///
    /// `true` の場合、DMA-BUF を経由して [`LibcameraNativeFrameBuffer`] を出力する。
    ///
    /// デフォルトは `false`。
    pub fn native_frame_output(mut self, native_frame_output: bool) -> Self {
        self.native_frame_output = native_frame_output;
        self
    }

    /// コントロールを追加で設定する。
    ///
    /// デフォルトは空。
    pub fn control(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.controls.push((key.into(), value.into()));
        self
    }

    /// 複数のコントロールを設定する。
    ///
    /// デフォルトは空。
    pub fn controls(mut self, controls: Vec<(String, String)>) -> Self {
        self.controls.extend(controls);
        self
    }

    /// [`LibcameraVideoCapturer`] を構築する。
    pub fn build(self) -> Result<LibcameraVideoCapturer> {
        if self.width <= 0 || self.height <= 0 {
            return Err(Error::LibcameraMessage {
                message: format!(
                    "width and height must be greater than 0: {}x{}",
                    self.width, self.height
                ),
            });
        }

        let source = AdaptedVideoTrackSource::new();
        Ok(LibcameraVideoCapturer {
            source,
            camera_index: self.camera_index,
            width: self.width,
            height: self.height,
            native_frame_output: self.native_frame_output,
            controls: self.controls,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        })
    }
}

/// libcamera の映像をキャプチャし WebRTC の映像ソースとして提供する。
///
/// `#[cfg(feature = "libcamera")]` でのみ利用可能。
pub struct LibcameraVideoCapturer {
    source: AdaptedVideoTrackSource,
    camera_index: u32,
    width: i32,
    height: i32,
    native_frame_output: bool,
    controls: Vec<(String, String)>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LibcameraVideoCapturer {
    /// [`LibcameraVideoCapturerBuilder`] を返す。
    pub fn builder() -> LibcameraVideoCapturerBuilder {
        LibcameraVideoCapturerBuilder::default()
    }

    /// キャプチャを開始する。
    pub fn start(&mut self) -> Result<()> {
        if self.thread.is_some() {
            return Ok(());
        }

        self.stop.store(false, Ordering::Release);

        let source = self.source.clone();
        let camera_index = self.camera_index;
        let width = self.width;
        let height = self.height;
        let native_frame_output = self.native_frame_output;
        let controls = self.controls.clone();
        let stop = self.stop.clone();

        let handle = thread::Builder::new()
            .name("libcamera-capturer".to_string())
            .spawn(move || {
                if let Err(err) = run_libcamera_loop(
                    source,
                    camera_index,
                    width,
                    height,
                    native_frame_output,
                    controls,
                    stop,
                ) {
                    rtc_log_error!("libcamera capture failed: {}", err);
                }
            })?;

        self.thread = Some(handle);
        Ok(())
    }

    /// キャプチャを停止する。
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    /// WebRTC の [`VideoTrackSource`] を返す。
    pub fn video_source(&self) -> VideoTrackSource {
        self.source.cast_to_video_track_source()
    }
}

impl Drop for LibcameraVideoCapturer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Copy, Debug)]
enum FramePixelFormat {
    I420,
    NV12,
}

#[derive(Debug)]
struct LibcameraNativeRequeueToken {
    cookie: u64,
    tx: std::sync::mpsc::Sender<u64>,
}

impl Drop for LibcameraNativeRequeueToken {
    fn drop(&mut self) {
        let _ = self.tx.send(self.cookie);
    }
}

/// libcamera から取得したネイティブ DMA-BUF フレームバッファ。
///
/// `#[cfg(feature = "libcamera")]` でのみ利用可能。
#[derive(Debug, Clone)]
pub struct LibcameraNativeFrameBuffer {
    raw_width: i32,
    raw_height: i32,
    scaled_width: i32,
    scaled_height: i32,
    fd: i32,
    size: usize,
    stride: i32,
    frame_pixel_format: FramePixelFormat,
    shared_requeue_token: Arc<LibcameraNativeRequeueToken>,
}

#[derive(Debug)]
struct LibcameraNativeFrameBufferConfig {
    raw_width: i32,
    raw_height: i32,
    scaled_width: i32,
    scaled_height: i32,
    fd: i32,
    size: usize,
    stride: i32,
    frame_pixel_format: FramePixelFormat,
    shared_requeue_token: Arc<LibcameraNativeRequeueToken>,
}

impl LibcameraNativeFrameBuffer {
    fn new(config: LibcameraNativeFrameBufferConfig) -> Self {
        Self {
            raw_width: config.raw_width,
            raw_height: config.raw_height,
            scaled_width: config.scaled_width,
            scaled_height: config.scaled_height,
            fd: config.fd,
            size: config.size,
            stride: config.stride,
            frame_pixel_format: config.frame_pixel_format,
            shared_requeue_token: config.shared_requeue_token,
        }
    }

    /// DMA-BUF のファイルディスクリプタを返す。
    pub fn fd(&self) -> i32 {
        self.fd
    }

    /// バッファサイズを返す。
    pub fn size(&self) -> usize {
        self.size
    }

    /// ストライドを返す。
    pub fn stride(&self) -> i32 {
        self.stride
    }

    /// 元の幅を返す。
    pub fn raw_width(&self) -> i32 {
        self.raw_width
    }

    /// 元の高さを返す。
    pub fn raw_height(&self) -> i32 {
        self.raw_height
    }

    /// スケーリング後の幅を返す。
    pub fn scaled_width(&self) -> i32 {
        self.scaled_width
    }

    /// スケーリング後の高さを返す。
    pub fn scaled_height(&self) -> i32 {
        self.scaled_height
    }

    /// I420 形式かどうかを返す。
    pub fn is_i420(&self) -> bool {
        matches!(self.frame_pixel_format, FramePixelFormat::I420)
    }

    /// NV12 形式かどうかを返す。
    pub fn is_nv12(&self) -> bool {
        matches!(self.frame_pixel_format, FramePixelFormat::NV12)
    }
}

#[derive(Clone, Copy)]
struct FrameDispatchConfig {
    width: i32,
    height: i32,
    adapted_width: i32,
    adapted_height: i32,
    timestamp_us: i64,
}

#[derive(Clone, Copy)]
struct NativeFrameDispatchConfig {
    raw_width: i32,
    raw_height: i32,
    scaled_width: i32,
    scaled_height: i32,
    timestamp_us: i64,
    frame_pixel_format: FramePixelFormat,
    native_info: NativeFrameInfo,
    stride: i32,
    cookie: u64,
}

impl VideoFrameBufferHandler for LibcameraNativeFrameBuffer {
    fn width(&self) -> i32 {
        self.scaled_width
    }

    fn height(&self) -> i32 {
        self.scaled_height
    }

    fn to_i420(&mut self) -> Option<I420Buffer> {
        None
    }

    fn crop_and_scale(
        &mut self,
        _offset_x: i32,
        _offset_y: i32,
        _crop_width: i32,
        _crop_height: i32,
        scaled_width: i32,
        scaled_height: i32,
    ) -> Option<VideoFrameBuffer> {
        Some(VideoFrameBuffer::new_with_handler(Box::new(Self {
            raw_width: self.raw_width,
            raw_height: self.raw_height,
            scaled_width,
            scaled_height,
            fd: self.fd,
            size: self.size,
            stride: self.stride,
            frame_pixel_format: self.frame_pixel_format,
            shared_requeue_token: self.shared_requeue_token.clone(),
        })))
    }
}

#[derive(Debug, Clone, Copy)]
struct NativeFrameInfo {
    fd: i32,
    size: usize,
}

struct FrameBufferLayout {
    planes: Vec<shiguredo_libcamera::FrameBufferPlane>,
    shared_fd: i32,
    mapped_len: usize,
}

/// MappedPlane は I420 や NV12 の各平面（Y/U/V, Y/UV）を表す
#[derive(Debug)]
struct MappedPlane {
    mapping: Rc<MappedDmabuf>,
    data_offset: usize,
    data_len: usize,
}

impl MappedPlane {
    fn as_slice(&self) -> &[u8] {
        let data_ptr = unsafe { (self.mapping.ptr as *const u8).add(self.data_offset) };
        unsafe { std::slice::from_raw_parts(data_ptr, self.data_len) }
    }
}

impl AsRef<[u8]> for MappedPlane {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

#[derive(Debug)]
struct MappedDmabuf {
    ptr: *mut std::ffi::c_void,
    mapped_len: usize,
}

impl Drop for MappedDmabuf {
    fn drop(&mut self) {
        if self.ptr != libc::MAP_FAILED {
            unsafe {
                libc::munmap(self.ptr, self.mapped_len);
            }
        }
    }
}

#[derive(Debug)]
enum CapturedFrameBuffers {
    Mapped(Vec<Vec<MappedPlane>>),
    Native(Vec<NativeFrameInfo>),
}

fn run_libcamera_loop(
    source: AdaptedVideoTrackSource,
    camera_index: u32,
    width: i32,
    height: i32,
    native_frame_output: bool,
    controls: Vec<(String, String)>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let manager = CameraManager::new()?;
    if manager.cameras_count() == 0 {
        return Err(Error::LibcameraMessage {
            message: "camera was not found".to_string(),
        });
    }
    if camera_index >= manager.cameras_count() as u32 {
        return Err(Error::LibcameraMessage {
            message: format!(
                "camera index is out of range: index={} count={}",
                camera_index,
                manager.cameras_count()
            ),
        });
    }

    let mut camera = manager.get_camera(camera_index as usize)?;

    camera.acquire()?;

    let result = run_libcamera_loop_inner(
        &mut camera,
        source,
        width,
        height,
        native_frame_output,
        controls,
        stop,
    );

    if let Err(err) = camera.release() {
        rtc_log_error!("failed to release camera: err={}", err);
    }

    rtc_log_info!("libcamera capture stopped");

    result
}

fn run_libcamera_loop_inner(
    camera: &mut Camera,
    mut source: AdaptedVideoTrackSource,
    width: i32,
    height: i32,
    native_frame_output: bool,
    controls: Vec<(String, String)>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let mut requests: Vec<shiguredo_libcamera::Request> = Vec::new();
    let mut camera_config = camera.generate_configuration(&[StreamRole::VideoRecording])?;

    // まず YUV420(YU12) で設定してみて、ダメなら NV12 を試す
    {
        let mut stream_config = camera_config.at(0)?;
        stream_config.set_size(Size {
            width: width as u32,
            height: height as u32,
        });
        stream_config.set_pixel_format(PixelFormat::from_fourcc(YU12_FOURCC));
    }

    let mut status = camera_config.validate();
    if status.is_err() {
        {
            let mut stream_config = camera_config.at(0)?;
            stream_config.set_pixel_format(PixelFormat::from_fourcc(NV12_FOURCC));
        }
        status = camera_config.validate();
    }

    let status = status?;

    camera.configure(&mut camera_config)?;

    let (width, height, stride, frame_pixel_format) = {
        let stream_config = camera_config.at(0)?;
        let size = stream_config.size();
        let pixel_format = stream_config.pixel_format();
        let frame_pixel_format = match pixel_format.fourcc {
            YU12_FOURCC => FramePixelFormat::I420,
            NV12_FOURCC => FramePixelFormat::NV12,
            _ => {
                return Err(Error::LibcameraMessage {
                    message: format!("unsupported pixel format: {}", pixel_format),
                });
            }
        };
        (
            size.width as i32,
            size.height as i32,
            stream_config.stride() as usize,
            frame_pixel_format,
        )
    };

    if status == ConfigStatus::Adjusted {
        rtc_log_info!(
            "libcamera configuration adjusted: width={} height={} stride={}",
            width,
            height,
            stride
        );
    }

    let stream = {
        let stream_config = camera_config.at(0)?;
        stream_config
            .stream()
            .ok_or_else(|| Error::LibcameraMessage {
                message: "failed to get stream".to_string(),
            })?
    };

    let allocator = FrameBufferAllocator::new(camera);
    let buffer_count = allocator.allocate(&stream)?;

    let (tx, rx) = std::sync::mpsc::channel::<(u64, Option<i64>)>();
    let (delayed_requeue_tx, delayed_requeue_rx) = std::sync::mpsc::channel::<u64>();
    let stream_for_callback = stream.clone();
    camera.on_request_completed(move |completed| {
        if completed.status() != RequestStatus::Complete {
            return;
        }

        let Some(buffer) = completed.find_buffer(&stream_for_callback) else {
            return;
        };
        let metadata = buffer.metadata();

        if metadata.status != FrameStatus::Success {
            let _ = tx.send((completed.cookie(), None));
            return;
        }

        let timestamp_us = (metadata.timestamp / 1000) as i64;
        let _ = tx.send((completed.cookie(), Some(timestamp_us)));
    });

    let parsed_controls = parse_controls(&controls);

    let stride_i32 = i32::try_from(stride).map_err(|_| Error::LibcameraMessage {
        message: format!("stride is too large: {}", stride),
    })?;
    let mut captured_frame_buffers = if native_frame_output {
        CapturedFrameBuffers::Native(Vec::with_capacity(buffer_count))
    } else {
        CapturedFrameBuffers::Mapped(Vec::with_capacity(buffer_count))
    };
    requests.clear();
    requests.reserve(buffer_count);
    for index in 0..buffer_count {
        let buffer = allocator.get_buffer(&stream, index)?;
        let frame_buffer_layout = collect_frame_buffer_layout(&buffer)?;
        match &mut captured_frame_buffers {
            CapturedFrameBuffers::Mapped(mapped_buffers) => {
                let mapped_planes = build_mapped_frame_buffer_planes(&frame_buffer_layout)?;
                mapped_buffers.push(mapped_planes);
            }
            CapturedFrameBuffers::Native(native_frame_infos) => {
                let native_frame_info = build_native_frame_info(&frame_buffer_layout);
                native_frame_infos.push(native_frame_info);
            }
        }
        let request = camera.create_request(index as u64)?;
        request.add_buffer(&stream, &buffer)?;
        apply_controls(&request, &parsed_controls);
        requests.push(request);
    }

    camera.start()?;

    let result = run_capture_loop(
        camera,
        &requests,
        &captured_frame_buffers,
        &mut source,
        width,
        height,
        stride_i32,
        frame_pixel_format,
        &parsed_controls,
        rx,
        delayed_requeue_rx,
        &delayed_requeue_tx,
        stop,
        buffer_count,
        stride,
    );

    // リソース（allocator, requests, captured_frame_buffers など）を
    // drop する前に camera.stop() を呼ぶ。
    // camera.stop() より先にこれらのリソースが drop されると、
    // libcamera の CameraManager スレッドが解放済みの FrameBuffer に
    // アクセスして SIGSEGV が発生する。
    let _ = camera.stop();

    result
}

#[expect(clippy::too_many_arguments)]
fn run_capture_loop(
    camera: &shiguredo_libcamera::Camera,
    requests: &[shiguredo_libcamera::Request],
    captured_frame_buffers: &CapturedFrameBuffers,
    source: &mut AdaptedVideoTrackSource,
    width: i32,
    height: i32,
    stride_i32: i32,
    frame_pixel_format: FramePixelFormat,
    parsed_controls: &[ParsedControl],
    rx: std::sync::mpsc::Receiver<(u64, Option<i64>)>,
    delayed_requeue_rx: std::sync::mpsc::Receiver<u64>,
    delayed_requeue_tx: &std::sync::mpsc::Sender<u64>,
    stop: Arc<AtomicBool>,
    buffer_count: usize,
    stride: usize,
) -> Result<()> {
    for request in requests {
        camera.queue_request(request)?;
    }

    rtc_log_info!(
        "libcamera capture started: camera_id={} width={} height={} stride={} buffers={}",
        camera.id(),
        width,
        height,
        stride,
        buffer_count
    );

    let mut aligner = TimestampAligner::new();

    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }

        // 破棄されたネイティブフレームがあったら再キューイングする
        while let Ok(cookie) = delayed_requeue_rx.try_recv() {
            requeue_request(camera, requests, cookie, parsed_controls);
        }

        let recv_result = rx.recv_timeout(Duration::from_millis(100));
        let (cookie, frame_info) = match recv_result {
            Ok(v) => v,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };

        let Some(timestamp_us) = frame_info else {
            rtc_log_warning!(
                "frame is not successful and will be requeued: cookie={}",
                cookie
            );
            requeue_request(camera, requests, cookie, parsed_controls);
            continue;
        };

        let AdaptFrameResult { applied, size } = source.adapt_frame(width, height, timestamp_us);
        if !applied {
            requeue_request(camera, requests, cookie, parsed_controls);
            continue;
        }

        match &captured_frame_buffers {
            CapturedFrameBuffers::Native(native_infos) => {
                let Some(native_info) = native_infos.get(cookie as usize).copied() else {
                    rtc_log_warning!(
                        "native buffer is missing for request cookie: cookie={} buffers={}",
                        cookie,
                        native_infos.len()
                    );
                    requeue_request(camera, requests, cookie, parsed_controls);
                    continue;
                };

                on_native_frame_buffer(
                    source,
                    &mut aligner,
                    NativeFrameDispatchConfig {
                        raw_width: width,
                        raw_height: height,
                        scaled_width: size.adapted_width,
                        scaled_height: size.adapted_height,
                        timestamp_us,
                        frame_pixel_format,
                        native_info,
                        stride: stride_i32,
                        cookie,
                    },
                    delayed_requeue_tx,
                );
                // ネイティブフレームが破棄された後に requeue する必要があるので、ここでは requeue_request は呼ばない
            }
            CapturedFrameBuffers::Mapped(mapped_buffers) => {
                let Some(mapped_planes) = mapped_buffers.get(cookie as usize) else {
                    rtc_log_warning!(
                        "mapped buffer is missing for request cookie: cookie={} buffers={}",
                        cookie,
                        mapped_buffers.len()
                    );
                    requeue_request(camera, requests, cookie, parsed_controls);
                    continue;
                };

                let buffer = match frame_pixel_format {
                    FramePixelFormat::I420 => {
                        copy_i420_planes_to_buffer(mapped_planes, width, height)
                            .map(|v| v.cast_to_video_frame_buffer())
                    }
                    FramePixelFormat::NV12 => {
                        copy_nv12_planes_to_buffer(mapped_planes, width, height)
                            .map(|v| v.cast_to_video_frame_buffer())
                    }
                };
                let buffer = match buffer {
                    Ok(buffer) => buffer,
                    Err(err) => {
                        rtc_log_warning!(
                            "failed to read frame: format={:?} width={} height={} stride={} planes={} err={}",
                            frame_pixel_format,
                            width,
                            height,
                            stride,
                            mapped_planes.len(),
                            err
                        );
                        requeue_request(camera, requests, cookie, parsed_controls);
                        continue;
                    }
                };

                on_frame_buffer(
                    source,
                    &mut aligner,
                    buffer,
                    FrameDispatchConfig {
                        width,
                        height,
                        adapted_width: size.adapted_width,
                        adapted_height: size.adapted_height,
                        timestamp_us,
                    },
                );
                requeue_request(camera, requests, cookie, parsed_controls);
            }
        }
    }

    Ok(())
}

fn on_frame_buffer(
    source: &mut AdaptedVideoTrackSource,
    aligner: &mut TimestampAligner,
    mut frame_buffer: VideoFrameBuffer,
    config: FrameDispatchConfig,
) {
    let translated_timestamp_us =
        aligner.translate(config.timestamp_us, shiguredo_webrtc::time_millis() * 1000);

    if config.adapted_width != config.width || config.adapted_height != config.height {
        frame_buffer = match frame_buffer.scale(config.adapted_width, config.adapted_height) {
            Some(buffer) => buffer,
            None => {
                rtc_log_warning!(
                    "failed to scale frame buffer: src={}x{} dst={}x{}",
                    config.width,
                    config.height,
                    config.adapted_width,
                    config.adapted_height
                );
                return;
            }
        };
    }

    let video_frame = VideoFrame::builder(&frame_buffer)
        .set_timestamp_us(translated_timestamp_us)
        .set_rtp_timestamp(0)
        .build();

    source.on_frame(&video_frame);
}

fn on_native_frame_buffer(
    source: &mut AdaptedVideoTrackSource,
    aligner: &mut TimestampAligner,
    config: NativeFrameDispatchConfig,
    delayed_requeue_tx: &std::sync::mpsc::Sender<u64>,
) {
    let translated_timestamp_us =
        aligner.translate(config.timestamp_us, shiguredo_webrtc::time_millis() * 1000);
    let requeue_token = Arc::new(LibcameraNativeRequeueToken {
        cookie: config.cookie,
        tx: delayed_requeue_tx.clone(),
    });
    let frame_buffer = VideoFrameBuffer::new_with_handler(Box::new(
        LibcameraNativeFrameBuffer::new(LibcameraNativeFrameBufferConfig {
            raw_width: config.raw_width,
            raw_height: config.raw_height,
            scaled_width: config.scaled_width,
            scaled_height: config.scaled_height,
            fd: config.native_info.fd,
            size: config.native_info.size,
            stride: config.stride,
            frame_pixel_format: config.frame_pixel_format,
            shared_requeue_token: requeue_token,
        }),
    ));
    let video_frame = VideoFrame::builder(&frame_buffer)
        .set_timestamp_us(translated_timestamp_us)
        .set_rtp_timestamp(0)
        .build();
    source.on_frame(&video_frame);
}

fn map_dmabuf_readonly(fd: i32, mapped_len: usize) -> Result<MappedDmabuf> {
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            mapped_len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };

    if ptr == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(MappedDmabuf { ptr, mapped_len })
}

fn collect_frame_buffer_layout(
    buffer: &shiguredo_libcamera::FrameBuffer,
) -> Result<FrameBufferLayout> {
    let planes_count = buffer.planes_count();
    if planes_count == 0 {
        return Err(Error::LibcameraMessage {
            message: "frame buffer has no planes".to_string(),
        });
    }

    let planes = (0..planes_count)
        .map(|index| {
            buffer.plane(index).ok_or_else(|| Error::LibcameraMessage {
                message: format!("failed to get plane {} from frame buffer", index),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // 全 plane の fd が一致していることを確認する
    let mut fds = planes.iter().map(|plane| plane.fd);
    let first_fd = fds.next().ok_or_else(|| Error::LibcameraMessage {
        message: "frame buffer has no planes".to_string(),
    })?;
    if !fds.all(|fd| fd == first_fd) {
        return Err(Error::LibcameraMessage {
            message: "all frame planes must share the same fd".to_string(),
        });
    }

    let mapped_len = planes
        .iter()
        .map(|plane| plane.offset as usize + plane.length as usize)
        .max()
        .unwrap_or(0);

    Ok(FrameBufferLayout {
        planes,
        shared_fd: first_fd,
        mapped_len,
    })
}

fn build_mapped_frame_buffer_planes(layout: &FrameBufferLayout) -> Result<Vec<MappedPlane>> {
    let mapping = Rc::new(map_dmabuf_readonly(layout.shared_fd, layout.mapped_len)?);
    Ok(layout
        .planes
        .iter()
        .map(|plane| MappedPlane {
            mapping: mapping.clone(),
            data_offset: plane.offset as usize,
            data_len: plane.length as usize,
        })
        .collect::<Vec<_>>())
}

fn build_native_frame_info(layout: &FrameBufferLayout) -> NativeFrameInfo {
    NativeFrameInfo {
        fd: layout.shared_fd,
        size: layout.mapped_len,
    }
}

fn plane_stride_from_len(plane_len: usize, rows: usize) -> Option<i32> {
    if rows == 0 || plane_len == 0 || !plane_len.is_multiple_of(rows) {
        return None;
    }
    i32::try_from(plane_len / rows).ok()
}

fn copy_i420_planes_to_buffer<P>(planes: &[P], width: i32, height: i32) -> Result<I420Buffer>
where
    P: AsRef<[u8]>,
{
    if width <= 0 || height <= 0 || planes.len() < 3 {
        return Err(Error::LibcameraMessage {
            message: format!(
                "invalid i420 frame metadata: width={} height={} planes={}",
                width,
                height,
                planes.len()
            ),
        });
    }

    let height_rows = height as usize;
    let chroma_rows = (height as usize).div_ceil(2);
    let src_y = planes[0].as_ref();
    let src_u = planes[1].as_ref();
    let src_v = planes[2].as_ref();
    let src_stride_y =
        plane_stride_from_len(src_y.len(), height_rows).ok_or_else(|| Error::LibcameraMessage {
            message: format!(
                "invalid i420 y stride: bytes={} rows={}",
                src_y.len(),
                height_rows
            ),
        })?;
    let src_stride_u =
        plane_stride_from_len(src_u.len(), chroma_rows).ok_or_else(|| Error::LibcameraMessage {
            message: format!(
                "invalid i420 u stride: bytes={} rows={}",
                src_u.len(),
                chroma_rows
            ),
        })?;
    let src_stride_v =
        plane_stride_from_len(src_v.len(), chroma_rows).ok_or_else(|| Error::LibcameraMessage {
            message: format!(
                "invalid i420 v stride: bytes={} rows={}",
                src_v.len(),
                chroma_rows
            ),
        })?;

    let mut buffer = I420Buffer::new(width, height);
    let dst_stride_y = buffer.stride_y();
    let dst_stride_u = buffer.stride_u();
    let dst_stride_v = buffer.stride_v();
    let (dst_y, dst_u, dst_v) = buffer.planes_mut();
    if !i420_copy(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width,
        height,
    ) {
        return Err(Error::LibcameraMessage {
            message: "failed to copy i420 frame".to_string(),
        });
    }

    Ok(buffer)
}

fn copy_nv12_planes_to_buffer<P>(planes: &[P], width: i32, height: i32) -> Result<NV12Buffer>
where
    P: AsRef<[u8]>,
{
    if width <= 0 || height <= 0 || planes.len() < 2 {
        return Err(Error::LibcameraMessage {
            message: format!(
                "invalid nv12 frame metadata: width={} height={} planes={}",
                width,
                height,
                planes.len()
            ),
        });
    }

    let height_rows = height as usize;
    let chroma_rows = (height as usize).div_ceil(2);
    let src_y = planes[0].as_ref();
    let src_uv = planes[1].as_ref();
    let src_stride_y =
        plane_stride_from_len(src_y.len(), height_rows).ok_or_else(|| Error::LibcameraMessage {
            message: format!(
                "invalid nv12 y stride: bytes={} rows={}",
                src_y.len(),
                height_rows
            ),
        })?;
    let src_stride_uv = plane_stride_from_len(src_uv.len(), chroma_rows).ok_or_else(|| {
        Error::LibcameraMessage {
            message: format!(
                "invalid nv12 uv stride: bytes={} rows={}",
                src_uv.len(),
                chroma_rows
            ),
        }
    })?;

    let mut buffer = NV12Buffer::new(width, height);
    let dst_stride_y = buffer.stride_y();
    let dst_stride_uv = buffer.stride_uv();
    let (dst_y, dst_uv) = buffer.planes_mut();
    if !nv12_copy(
        src_y,
        src_stride_y,
        src_uv,
        src_stride_uv,
        dst_y,
        dst_stride_y,
        dst_uv,
        dst_stride_uv,
        width,
        height,
    ) {
        return Err(Error::LibcameraMessage {
            message: "failed to copy nv12 frame".to_string(),
        });
    }

    Ok(buffer)
}

fn requeue_request(
    camera: &shiguredo_libcamera::Camera,
    requests: &[shiguredo_libcamera::Request],
    cookie: u64,
    controls: &[ParsedControl],
) {
    let index = cookie as usize;
    let Some(request) = requests.get(index) else {
        rtc_log_warning!("request cookie is out of range: {}", cookie);
        return;
    };
    request.reuse();
    apply_controls(request, controls);
    if let Err(err) = camera.queue_request(request) {
        rtc_log_warning!("failed to requeue request: {}", err);
    }
}

// パース済みコントロール値
#[derive(Clone)]
enum ControlValue {
    Bool(bool),
    I32(i32),
    I64(i64),
    F32(f32),
    I32Array(Vec<i32>),
    I64Array(Vec<i64>),
    F32Array(Vec<f32>),
    Rect(Rectangle),
    RectArray(Vec<Rectangle>),
}

// パース済みコントロール
#[derive(Clone)]
struct ParsedControl {
    id: &'static ControlId,
    value: ControlValue,
}

fn all_control_ids() -> &'static [&'static ControlId] {
    static IDS: &[&ControlId] = &[
        &core::AE_ENABLE,
        &core::AE_STATE,
        &core::AE_METERING_MODE,
        &core::AE_CONSTRAINT_MODE,
        &core::AE_EXPOSURE_MODE,
        &core::EXPOSURE_VALUE,
        &core::EXPOSURE_TIME,
        &core::EXPOSURE_TIME_MODE,
        &core::ANALOGUE_GAIN,
        &core::ANALOGUE_GAIN_MODE,
        &core::AE_FLICKER_MODE,
        &core::AE_FLICKER_PERIOD,
        &core::AE_FLICKER_DETECTED,
        &core::BRIGHTNESS,
        &core::CONTRAST,
        &core::LUX,
        &core::AWB_ENABLE,
        &core::AWB_MODE,
        &core::AWB_LOCKED,
        &core::COLOUR_GAINS,
        &core::COLOUR_TEMPERATURE,
        &core::SATURATION,
        &core::SENSOR_BLACK_LEVELS,
        &core::SHARPNESS,
        &core::FOCUS_FOM,
        &core::COLOUR_CORRECTION_MATRIX,
        &core::SCALER_CROP,
        &core::DIGITAL_GAIN,
        &core::FRAME_DURATION,
        &core::FRAME_DURATION_LIMITS,
        &core::SENSOR_TEMPERATURE,
        &core::SENSOR_TIMESTAMP,
        &core::AF_MODE,
        &core::AF_RANGE,
        &core::AF_SPEED,
        &core::AF_METERING,
        &core::AF_WINDOWS,
        &core::AF_TRIGGER,
        &core::AF_PAUSE,
        &core::LENS_POSITION,
        &core::AF_STATE,
        &core::AF_PAUSE_STATE,
        &core::HDR_MODE,
        &core::HDR_CHANNEL,
        &core::GAMMA,
        &core::DEBUG_METADATA_ENABLE,
        &core::FRAME_WALL_CLOCK,
        &core::WDR_MODE,
        &core::WDR_STRENGTH,
        &core::WDR_MAX_BRIGHT_PIXELS,
        &core::LENS_DEWARP_ENABLE,
        &core::LENS_SHADING_CORRECTION_ENABLE,
        &draft::AE_PRECAPTURE_TRIGGER,
        &draft::NOISE_REDUCTION_MODE,
        &draft::COLOR_CORRECTION_ABERRATION_MODE,
        &draft::AWB_STATE,
        &draft::SENSOR_ROLLING_SHUTTER_SKEW,
        &draft::LENS_SHADING_MAP_MODE,
        &draft::PIPELINE_DEPTH,
        &draft::MAX_LATENCY,
        &draft::TEST_PATTERN_MODE,
        &draft::FACE_DETECT_MODE,
        &draft::FACE_DETECT_FACE_RECTANGLES,
        &draft::FACE_DETECT_FACE_SCORES,
        &draft::FACE_DETECT_FACE_LANDMARKS,
        &draft::FACE_DETECT_FACE_IDS,
        &rpi::STATS_OUTPUT_ENABLE,
        &rpi::BCM2835_STATS_OUTPUT,
        &rpi::SCALER_CROPS,
        &rpi::PISP_STATS_OUTPUT,
        &rpi::SYNC_MODE,
        &rpi::SYNC_READY,
        &rpi::SYNC_TIMER,
        &rpi::SYNC_FRAMES,
        &rpi::CNN_OUTPUT_TENSOR,
        &rpi::CNN_OUTPUT_TENSOR_INFO,
        &rpi::CNN_ENABLE_INPUT_TENSOR,
        &rpi::CNN_INPUT_TENSOR,
        &rpi::CNN_INPUT_TENSOR_INFO,
        &rpi::CNN_KPI_INFO,
    ];
    IDS
}

fn find_control_id(name: &str) -> Option<&'static ControlId> {
    all_control_ids()
        .iter()
        .find(|id| id.name() == name)
        .copied()
}

fn resolve_enum_value(control_name: &str, value_str: &str) -> Option<i32> {
    if let Ok(value) = value_str.parse::<i32>() {
        return Some(value);
    }

    match control_name {
        "AfMode" => match value_str {
            "Manual" => Some(core::af_mode::MANUAL),
            "Auto" => Some(core::af_mode::AUTO),
            "Continuous" => Some(core::af_mode::CONTINUOUS),
            _ => None,
        },
        "AfRange" => match value_str {
            "Normal" => Some(core::af_range::NORMAL),
            "Macro" => Some(core::af_range::MACRO),
            "Full" => Some(core::af_range::FULL),
            _ => None,
        },
        "AfSpeed" => match value_str {
            "Normal" => Some(core::af_speed::NORMAL),
            "Fast" => Some(core::af_speed::FAST),
            _ => None,
        },
        "AfTrigger" => match value_str {
            "Start" => Some(core::af_trigger::START),
            "Cancel" => Some(core::af_trigger::CANCEL),
            _ => None,
        },
        "AeMeteringMode" => match value_str {
            "CentreWeighted" => Some(core::ae_metering_mode::CENTRE_WEIGHTED),
            "Spot" => Some(core::ae_metering_mode::SPOT),
            "Matrix" => Some(core::ae_metering_mode::MATRIX),
            "Custom" => Some(core::ae_metering_mode::CUSTOM),
            _ => None,
        },
        "AeConstraintMode" => match value_str {
            "Normal" => Some(core::ae_constraint_mode::NORMAL),
            "Highlight" => Some(core::ae_constraint_mode::HIGHLIGHT),
            "Shadows" => Some(core::ae_constraint_mode::SHADOWS),
            "Custom" => Some(core::ae_constraint_mode::CUSTOM),
            _ => None,
        },
        "AeExposureMode" => match value_str {
            "Normal" => Some(core::ae_exposure_mode::NORMAL),
            "Short" => Some(core::ae_exposure_mode::SHORT),
            "Long" => Some(core::ae_exposure_mode::LONG),
            "Custom" => Some(core::ae_exposure_mode::CUSTOM),
            _ => None,
        },
        "ExposureTimeMode" => match value_str {
            "Auto" => Some(core::exposure_time_mode::AUTO),
            "Manual" => Some(core::exposure_time_mode::MANUAL),
            _ => None,
        },
        "AnalogueGainMode" => match value_str {
            "Auto" => Some(core::analogue_gain_mode::AUTO),
            "Manual" => Some(core::analogue_gain_mode::MANUAL),
            _ => None,
        },
        "AwbMode" => match value_str {
            "Auto" => Some(core::awb_mode::AUTO),
            "Incandescent" => Some(core::awb_mode::INCANDESCENT),
            "Tungsten" => Some(core::awb_mode::TUNGSTEN),
            "Fluorescent" => Some(core::awb_mode::FLUORESCENT),
            "Indoor" => Some(core::awb_mode::INDOOR),
            "Daylight" => Some(core::awb_mode::DAYLIGHT),
            "Cloudy" => Some(core::awb_mode::CLOUDY),
            "Custom" => Some(core::awb_mode::CUSTOM),
            _ => None,
        },
        "HdrMode" => match value_str {
            "Off" => Some(core::hdr_mode::OFF),
            "MultiExposureUnmerged" => Some(core::hdr_mode::MULTI_EXPOSURE_UNMERGED),
            "MultiExposure" => Some(core::hdr_mode::MULTI_EXPOSURE),
            "SingleExposure" => Some(core::hdr_mode::SINGLE_EXPOSURE),
            "Night" => Some(core::hdr_mode::NIGHT),
            _ => None,
        },
        "NoiseReductionMode" => match value_str {
            "Off" => Some(draft::noise_reduction_mode::OFF),
            "Fast" => Some(draft::noise_reduction_mode::FAST),
            "HighQuality" => Some(draft::noise_reduction_mode::HIGH_QUALITY),
            "Minimal" => Some(draft::noise_reduction_mode::MINIMAL),
            "ZSL" => Some(draft::noise_reduction_mode::ZSL),
            _ => None,
        },
        _ => None,
    }
}

fn parse_rectangle(value: &str) -> Option<Rectangle> {
    let parts: Vec<&str> = value.split(',').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(Rectangle {
        x: parts[0].trim().parse().ok()?,
        y: parts[1].trim().parse().ok()?,
        width: parts[2].trim().parse().ok()?,
        height: parts[3].trim().parse().ok()?,
    })
}

fn parse_control_value(id: &ControlId, value: &str) -> Option<ControlValue> {
    match id.control_type() {
        ControlType::Bool => {
            let lower = value.to_ascii_lowercase();
            let parsed = match lower.as_str() {
                "0" | "false" => false,
                "1" | "true" => true,
                _ => return None,
            };
            Some(ControlValue::Bool(parsed))
        }
        ControlType::Int32 => {
            if let Some(enum_value) = resolve_enum_value(id.name(), value) {
                return Some(ControlValue::I32(enum_value));
            }
            if value.contains(',') {
                let values: Option<Vec<i32>> = value
                    .split(',')
                    .map(|v| v.trim().parse::<i32>().ok())
                    .collect();
                return values.map(ControlValue::I32Array);
            }
            value.parse::<i32>().ok().map(ControlValue::I32)
        }
        ControlType::Int64 => {
            if value.contains(',') {
                let values: Option<Vec<i64>> = value
                    .split(',')
                    .map(|v| v.trim().parse::<i64>().ok())
                    .collect();
                return values.map(ControlValue::I64Array);
            }
            value.parse::<i64>().ok().map(ControlValue::I64)
        }
        ControlType::Float => {
            if value.contains(',') {
                let values: Option<Vec<f32>> = value
                    .split(',')
                    .map(|v| v.trim().parse::<f32>().ok())
                    .collect();
                return values.map(ControlValue::F32Array);
            }
            value.parse::<f32>().ok().map(ControlValue::F32)
        }
        ControlType::Rectangle => {
            if value.contains(';') {
                let values: Option<Vec<Rectangle>> = value
                    .split(';')
                    .map(|v| parse_rectangle(v.trim()))
                    .collect();
                return values.map(ControlValue::RectArray);
            }
            parse_rectangle(value).map(ControlValue::Rect)
        }
        _ => {
            rtc_log_warning!(
                "unsupported libcamera control type: name={} type={:?}",
                id.name(),
                id.control_type()
            );
            None
        }
    }
}

fn parse_controls(controls: &[(String, String)]) -> Vec<ParsedControl> {
    let mut parsed = Vec::with_capacity(controls.len());

    for (key, raw_value) in controls {
        let Some(id) = find_control_id(key) else {
            rtc_log_warning!("unknown libcamera control: {}", key);
            continue;
        };

        if id.direction() == Direction::Out {
            rtc_log_warning!("read-only libcamera control is ignored: {}", key);
            continue;
        }

        let Some(value) = parse_control_value(id, raw_value) else {
            rtc_log_warning!(
                "invalid libcamera control value: key={} value={}",
                key,
                raw_value
            );
            continue;
        };

        rtc_log_info!(
            "libcamera control configured: key={} value={}",
            key,
            raw_value
        );
        parsed.push(ParsedControl { id, value });
    }

    parsed
}

fn apply_controls(request: &shiguredo_libcamera::Request, controls: &[ParsedControl]) {
    if controls.is_empty() {
        return;
    }

    let mut control_list = request.controls();
    for control in controls {
        match &control.value {
            ControlValue::Bool(value) => control_list.set_bool(control.id, *value),
            ControlValue::I32(value) => control_list.set_i32(control.id, *value),
            ControlValue::I64(value) => control_list.set_i64(control.id, *value),
            ControlValue::F32(value) => control_list.set_f32(control.id, *value),
            ControlValue::I32Array(value) => control_list.set_i32_array(control.id, value),
            ControlValue::I64Array(value) => control_list.set_i64_array(control.id, value),
            ControlValue::F32Array(value) => control_list.set_f32_array(control.id, value),
            ControlValue::Rect(value) => control_list.set_rectangle(control.id, *value),
            ControlValue::RectArray(value) => control_list.set_rectangle_array(control.id, value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::TryRecvError;

    #[test]
    fn copy_i420_planes_to_buffer_reads_pre_mapped_planes() {
        let planes = vec![
            vec![0_u8, 1, 2, 3, 4, 5, 6, 7],
            vec![10_u8, 11],
            vec![20_u8, 21],
        ];

        let buffer = copy_i420_planes_to_buffer(&planes, 4, 2)
            .expect("copy_i420_planes_to_buffer should succeed");

        assert_eq!(&buffer.y_data()[..8], &[0_u8, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(&buffer.u_data()[..2], &[10_u8, 11]);
        assert_eq!(&buffer.v_data()[..2], &[20_u8, 21]);
    }

    #[test]
    fn copy_nv12_planes_to_buffer_reads_pre_mapped_planes() {
        let planes = vec![vec![0_u8, 1, 2, 3, 4, 5, 6, 7], vec![10_u8, 11, 12, 13]];

        let buffer = copy_nv12_planes_to_buffer(&planes, 4, 2)
            .expect("copy_nv12_planes_to_buffer should succeed");

        assert_eq!(&buffer.y_data()[..8], &[0_u8, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(&buffer.uv_data()[..4], &[10_u8, 11, 12, 13]);
    }

    #[test]
    fn copy_i420_planes_to_buffer_rejects_insufficient_planes() {
        let planes = vec![vec![0_u8; 8], vec![0_u8; 2]];
        let err = match copy_i420_planes_to_buffer(&planes, 4, 2) {
            Ok(_) => panic!("copy_i420_planes_to_buffer should fail"),
            Err(err) => err,
        };
        assert!(format!("{err}").contains("invalid i420 frame metadata"));
    }

    #[test]
    fn copy_i420_planes_to_buffer_rejects_invalid_stride() {
        let planes = vec![vec![0_u8; 7], vec![0_u8; 2], vec![0_u8; 2]];
        let err = match copy_i420_planes_to_buffer(&planes, 4, 2) {
            Ok(_) => panic!("copy_i420_planes_to_buffer should fail"),
            Err(err) => err,
        };
        assert!(format!("{err}").contains("invalid i420 y stride"));
    }

    #[test]
    fn copy_nv12_planes_to_buffer_rejects_invalid_stride() {
        let planes = vec![vec![0_u8; 8], vec![0_u8; 3]];
        let err = match copy_nv12_planes_to_buffer(&planes, 4, 4) {
            Ok(_) => panic!("copy_nv12_planes_to_buffer should fail"),
            Err(err) => err,
        };
        assert!(format!("{err}").contains("invalid nv12 uv stride"));
    }

    fn create_native_frame_buffer_for_test(
        cookie: u64,
        raw_width: i32,
        raw_height: i32,
        scaled_width: i32,
        scaled_height: i32,
    ) -> (VideoFrameBuffer, std::sync::mpsc::Receiver<u64>) {
        let (tx, rx) = std::sync::mpsc::channel::<u64>();
        let requeue_token = Arc::new(LibcameraNativeRequeueToken { cookie, tx });
        let buffer = VideoFrameBuffer::new_with_handler(Box::new(LibcameraNativeFrameBuffer::new(
            LibcameraNativeFrameBufferConfig {
                raw_width,
                raw_height,
                scaled_width,
                scaled_height,
                fd: 123,
                size: 4096,
                stride: 640,
                frame_pixel_format: FramePixelFormat::I420,
                shared_requeue_token: requeue_token,
            },
        )));
        (buffer, rx)
    }

    #[test]
    fn native_frame_buffer_crop_and_scale_updates_scaled_size_only() {
        let (mut frame_buffer, _rx) = create_native_frame_buffer_for_test(10, 640, 480, 640, 480);

        let scaled = frame_buffer
            .crop_and_scale(100, 50, 320, 240, 160, 120)
            .expect("crop_and_scale の実行に失敗しました");

        let native_original = unsafe { frame_buffer.as_native_ref::<LibcameraNativeFrameBuffer>() }
            .expect("元バッファから native 参照を取得できませんでした");
        let native_scaled = unsafe { scaled.as_native_ref::<LibcameraNativeFrameBuffer>() }
            .expect("スケール後バッファから native 参照を取得できませんでした");

        assert_eq!(native_original.raw_width(), 640);
        assert_eq!(native_original.raw_height(), 480);
        assert_eq!(native_original.scaled_width(), 640);
        assert_eq!(native_original.scaled_height(), 480);

        assert_eq!(native_scaled.raw_width(), 640);
        assert_eq!(native_scaled.raw_height(), 480);
        assert_eq!(native_scaled.scaled_width(), 160);
        assert_eq!(native_scaled.scaled_height(), 120);
        assert_eq!(native_scaled.fd(), 123);
        assert_eq!(native_scaled.size(), 4096);
        assert_eq!(native_scaled.stride(), 640);
    }

    #[test]
    fn native_frame_buffer_requeue_notified_only_after_all_references_are_dropped() {
        let (mut frame_buffer, rx) = create_native_frame_buffer_for_test(42, 640, 480, 640, 480);

        let scaled = frame_buffer
            .crop_and_scale(0, 0, 640, 480, 320, 240)
            .expect("crop_and_scale の実行に失敗しました");

        drop(frame_buffer);
        match rx.try_recv() {
            Err(TryRecvError::Empty) => {}
            Ok(cookie) => panic!("バッファが残っているのに通知されました: cookie={cookie}"),
            Err(TryRecvError::Disconnected) => panic!("通知チャネルが切断されました"),
        }

        drop(scaled);
        let cookie = rx
            .recv_timeout(Duration::from_millis(100))
            .expect("最終解放後の通知を受信できませんでした");
        assert_eq!(cookie, 42);

        match rx.try_recv() {
            Err(TryRecvError::Empty) => {}
            Ok(extra) => panic!("通知が複数回送信されました: cookie={extra}"),
            Err(TryRecvError::Disconnected) => {}
        }
    }

    #[test]
    fn libcamera_builder_native_frame_output_defaults_to_false() {
        let capturer = LibcameraVideoCapturer::builder()
            .build()
            .expect("キャプチャラの生成に失敗しました");
        assert!(!capturer.native_frame_output);
    }

    #[test]
    fn libcamera_builder_native_frame_output_can_be_enabled() {
        let capturer = LibcameraVideoCapturer::builder()
            .native_frame_output(true)
            .build()
            .expect("キャプチャラの生成に失敗しました");
        assert!(capturer.native_frame_output);
    }

    #[test]
    fn captured_frame_buffers_enforces_exclusive_variants() {
        let mut mapped = CapturedFrameBuffers::Mapped(Vec::with_capacity(1));
        match &mut mapped {
            CapturedFrameBuffers::Mapped(mapped_buffers) => mapped_buffers.push(Vec::new()),
            CapturedFrameBuffers::Native(_) => panic!("mapped variant expected"),
        }
        let native_view = match &mapped {
            CapturedFrameBuffers::Native(native_infos) => native_infos.first(),
            CapturedFrameBuffers::Mapped(_) => None,
        };
        assert!(native_view.is_none());

        let mut native = CapturedFrameBuffers::Native(Vec::with_capacity(1));
        match &mut native {
            CapturedFrameBuffers::Native(native_infos) => {
                native_infos.push(NativeFrameInfo { fd: 3, size: 4 });
            }
            CapturedFrameBuffers::Mapped(_) => panic!("native variant expected"),
        }
        let mapped_view = match &native {
            CapturedFrameBuffers::Mapped(mapped_buffers) => mapped_buffers.first(),
            CapturedFrameBuffers::Native(_) => None,
        };
        assert!(mapped_view.is_none());
    }
}
