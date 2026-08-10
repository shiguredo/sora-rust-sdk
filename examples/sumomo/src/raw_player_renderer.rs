use crate::error::Result;
use crate::video::I420Frame;

/// SDL のテクスチャ・レンダラー・ウィンドウを解放した後に `raw_player::quit()` を呼ぶための
/// cleanup guard。
///
/// `RawPlayerRenderer` のフィールドに持たせ、`RawPlayerRenderer::drop` で SDL オブジェクトを
/// 明示的に drop した後に、この guard が自動 drop されることで `quit()` が実行される。
struct SdlCleanupGuard;

impl Drop for SdlCleanupGuard {
    fn drop(&mut self) {
        // Safety: SDL オブジェクト (texture / renderer / window) は
        // RawPlayerRenderer::drop で先に drop されている
        unsafe { raw_player::quit() };
    }
}

pub(crate) struct RawPlayerRenderer {
    texture: Option<raw_player::Texture>,
    renderer: Option<raw_player::Renderer>,
    window: Option<raw_player::Window>,
    // フィールドの暗黙 drop 順に依存せず、texture → renderer → window の順に
    // 明示的に drop した後に quit() を呼ぶための RAII guard。
    #[expect(dead_code)]
    cleanup_guard: SdlCleanupGuard,
    running: bool,
}

impl RawPlayerRenderer {
    pub(crate) fn new(title: &str, width: i32, height: i32) -> Result<Self> {
        raw_player::init()?;
        let cleanup_guard = SdlCleanupGuard;
        let window = raw_player::Window::new(title, width, height)?;
        let renderer = raw_player::Renderer::new(&window)?;
        Ok(Self {
            texture: None,
            renderer: Some(renderer),
            window: Some(window),
            cleanup_guard,
            running: true,
        })
    }

    pub(crate) fn render_frame(&mut self, frame: &I420Frame) -> Result<()> {
        let width = frame.width;
        let height = frame.height;

        let needs_recreate = self
            .texture
            .as_ref()
            .is_none_or(|t| t.width() != width || t.height() != height);
        if needs_recreate {
            self.window
                .as_mut()
                .expect("BUG: window が None です")
                .set_size(width, height)?;
            self.texture = Some(raw_player::Texture::new_yuv(
                self.renderer.as_ref().expect("BUG: renderer が None です"),
                width,
                height,
            )?);
        }

        if let Some(ref mut texture) = self.texture {
            texture.update_yuv(
                &frame.y_data,
                frame.y_stride,
                &frame.u_data,
                frame.u_stride,
                &frame.v_data,
                frame.v_stride,
            )?;

            let renderer = self.renderer.as_mut().expect("BUG: renderer が None です");
            renderer.set_draw_color(0, 0, 0, 255)?;
            renderer.clear()?;
            renderer.copy(texture)?;
            renderer.present()?;
        }
        Ok(())
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

impl Drop for RawPlayerRenderer {
    fn drop(&mut self) {
        // SDL オブジェクトを texture → renderer → window の順に明示的に drop する。
        // その後、cleanup_guard が自動 drop され raw_player::quit() が実行される。
        self.texture.take();
        self.renderer.take();
        self.window.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::I420Buffer;

    /// SDL_VIDEODRIVER=dummy を設定して SDL を初期化する。
    fn init_dummy_sdl() {
        // Safety: グローバルな SDL 状態を操作するテスト内でのみ呼ぶ
        unsafe { std::env::set_var("SDL_VIDEODRIVER", "dummy") };
    }

    #[test]
    #[serial_test::serial]
    fn sdl_owner_drops_in_order_and_quits() {
        init_dummy_sdl();

        let mut renderer = RawPlayerRenderer::new("Sumomo - Test", 640, 480)
            .expect("SDL dummy driver での Renderer 作成に失敗しました");
        assert!(renderer.is_running());

        let frame = I420Frame::from_buffer(&I420Buffer::new(640, 480));
        renderer
            .render_frame(&frame)
            .expect("通常の render に失敗しました");

        drop(renderer);
    }

    #[test]
    #[serial_test::serial]
    fn sdl_owner_can_reinitialize_after_drop() {
        init_dummy_sdl();

        {
            let renderer = RawPlayerRenderer::new("Sumomo - Test", 640, 480)
                .expect("SDL dummy driver での Renderer 作成に失敗しました");
            assert!(renderer.is_running());
        }

        let renderer = RawPlayerRenderer::new("Sumomo - Test", 320, 240)
            .expect("quit 後の再初期化に失敗しました");
        assert!(renderer.is_running());
    }

    #[test]
    #[serial_test::serial]
    fn partial_init_failure_quits_and_can_reinitialize() {
        init_dummy_sdl();

        // タイトルに null byte を含むと Window::new が失敗する。
        // init() 成功後に window 作成が失敗した場合も cleanup guard が quit を実行し、
        // その後に再初期化できることを確認する。
        let err = match RawPlayerRenderer::new("Sumomo\0Test", 640, 480) {
            Ok(_) => panic!("null byte を含む title は window 作成に失敗する必要があります"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("Invalid argument"),
            "window 作成失敗は Invalid argument である必要があります: {err}"
        );

        let renderer = RawPlayerRenderer::new("Sumomo - Test", 320, 240)
            .expect("部分初期化 error 後の再初期化に失敗しました");
        assert!(renderer.is_running());
    }

    #[test]
    #[serial_test::serial]
    fn render_fails_on_invalid_frame_size() {
        init_dummy_sdl();

        let mut renderer = RawPlayerRenderer::new("Sumomo - Test", 640, 480)
            .expect("SDL dummy driver での Renderer 作成に失敗しました");
        assert!(renderer.is_running());

        // 実 I420Buffer から frame を作り、width / height を不正な値へ書き換える。
        // 実データ長と stride が一致しないため、SDL の update_yuv が error を返す。
        let mut frame = I420Frame::from_buffer(&I420Buffer::new(640, 480));
        frame.width = 0;
        frame.height = 0;

        let result = renderer.render_frame(&frame);
        assert!(
            result.is_err(),
            "invalid size の frame は render が error になる必要があります"
        );
    }
}
