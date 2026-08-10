use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use shiguredo_webrtc::{I420Buffer, VideoFrameRef, VideoSinkHandler, rtc_log_info};

use crate::error::Result;

/// 受信した I420 フレーム。
pub(crate) struct I420Frame {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) y_data: Vec<u8>,
    // stride は raw-player (SDL の update_yuv) でのみ使う。
    #[cfg(feature = "raw-player")]
    pub(crate) y_stride: i32,
    pub(crate) u_data: Vec<u8>,
    #[cfg(feature = "raw-player")]
    pub(crate) u_stride: i32,
    pub(crate) v_data: Vec<u8>,
    #[cfg(feature = "raw-player")]
    pub(crate) v_stride: i32,
}

impl I420Frame {
    pub(crate) fn from_buffer(buffer: &I420Buffer) -> Self {
        Self {
            width: buffer.width(),
            height: buffer.height(),
            y_data: buffer.y_data().to_vec(),
            #[cfg(feature = "raw-player")]
            y_stride: buffer.stride_y(),
            u_data: buffer.u_data().to_vec(),
            #[cfg(feature = "raw-player")]
            u_stride: buffer.stride_u(),
            v_data: buffer.v_data().to_vec(),
            #[cfg(feature = "raw-player")]
            v_stride: buffer.stride_v(),
        }
    }

    /// I420 プレーンを [I420Buffer] へ変換する。
    ///
    /// libyuv のスケーリング (scale_from) や変換 API が I420Buffer を要求するため、
    /// 同じ寸法の I420Buffer を再構築して返す。
    pub(crate) fn to_buffer(&self) -> I420Buffer {
        let mut buffer = I420Buffer::new(self.width, self.height);
        let (y, u, v) = buffer.planes_mut();
        y.copy_from_slice(&self.y_data);
        u.copy_from_slice(&self.u_data);
        v.copy_from_slice(&self.v_data);
        buffer
    }
}

/// 受信した video frame を描画するレンダラー。
///
/// 通常表示 (ANSI) と raw-player (SDL) で共通のインターフェースを提供し、
/// main loop から frame を渡して描画する。
pub(crate) enum VideoRenderer {
    /// ANSI ターミナルへの描画。
    Ansi(crate::ansi_renderer::AnsiRenderer),
    /// SDL ウィンドウへの描画。
    #[cfg(feature = "raw-player")]
    RawPlayer(crate::raw_player_renderer::RawPlayerRenderer),
}

impl VideoRenderer {
    /// I420 フレームを描画する。
    ///
    /// 失敗は呼び出し元 (main loop) へ返し、primary error として扱う。
    pub(crate) fn render_frame(&mut self, frame: &I420Frame) -> Result<()> {
        match self {
            VideoRenderer::Ansi(renderer) => renderer.render_frame(frame),
            #[cfg(feature = "raw-player")]
            VideoRenderer::RawPlayer(renderer) => renderer.render_frame(frame),
        }
    }

    /// レンダラー固有のイベントを処理する。
    ///
    /// raw-player では SDL の window close / Escape を検出する。
    /// ANSI では何もしない。
    pub(crate) fn poll_events(&mut self) {
        match self {
            VideoRenderer::Ansi(_) => {}
            #[cfg(feature = "raw-player")]
            VideoRenderer::RawPlayer(renderer) => renderer.poll_events(),
        }
    }

    /// レンダラーが描画を継続すべきかどうかを返す。
    ///
    /// raw-player では window close / Escape で false になる。
    /// ANSI では常に true。
    pub(crate) fn is_running(&self) -> bool {
        match self {
            VideoRenderer::Ansi(_) => true,
            #[cfg(feature = "raw-player")]
            VideoRenderer::RawPlayer(renderer) => renderer.is_running(),
        }
    }
}

/// WebRTC の video track の frame を channel へ送る handler。
///
/// frame の描画は行わず、I420 に変換して main loop へ送るだけ。
/// 描画と error 検出は main loop が行う。
pub(crate) struct VideoFrameSinkHandler {
    pub(crate) frame_tx: tokio::sync::mpsc::Sender<I420Frame>,
    pub(crate) first_frame: Arc<AtomicBool>,
    pub(crate) track_id_for_log: String,
}

impl VideoSinkHandler for VideoFrameSinkHandler {
    fn on_frame(&mut self, frame: VideoFrameRef<'_>) {
        if !self.first_frame.swap(true, Ordering::Relaxed) {
            rtc_log_info!("Video frame received: track_id={}", self.track_id_for_log);
        }
        let mut buffer = frame.buffer();
        let Some(i420_buffer) = buffer.to_i420() else {
            return;
        };
        let i420_frame = I420Frame::from_buffer(&i420_buffer);
        let _ = self.frame_tx.try_send(i420_frame);
    }
}
