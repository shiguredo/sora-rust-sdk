use std::f64::consts::PI;
use std::sync::Arc as StdArc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use raden::{Circle, Context, Image, Path, PipelineRuntime, PixelFormat, Rect, Rgba32};
use shiguredo_webrtc::{
    AdaptFrameResult, AdaptedVideoTrackSource, TimestampAligner, VideoTrackSource,
};
use sora_sdk::Result;

/// FakeVideoCapturer の設定。
#[derive(Clone)]
pub struct FakeVideoCapturerConfig {
    /// 映像の幅 (デフォルト: 640)
    pub width: i32,
    /// 映像の高さ (デフォルト: 480)
    pub height: i32,
    /// フレームレート (デフォルト: 30)
    pub fps: i32,
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

/// ダミー映像を生成するキャプチャー。
pub struct FakeVideoCapturer {
    source: AdaptedVideoTrackSource,
    timestamp_aligner: Option<TimestampAligner>,
    image: Option<(Image, PipelineRuntime)>,
    width: i32,
    height: i32,
    fps: i32,
    start_time_ms: i64,
    video_source: VideoTrackSource,
    stop: StdArc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FakeVideoCapturer {
    /// 新しい FakeVideoCapturer を作成する。
    pub fn new(config: FakeVideoCapturerConfig) -> Result<Self> {
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
        let image = Image::new(width as u32, height as u32, PixelFormat::Prgb32);
        let runtime = PipelineRuntime::new();
        Ok(Self {
            image: Some((image, runtime)),
            width,
            height,
            fps,
            start_time_ms: shiguredo_webrtc::time_millis(),
            video_source,
            source,
            timestamp_aligner: Some(timestamp_aligner),
            stop: StdArc::new(AtomicBool::new(false)),
            handle: None,
        })
    }

    /// VideoTrackSource を取得する。
    pub fn video_source(&self) -> VideoTrackSource {
        self.video_source.clone()
    }

    /// キャプチャーを開始する。
    pub fn start(&mut self) -> bool {
        if self.handle.is_some() {
            return true;
        }
        let mut source = self.source.clone();
        let mut timestamp_aligner = match self.timestamp_aligner.take() {
            Some(t) => t,
            None => return true,
        };
        let (mut image, mut runtime) = match self.image.take() {
            Some(pair) => pair,
            None => return false,
        };
        let width = self.width;
        let height = self.height;
        let fps = self.fps.max(1);
        let start_time_ms = self.start_time_ms;
        let stop = self.stop.clone();
        let handle = thread::Builder::new()
            .name("fake-video-capturer".to_string())
            .spawn(move || {
                let mut frame_counter: u32 = 0;
                while !stop.load(Ordering::Acquire) {
                    tick_once(
                        &mut source,
                        &mut timestamp_aligner,
                        &mut image,
                        &mut runtime,
                        width,
                        height,
                        fps,
                        start_time_ms,
                        frame_counter,
                    );
                    let sleep_ms = (1000 / fps).saturating_sub(2).max(1);
                    shiguredo_webrtc::thread_sleep_ms(sleep_ms);
                    frame_counter = frame_counter.wrapping_add(1);
                }
            });
        match handle {
            Ok(h) => {
                self.handle = Some(h);
                true
            }
            Err(_) => false,
        }
    }

    /// キャプチャーを停止する。
    pub fn stop(&mut self) {
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

#[allow(clippy::too_many_arguments)]
fn tick_once(
    source: &mut AdaptedVideoTrackSource,
    timestamp_aligner: &mut TimestampAligner,
    image: &mut Image,
    runtime: &mut PipelineRuntime,
    width: i32,
    height: i32,
    fps: i32,
    start_time_ms: i64,
    frame_counter: u32,
) {
    let elapsed_ms = shiguredo_webrtc::time_millis() - start_time_ms;

    let mut ctx = Context::new(image, runtime);

    ctx.set_fill_style(Rgba32::rgb(0, 0, 0));
    ctx.fill_all();

    ctx.save();
    draw_digital_clock(&mut ctx, width, height, elapsed_ms);
    ctx.restore();

    ctx.save();
    draw_animations(&mut ctx, width, height, fps, frame_counter);
    ctx.restore();

    ctx.save();
    draw_boxes(&mut ctx, width, height, frame_counter);
    ctx.restore();

    ctx.end();

    let pixel_data = image.data();
    let Some(src_stride) = width.checked_mul(4) else {
        return;
    };
    let mut buffer = shiguredo_webrtc::I420Buffer::new(width, height);
    let dst_stride_y = buffer.stride_y();
    let dst_stride_u = buffer.stride_u();
    let dst_stride_v = buffer.stride_v();
    let (dst_y, dst_u, dst_v) = buffer.planes_mut();
    if !shiguredo_webrtc::abgr_to_i420(
        pixel_data,
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
        let mut scaled = shiguredo_webrtc::I420Buffer::new(size.adapted_width, size.adapted_height);
        scaled.scale_from(&buffer);
        shiguredo_webrtc::VideoFrame::builder(&scaled.cast_to_video_frame_buffer())
            .set_timestamp_us(translated_timestamp_us)
            .set_rtp_timestamp(0)
            .build()
    } else {
        shiguredo_webrtc::VideoFrame::builder(&buffer.cast_to_video_frame_buffer())
            .set_timestamp_us(translated_timestamp_us)
            .set_rtp_timestamp(0)
            .build()
    };
    source.on_frame(&frame);
}

fn draw_animations(ctx: &mut Context<'_>, width: i32, height: i32, fps: i32, frame_counter: u32) {
    let w = width as f64;
    let h = height as f64;

    ctx.translate(w * 0.5, h * 0.5);
    ctx.rotate(-PI / 2.0);

    ctx.set_fill_style(Rgba32::rgb(255, 255, 255));
    ctx.fill_pie(&raden::Arc::new(0.0, 0.0, w * 0.3, w * 0.3, 0.0, 2.0 * PI));

    ctx.set_fill_style(Rgba32::rgb(160, 160, 160));
    let sweep = (frame_counter % fps as u32) as f64 / fps as f64 * 2.0 * PI;
    ctx.fill_pie(&raden::Arc::new(0.0, 0.0, w * 0.3, w * 0.3, 0.0, sweep));
}

fn draw_boxes(ctx: &mut Context<'_>, width: i32, height: i32, frame_counter: u32) {
    let w = width as f64;
    let h = height as f64;

    let box_size = 50.0;
    let num_boxes = 5;

    for i in 0..num_boxes {
        let phase = ((frame_counter + i * 20) % 100) as f64 / 100.0;
        let x = phase * (w - box_size);
        let y = h * 0.5 + (phase * PI * 2.0).sin() * h * 0.2;

        let color = match i % 5 {
            0 => Rgba32::rgb(255, 0, 0),
            1 => Rgba32::rgb(0, 255, 0),
            2 => Rgba32::rgb(0, 0, 255),
            3 => Rgba32::rgb(255, 255, 0),
            4 => Rgba32::rgb(255, 0, 255),
            _ => Rgba32::rgb(255, 255, 255),
        };

        ctx.set_fill_style(color);
        ctx.fill_rect(&Rect::new(x, y, box_size, box_size));
    }
}

fn draw_digital_clock(ctx: &mut Context<'_>, width: i32, height: i32, elapsed_ms: i64) {
    let w = width as f64;
    let h = height as f64;

    let hours = (elapsed_ms / (60 * 60 * 1000)) % 10000;
    let minutes = (elapsed_ms / (60 * 1000)) % 60;
    let seconds = (elapsed_ms / 1000) % 60;
    let milliseconds = elapsed_ms % 1000;

    let clock_x = w * 0.02;
    let clock_y = h * 0.02;
    let digit_width = w * 0.018;
    let digit_height = h * 0.04;
    let spacing = digit_width * 0.3;
    let colon_width = digit_width * 0.3;

    ctx.set_fill_style(Rgba32::rgb(0, 255, 255));

    let mut x = clock_x;

    // 時間（4 桁）
    draw_7segment(
        ctx,
        ((hours / 1000) % 10) as i32,
        x,
        clock_y,
        digit_width,
        digit_height,
    );
    x += digit_width + spacing;
    draw_7segment(
        ctx,
        ((hours / 100) % 10) as i32,
        x,
        clock_y,
        digit_width,
        digit_height,
    );
    x += digit_width + spacing;
    draw_7segment(
        ctx,
        ((hours / 10) % 10) as i32,
        x,
        clock_y,
        digit_width,
        digit_height,
    );
    x += digit_width + spacing;
    draw_7segment(
        ctx,
        (hours % 10) as i32,
        x,
        clock_y,
        digit_width,
        digit_height,
    );
    x += digit_width + spacing;

    draw_colon(ctx, x, clock_y, digit_height);
    x += colon_width + spacing;

    // 分（2 桁）
    draw_7segment(
        ctx,
        (minutes / 10) as i32,
        x,
        clock_y,
        digit_width,
        digit_height,
    );
    x += digit_width + spacing;
    draw_7segment(
        ctx,
        (minutes % 10) as i32,
        x,
        clock_y,
        digit_width,
        digit_height,
    );
    x += digit_width + spacing;

    draw_colon(ctx, x, clock_y, digit_height);
    x += colon_width + spacing;

    // 秒（2 桁）
    draw_7segment(
        ctx,
        (seconds / 10) as i32,
        x,
        clock_y,
        digit_width,
        digit_height,
    );
    x += digit_width + spacing;
    draw_7segment(
        ctx,
        (seconds % 10) as i32,
        x,
        clock_y,
        digit_width,
        digit_height,
    );
    x += digit_width + spacing;

    // ドット
    ctx.fill_circle(&Circle::new(
        x + colon_width * 0.3,
        clock_y + digit_height * 0.8,
        digit_height * 0.05,
    ));
    x += colon_width + spacing;

    // ミリ秒（3 桁）
    let ms_digit_width = digit_width * 0.7;
    let ms_digit_height = digit_height * 0.7;

    ctx.set_fill_style(Rgba32::rgb(200, 200, 200));
    draw_7segment(
        ctx,
        ((milliseconds / 100) % 10) as i32,
        x,
        clock_y + (digit_height - ms_digit_height) / 2.0,
        ms_digit_width,
        ms_digit_height,
    );
    x += ms_digit_width + spacing * 0.8;
    draw_7segment(
        ctx,
        ((milliseconds / 10) % 10) as i32,
        x,
        clock_y + (digit_height - ms_digit_height) / 2.0,
        ms_digit_width,
        ms_digit_height,
    );
    x += ms_digit_width + spacing * 0.8;
    draw_7segment(
        ctx,
        (milliseconds % 10) as i32,
        x,
        clock_y + (digit_height - ms_digit_height) / 2.0,
        ms_digit_width,
        ms_digit_height,
    );
}

fn draw_7segment(ctx: &mut Context<'_>, digit: i32, x: f64, y: f64, width: f64, height: f64) {
    let thickness = width * 0.15;
    let gap = thickness * 0.2;

    let segments: [[bool; 7]; 10] = [
        [true, true, true, true, true, true, false],     // 0
        [false, true, true, false, false, false, false], // 1
        [true, true, false, true, true, false, true],    // 2
        [true, true, true, true, false, false, true],    // 3
        [false, true, true, false, false, true, true],   // 4
        [true, false, true, true, false, true, true],    // 5
        [true, false, true, true, true, true, true],     // 6
        [true, true, true, false, false, false, false],  // 7
        [true, true, true, true, true, true, true],      // 8
        [true, true, true, true, false, true, true],     // 9
    ];

    if !(0..=9).contains(&digit) {
        return;
    }
    let seg = &segments[digit as usize];

    let draw_horizontal = |ctx: &mut Context<'_>, sx: f64, sy: f64| {
        let mut path = Path::new();
        path.move_to(sx + gap, sy);
        path.line_to(sx + width - gap, sy);
        path.line_to(sx + width - gap - thickness * 0.5, sy + thickness * 0.5);
        path.line_to(sx + width - gap, sy + thickness);
        path.line_to(sx + gap, sy + thickness);
        path.line_to(sx + gap + thickness * 0.5, sy + thickness * 0.5);
        path.close();
        ctx.fill_path(&path);
    };

    let draw_vertical = |ctx: &mut Context<'_>, sx: f64, sy: f64, sh: f64| {
        let mut path = Path::new();
        path.move_to(sx, sy + gap);
        path.line_to(sx + thickness * 0.5, sy + gap + thickness * 0.5);
        path.line_to(sx + thickness, sy + gap);
        path.line_to(sx + thickness, sy + sh - gap);
        path.line_to(sx + thickness * 0.5, sy + sh - gap - thickness * 0.5);
        path.line_to(sx, sy + sh - gap);
        path.close();
        ctx.fill_path(&path);
    };

    if seg[0] {
        draw_horizontal(ctx, x, y);
    }
    if seg[1] {
        draw_vertical(ctx, x + width - thickness, y, height * 0.5);
    }
    if seg[2] {
        draw_vertical(ctx, x + width - thickness, y + height * 0.5, height * 0.5);
    }
    if seg[3] {
        draw_horizontal(ctx, x, y + height - thickness);
    }
    if seg[4] {
        draw_vertical(ctx, x, y + height * 0.5, height * 0.5);
    }
    if seg[5] {
        draw_vertical(ctx, x, y, height * 0.5);
    }
    if seg[6] {
        draw_horizontal(ctx, x, y + height * 0.5 - thickness * 0.5);
    }
}

fn draw_colon(ctx: &mut Context<'_>, x: f64, y: f64, height: f64) {
    let dot_size = height * 0.1;
    ctx.fill_circle(&Circle::new(x + dot_size, y + height * 0.3, dot_size));
    ctx.fill_circle(&Circle::new(x + dot_size, y + height * 0.7, dot_size));
}
