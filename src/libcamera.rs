use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use shiguredo_libcamera::{
    CameraManager, ConfigStatus, ControlId, ControlType, Direction, FrameBufferAllocator,
    FrameStatus, PixelFormat, Rectangle, RequestStatus, Size, StreamRole, core, draft, rpi,
};
use shiguredo_webrtc::{
    AdaptFrameResult, AdaptedVideoTrackSource, I420Buffer, NV12Buffer, TimestampAligner,
    VideoFrame, VideoFrameBuffer, VideoTrackSource, i420_copy, nv12_copy, rtc_log_error,
    rtc_log_info, rtc_log_warning,
};

use crate::error::{Error, Result};

const DEFAULT_CAMERA_INDEX: u32 = 0;
const DEFAULT_WIDTH: i32 = 640;
const DEFAULT_HEIGHT: i32 = 480;
const YU12_FOURCC: u32 = u32::from_le_bytes([b'Y', b'U', b'1', b'2']);
const NV12_FOURCC: u32 = u32::from_le_bytes([b'N', b'V', b'1', b'2']);

pub struct LibcameraVideoCapturerBuilder {
    camera_index: u32,
    width: i32,
    height: i32,
    controls: Vec<(String, String)>,
}

impl Default for LibcameraVideoCapturerBuilder {
    fn default() -> Self {
        Self {
            camera_index: DEFAULT_CAMERA_INDEX,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            controls: Vec::new(),
        }
    }
}

impl LibcameraVideoCapturerBuilder {
    pub fn camera_index(mut self, camera_index: u32) -> Self {
        self.camera_index = camera_index;
        self
    }

    pub fn width(mut self, width: i32) -> Self {
        self.width = width;
        self
    }

    pub fn height(mut self, height: i32) -> Self {
        self.height = height;
        self
    }

    pub fn control(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.controls.push((key.into(), value.into()));
        self
    }

    pub fn controls(mut self, controls: Vec<(String, String)>) -> Self {
        self.controls.extend(controls);
        self
    }

    pub fn build(self) -> Result<LibcameraVideoCapturer> {
        if self.width <= 0 {
            return Err(Error::LibcameraMessage {
                message: format!("width must be greater than 0: {}", self.width),
            });
        }
        if self.height <= 0 {
            return Err(Error::LibcameraMessage {
                message: format!("height must be greater than 0: {}", self.height),
            });
        }

        let source = AdaptedVideoTrackSource::new();
        let video_source = source.cast_to_video_track_source();
        Ok(LibcameraVideoCapturer {
            source,
            video_source,
            camera_index: self.camera_index,
            width: self.width,
            height: self.height,
            controls: self.controls,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        })
    }
}

pub struct LibcameraVideoCapturer {
    source: AdaptedVideoTrackSource,
    video_source: VideoTrackSource,
    camera_index: u32,
    width: i32,
    height: i32,
    controls: Vec<(String, String)>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LibcameraVideoCapturer {
    pub fn builder() -> LibcameraVideoCapturerBuilder {
        LibcameraVideoCapturerBuilder::default()
    }

    pub fn start(&mut self) -> Result<()> {
        if self.thread.is_some() {
            return Ok(());
        }

        self.stop.store(false, Ordering::Release);

        let source = self.source.clone();
        let camera_index = self.camera_index;
        let width = self.width;
        let height = self.height;
        let controls = self.controls.clone();
        let stop = self.stop.clone();

        let handle = thread::Builder::new()
            .name("libcamera-capturer".to_string())
            .spawn(move || {
                if let Err(err) =
                    run_libcamera_loop(source, camera_index, width, height, controls, stop)
                {
                    rtc_log_error!("libcamera capture failed: {}", err);
                }
            })?;

        self.thread = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    pub fn video_source(&self) -> VideoTrackSource {
        self.video_source.clone()
    }
}

impl Drop for LibcameraVideoCapturer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Copy)]
enum FramePixelFormat {
    I420,
    NV12,
}

#[derive(Clone, Copy)]
struct FramePlaneInfo {
    fd: i32,
    offset: u32,
    length: u32,
}

type FrameInfo = (Vec<FramePlaneInfo>, i64);

fn run_libcamera_loop(
    mut source: AdaptedVideoTrackSource,
    camera_index: u32,
    width: i32,
    height: i32,
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

    let mut requests: Vec<shiguredo_libcamera::Request> = Vec::new();
    let mut camera_started = false;
    let result = (|| -> Result<()> {
        let mut camera_config = camera.generate_configuration(&[StreamRole::VideoRecording])?;

        {
            let mut stream_config = camera_config.at(0)?;
            stream_config.set_size(Size {
                width: width as u32,
                height: height as u32,
            });
            stream_config.set_pixel_format(PixelFormat::from_fourcc(NV12_FOURCC));
        }

        let mut status = camera_config.validate();
        if status.is_err() {
            {
                let mut stream_config = camera_config.at(0)?;
                stream_config.set_pixel_format(PixelFormat::from_fourcc(YU12_FOURCC));
            }
            status = camera_config.validate();
        }

        let status = status?;

        camera.configure(&mut camera_config)?;

        let (width, height, stride, frame_pixel_format) = {
            let stream_config = camera_config.at(0)?;
            let size = stream_config.size();
            let pixel_format = stream_config.pixel_format();
            let frame_pixel_format = if pixel_format.fourcc == YU12_FOURCC {
                FramePixelFormat::I420
            } else if pixel_format.fourcc == NV12_FOURCC {
                FramePixelFormat::NV12
            } else {
                return Err(Error::LibcameraMessage {
                    message: format!("unsupported pixel format: {}", pixel_format),
                });
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

        let allocator = FrameBufferAllocator::new(&camera);
        let buffer_count = allocator.allocate(&stream)?;

        let (tx, rx) = std::sync::mpsc::channel::<(u64, Option<FrameInfo>)>();
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

            let planes_count = buffer.planes_count();
            if planes_count == 0 {
                let _ = tx.send((completed.cookie(), None));
                return;
            }

            let mut planes = Vec::with_capacity(planes_count);
            for index in 0..planes_count {
                let Some(plane) = buffer.plane(index) else {
                    continue;
                };
                planes.push(FramePlaneInfo {
                    fd: plane.fd,
                    offset: plane.offset,
                    length: plane.length,
                });
            }

            if planes.is_empty() {
                let _ = tx.send((completed.cookie(), None));
                return;
            }

            let timestamp_us = (metadata.timestamp / 1000) as i64;
            let _ = tx.send((completed.cookie(), Some((planes, timestamp_us))));
        });

        let parsed_controls = parse_controls(&controls);

        requests.clear();
        requests.reserve(buffer_count);
        for index in 0..buffer_count {
            let buffer = allocator.get_buffer(&stream, index)?;
            let request = camera.create_request(index as u64)?;
            request.add_buffer(&stream, &buffer)?;
            apply_controls(&request, &parsed_controls);
            requests.push(request);
        }

        camera.start()?;
        camera_started = true;

        for request in &requests {
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
        let mut stopping = false;
        let mut stopping_idle_ticks = 0u32;

        loop {
            if !stopping && stop.load(Ordering::Acquire) {
                // 停止要求後は requeue を止め、完了通知の流れが落ち着くまで待機する。
                stopping = true;
                stopping_idle_ticks = 0;
            }

            let recv_result = rx.recv_timeout(Duration::from_millis(100));
            let (cookie, frame_info) = match recv_result {
                Ok(v) => {
                    if stopping {
                        stopping_idle_ticks = 0;
                    }
                    v
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if stopping {
                        stopping_idle_ticks += 1;
                        if stopping_idle_ticks >= 5 {
                            break;
                        }
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };

            if stopping {
                continue;
            }

            if let Some((planes, timestamp_us)) = frame_info {
                match frame_pixel_format {
                    FramePixelFormat::I420 => {
                        let buffer_result = copy_i420_planes_to_buffer(&planes, width, height);
                        match buffer_result {
                            Ok(buffer) => on_frame_buffer(
                                &mut source,
                                &mut aligner,
                                buffer.cast_to_video_frame_buffer(),
                                width,
                                height,
                                timestamp_us,
                            ),
                            Err(err) => {
                                rtc_log_warning!(
                                    "failed to read i420 frame: width={} height={} stride={} planes={} err={}",
                                    width,
                                    height,
                                    stride,
                                    planes.len(),
                                    err
                                );
                            }
                        }
                    }
                    FramePixelFormat::NV12 => {
                        let buffer_result = copy_nv12_planes_to_buffer(&planes, width, height);
                        match buffer_result {
                            Ok(buffer) => on_frame_buffer(
                                &mut source,
                                &mut aligner,
                                buffer.cast_to_video_frame_buffer(),
                                width,
                                height,
                                timestamp_us,
                            ),
                            Err(err) => {
                                rtc_log_warning!(
                                    "failed to read nv12 frame: width={} height={} stride={} planes={} err={}",
                                    width,
                                    height,
                                    stride,
                                    planes.len(),
                                    err
                                );
                            }
                        }
                    }
                }
            }

            requeue_request(&camera, &requests, cookie, &parsed_controls);
        }

        Ok(())
    })();

    if camera_started {
        let _ = camera.stop();
    }
    let _ = camera.release();

    rtc_log_info!("libcamera capture stopped");

    result
}

fn on_frame_buffer(
    source: &mut AdaptedVideoTrackSource,
    aligner: &mut TimestampAligner,
    mut frame_buffer: VideoFrameBuffer,
    width: i32,
    height: i32,
    timestamp_us: i64,
) {
    let AdaptFrameResult { applied, size } = source.adapt_frame(width, height, timestamp_us);
    if !applied {
        return;
    }

    let translated_timestamp_us =
        aligner.translate(timestamp_us, shiguredo_webrtc::time_millis() * 1000);

    if size.adapted_width != width || size.adapted_height != height {
        frame_buffer = match frame_buffer.scale(size.adapted_width, size.adapted_height) {
            Some(buffer) => buffer,
            None => {
                rtc_log_warning!(
                    "failed to scale frame buffer: src={}x{} dst={}x{}",
                    width,
                    height,
                    size.adapted_width,
                    size.adapted_height
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

struct MappedBuffer {
    ptr: *mut std::ffi::c_void,
    mapped_len: usize,
    data_offset: usize,
    data_len: usize,
}

impl MappedBuffer {
    fn as_slice(&self) -> &[u8] {
        let data_ptr = unsafe { (self.ptr as *const u8).add(self.data_offset) };
        unsafe { std::slice::from_raw_parts(data_ptr, self.data_len) }
    }
}

impl Drop for MappedBuffer {
    fn drop(&mut self) {
        if self.ptr != libc::MAP_FAILED {
            unsafe {
                libc::munmap(self.ptr, self.mapped_len);
            }
        }
    }
}

fn map_dmabuf_readonly(fd: i32, offset: u32, length: u32) -> Result<MappedBuffer> {
    let data_len = length as usize;
    let page_size_raw = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size_raw <= 0 {
        return Err(Error::LibcameraMessage {
            message: "failed to get system page size".to_string(),
        });
    }
    let page_size = page_size_raw as usize;
    let offset_usize = offset as usize;
    let aligned_offset = offset_usize / page_size * page_size;
    let data_offset = offset_usize - aligned_offset;
    let mapped_len = data_len
        .checked_add(data_offset)
        .ok_or_else(|| Error::LibcameraMessage {
            message: "mapped length overflow".to_string(),
        })?;

    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            mapped_len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            aligned_offset as libc::off_t,
        )
    };

    if ptr == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(MappedBuffer {
        ptr,
        mapped_len,
        data_offset,
        data_len,
    })
}

fn plane_stride_from_len(plane_len: usize, rows: usize) -> Option<i32> {
    if rows == 0 || plane_len == 0 || !plane_len.is_multiple_of(rows) {
        return None;
    }
    i32::try_from(plane_len / rows).ok()
}

fn copy_i420_planes_to_buffer(
    planes: &[FramePlaneInfo],
    width: i32,
    height: i32,
) -> Result<I420Buffer> {
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

    let y_plane = map_dmabuf_readonly(planes[0].fd, planes[0].offset, planes[0].length)?;
    let u_plane = map_dmabuf_readonly(planes[1].fd, planes[1].offset, planes[1].length)?;
    let v_plane = map_dmabuf_readonly(planes[2].fd, planes[2].offset, planes[2].length)?;

    let height_rows = height as usize;
    let chroma_rows = ((height + 1) / 2) as usize;
    let src_y = y_plane.as_slice();
    let src_u = u_plane.as_slice();
    let src_v = v_plane.as_slice();
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
            message: "failed to copy i420 frame using libyuv".to_string(),
        });
    }

    Ok(buffer)
}

fn copy_nv12_planes_to_buffer(
    planes: &[FramePlaneInfo],
    width: i32,
    height: i32,
) -> Result<NV12Buffer> {
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

    let y_plane = map_dmabuf_readonly(planes[0].fd, planes[0].offset, planes[0].length)?;
    let uv_plane = map_dmabuf_readonly(planes[1].fd, planes[1].offset, planes[1].length)?;

    let height_rows = height as usize;
    let chroma_rows = ((height + 1) / 2) as usize;
    let src_y = y_plane.as_slice();
    let src_uv = uv_plane.as_slice();
    let src_stride_y = match plane_stride_from_len(src_y.len(), height_rows) {
        Some(value) => value,
        None => {
            return Err(Error::LibcameraMessage {
                message: format!(
                    "invalid nv12 y stride: bytes={} rows={}",
                    src_y.len(),
                    height_rows
                ),
            });
        }
    };
    let src_stride_uv = match plane_stride_from_len(src_uv.len(), chroma_rows) {
        Some(value) => value,
        None => {
            return Err(Error::LibcameraMessage {
                message: format!(
                    "invalid nv12 uv stride: bytes={} rows={}",
                    src_uv.len(),
                    chroma_rows
                ),
            });
        }
    };

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
            message: "failed to copy nv12 frame using libyuv".to_string(),
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
