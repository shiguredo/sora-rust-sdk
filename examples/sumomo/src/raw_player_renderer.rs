use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use shiguredo_webrtc::{I420Buffer, VideoFrameRef, VideoSinkHandler, rtc_log_info};

use crate::error::Result;

pub(crate) struct I420Frame {
    pub(crate) width: i32,
    pub(crate) height: i32,
    y_data: Vec<u8>,
    y_stride: i32,
    u_data: Vec<u8>,
    u_stride: i32,
    v_data: Vec<u8>,
    v_stride: i32,
}

impl I420Frame {
    pub(crate) fn from_buffer(buffer: &I420Buffer) -> Self {
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

pub(crate) struct RawPlayerRenderer {
    window: raw_player::Window,
    renderer: raw_player::Renderer,
    texture: Option<raw_player::Texture>,
    running: bool,
}

impl RawPlayerRenderer {
    pub(crate) fn new(title: &str, width: i32, height: i32) -> Result<Self> {
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

    pub(crate) fn render(&mut self, frame: &I420Frame) {
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

    pub(crate) fn poll_events(&mut self) {
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

    pub(crate) fn is_running(&self) -> bool {
        self.running
    }
}

pub(crate) struct RawPlayerTrackSinkHandler {
    pub(crate) frame_tx: std::sync::mpsc::SyncSender<I420Frame>,
    pub(crate) first_frame: Arc<AtomicBool>,
    pub(crate) track_id_for_log: String,
}

impl VideoSinkHandler for RawPlayerTrackSinkHandler {
    fn on_frame(&mut self, frame: VideoFrameRef<'_>) {
        if !self.first_frame.swap(true, Ordering::Relaxed) {
            rtc_log_info!(
                "ビデオ フレームを受信しました: track_id={}",
                self.track_id_for_log
            );
        }
        let mut buffer = frame.buffer();
        let Some(i420_buffer) = buffer.to_i420() else {
            return;
        };
        let i420_frame = I420Frame::from_buffer(&i420_buffer);
        let _ = self.frame_tx.try_send(i420_frame);
    }
}
