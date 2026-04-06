use std::fmt::Write as FmtWrite;
use std::io;
use std::io::Write as IoWrite;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use shiguredo_webrtc::{
    I420Buffer, LibyuvFourcc, VideoFrameRef, VideoSinkHandler, convert_from_i420, rtc_log_info,
};

/// ANSI 描画用の簡易レンダラー。
pub(crate) struct AnsiRenderer {
    width: i32,
    height: i32,
}

impl AnsiRenderer {
    pub(crate) fn new() -> Self {
        Self {
            width: 80,
            height: 45,
        }
    }

    pub(crate) fn render(&self, frame: VideoFrameRef) {
        render_frame(frame, self.width, self.height);
    }
}

pub(crate) struct AnsiTrackSinkHandler {
    pub(crate) renderer: Arc<AnsiRenderer>,
    pub(crate) first_frame: Arc<AtomicBool>,
    pub(crate) track_id_for_log: String,
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
    let mut src = frame.buffer();
    let Some(src_i420) = src.to_i420() else {
        return;
    };
    let mut scaled = I420Buffer::new(width, height);
    scaled.scale_from(&src_i420);

    let width_u = width.max(0) as usize;
    let height_u = height.max(0) as usize;
    let Some(dst_stride) = width_u.checked_mul(4) else {
        return;
    };
    let Some(dst_bytes) = dst_stride.checked_mul(height_u) else {
        return;
    };
    let mut image = vec![0u8; dst_bytes];
    if !convert_from_i420(
        scaled.y_data(),
        scaled.stride_y(),
        scaled.u_data(),
        scaled.stride_u(),
        scaled.v_data(),
        scaled.stride_v(),
        &mut image,
        dst_stride as i32,
        width,
        height,
        LibyuvFourcc::Argb,
    ) {
        return;
    }
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
