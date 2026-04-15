use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use shiguredo_webrtc::{
    AdaptFrameResult, AdaptedVideoTrackSource, I420Buffer, TimestampAligner, VideoFrame,
    VideoTrackSource,
};

use crate::error::Result;

/// u32 のスライスを読み取り専用の u8 スライスとして扱う。
fn u32_slice_as_u8_slice(data: &[u32]) -> &[u8] {
    let len = std::mem::size_of_val(data);
    let ptr = data.as_ptr() as *const u8;
    // 安全性: u32 の連続領域を読み取り専用の u8 スライスとして扱う。
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// on_tick は 1 秒ごとに、1 秒の先頭で生成されるフレームだけで呼ばれる。
#[derive(Clone)]
pub(crate) struct FakeVideoCapturerConfig {
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

pub(crate) struct FakeVideoCapturer {
    source: AdaptedVideoTrackSource,
    timestamp_aligner: Option<TimestampAligner>,
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
    pub(crate) fn new(config: FakeVideoCapturerConfig) -> Result<Self> {
        let width = if config.width > 0 { config.width } else { 640 };
        let height = if config.height > 0 {
            config.height
        } else {
            480
        };
        let fps = if config.fps > 0 { config.fps } else { 30 };
        let source = AdaptedVideoTrackSource::new();
        let timestamp_aligner = TimestampAligner::new();
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

    pub(crate) fn video_source(&self) -> VideoTrackSource {
        self.video_source.clone()
    }

    pub(crate) fn start(&mut self) -> Result<()> {
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
                    shiguredo_webrtc::Thread::sleep_ms(sleep_ms);
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
    timestamp_aligner: &mut TimestampAligner,
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

    let Some(src_stride) = width.checked_mul(4) else {
        return;
    };
    let mut buffer = I420Buffer::new(width, height);
    let dst_stride_y = buffer.stride_y();
    let dst_stride_u = buffer.stride_u();
    let dst_stride_v = buffer.stride_v();
    let (dst_y, dst_u, dst_v) = buffer.planes_mut();
    if !shiguredo_webrtc::abgr_to_i420(
        u32_slice_as_u8_slice(image),
        src_stride,
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width,
        height,
    ) {
        return;
    }
    let timestamp_us = elapsed_ms * 1000;
    let translated_timestamp_us =
        timestamp_aligner.translate(timestamp_us, shiguredo_webrtc::time_millis() * 1000);
    let AdaptFrameResult { applied, size } = source.adapt_frame(width, height, timestamp_us);
    let frame = if applied && (size.adapted_width != width || size.adapted_height != height) {
        let mut scaled = I420Buffer::new(size.adapted_width, size.adapted_height);
        scaled.scale_from(&buffer);
        VideoFrame::builder(&scaled.cast_to_video_frame_buffer())
            .set_timestamp_us(translated_timestamp_us)
            .set_rtp_timestamp(0)
            .build()
    } else {
        VideoFrame::builder(&buffer.cast_to_video_frame_buffer())
            .set_timestamp_us(translated_timestamp_us)
            .set_rtp_timestamp(0)
            .build()
    };
    source.on_frame(&frame);
}
