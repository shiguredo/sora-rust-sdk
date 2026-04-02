use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serial_test::serial;
use shiguredo_video_device::{
    PixelFormat, VideoCapture, VideoCaptureConfig, VideoDeviceList, VideoFrame,
};

/// テスト用: 最初のデバイスの対応フォーマットに基づくキャプチャ設定を取得する。
/// デバイスがない、またはフォーマットが取得できない場合は None を返す。
fn first_device_capture_config() -> Option<VideoCaptureConfig> {
    let device_list = VideoDeviceList::enumerate().ok()?;
    if device_list.is_empty() {
        return None;
    }

    let device = &device_list.devices()[0];
    let device_id = device.unique_id().ok();
    let formats = device.formats();

    if formats.is_empty() {
        return None;
    }

    // 640x480 に対応するフォーマットを優先、なければ最初のフォーマットを使用
    let format = formats
        .iter()
        .find(|f| f.width == 640 && f.height == 480)
        .unwrap_or(&formats[0]);

    // VideoFormat の min_fps/max_fps がデバイスの実際の対応フレームレート（離散値）と
    // 一致しない場合があるため、広く対応されている 30fps を使用する
    let min_fps = format.min_fps.ceil().max(1.0) as i32;
    let max_fps = format.max_fps.floor().max(min_fps as f32) as i32;
    let preferred_fps = 30_i32;
    let selected_fps = preferred_fps.clamp(min_fps, max_fps);

    Some(VideoCaptureConfig {
        device_id,
        width: format.width,
        height: format.height,
        fps: selected_fps,
        pixel_format: None,
    })
}

/// テスト用: フレーム受信をタイムアウト付きで待機する。
fn wait_for_frame_count(frame_count: &AtomicU32, timeout: Duration) -> u32 {
    let started_at = Instant::now();
    let poll_interval = Duration::from_millis(100);
    let log_interval = Duration::from_millis(500);
    let mut next_log_at = log_interval;

    loop {
        let count = frame_count.load(Ordering::SeqCst);
        if count > 0 {
            println!(
                "video capture received frame(s): count={}, elapsed_ms={}",
                count,
                started_at.elapsed().as_millis()
            );
            return count;
        }

        let elapsed = started_at.elapsed();
        if elapsed >= timeout {
            println!(
                "video capture timed out: count={}, timeout_ms={}",
                count,
                timeout.as_millis()
            );
            return count;
        }

        if elapsed >= next_log_at {
            println!(
                "video capture waiting for frame: elapsed_ms={}, count={}",
                elapsed.as_millis(),
                count
            );
            next_log_at += log_interval;
        }

        std::thread::sleep(poll_interval);
    }
}

#[test]
fn test_video_device_enumerate() {
    let result = VideoDeviceList::enumerate();
    assert!(result.is_ok(), "デバイス列挙に失敗: {:?}", result.err());

    let device_list = result.unwrap();
    println!("検出されたビデオデバイス数: {}", device_list.len());

    for device in device_list.devices() {
        if let Ok(name) = device.name() {
            println!("  デバイス名: {}", name);
        }
        if let Ok(id) = device.unique_id() {
            println!("  デバイス ID: {}", id);
        }
    }
}

#[test]
fn test_video_device_info() {
    let device_list = match VideoDeviceList::enumerate() {
        Ok(list) => list,
        Err(e) => {
            println!("デバイス列挙に失敗（スキップ）: {:?}", e);
            return;
        }
    };

    if device_list.is_empty() {
        println!("ビデオデバイスが見つかりません（スキップ）");
        return;
    }

    let device = &device_list.devices()[0];

    let name = device.name();
    assert!(name.is_ok(), "デバイス名の取得に失敗: {:?}", name.err());
    let name = name.unwrap();
    assert!(!name.is_empty(), "デバイス名が空");

    let unique_id = device.unique_id();
    assert!(
        unique_id.is_ok(),
        "デバイス ID の取得に失敗: {:?}",
        unique_id.err()
    );
    let unique_id = unique_id.unwrap();
    assert!(!unique_id.is_empty(), "デバイス ID が空");

    println!("デバイス名: {}, ID: {}", name, unique_id);
}

#[test]
fn test_video_capture_config_default() {
    let config = VideoCaptureConfig::default();
    assert!(config.device_id.is_none());
    assert_eq!(config.width, 640);
    assert_eq!(config.height, 480);
    assert_eq!(config.fps, 30);
}

#[test]
fn test_video_device_formats() {
    let device_list = match VideoDeviceList::enumerate() {
        Ok(list) => list,
        Err(e) => {
            println!("デバイス列挙に失敗（スキップ）: {:?}", e);
            return;
        }
    };

    if device_list.is_empty() {
        println!("ビデオデバイスが見つかりません（スキップ）");
        return;
    }

    let device = &device_list.devices()[0];
    let formats = device.formats();

    println!("デバイス: {}", device.name().unwrap_or_default());
    println!("対応フォーマット数: {}", formats.len());

    for format in &formats {
        println!(
            "  {}x{} @ {}-{} fps ({})",
            format.width,
            format.height,
            format.min_fps,
            format.max_fps,
            format.pixel_format.name()
        );

        // 基本的な検証
        assert!(format.width > 0, "幅が 0 以下");
        assert!(format.height > 0, "高さが 0 以下");
        assert!(format.min_fps > 0.0, "最小 fps が 0 以下");
        assert!(
            format.max_fps >= format.min_fps,
            "最大 fps が最小 fps より小さい"
        );
        assert!(
            matches!(
                format.pixel_format,
                PixelFormat::Nv12 | PixelFormat::Yuy2 | PixelFormat::I420
            ),
            "不明なピクセルフォーマット"
        );
    }

    // 少なくとも 1 つのフォーマットがあることを確認
    assert!(!formats.is_empty(), "対応フォーマットがありません");
}

#[test]
#[serial]
fn test_video_capture_session_create() {
    let config = match first_device_capture_config() {
        Some(c) => c,
        None => {
            println!("ビデオデバイスが見つかりません（スキップ）");
            return;
        }
    };

    let expected_width = config.width;
    let expected_height = config.height;
    let expected_fps = config.fps;

    let capture = VideoCapture::new(config, |_frame: VideoFrame<'_>| {});
    assert!(
        capture.is_ok(),
        "キャプチャセッション作成に失敗: {:?}",
        capture.err()
    );

    let capture = capture.unwrap();
    assert_eq!(capture.config().width, expected_width);
    assert_eq!(capture.config().height, expected_height);
    assert_eq!(capture.config().fps, expected_fps);
}

#[test]
#[serial]
fn test_video_capture_start_stop() {
    let config = match first_device_capture_config() {
        Some(c) => c,
        None => {
            println!("ビデオデバイスが見つかりません（スキップ）");
            return;
        }
    };

    let mut capture = match VideoCapture::new(config, |_frame: VideoFrame<'_>| {}) {
        Ok(c) => c,
        Err(e) => {
            println!("キャプチャセッション作成に失敗（スキップ）: {:?}", e);
            return;
        }
    };

    let start_result = capture.start();
    assert!(
        start_result.is_ok(),
        "キャプチャ開始に失敗: {:?}",
        start_result.err()
    );

    // フレームを受け取る時間を確保
    std::thread::sleep(Duration::from_millis(500));

    capture.stop();
    println!("キャプチャ開始・停止テスト完了");
}

#[test]
#[serial]
fn test_video_capture_frame_received() {
    let config = match first_device_capture_config() {
        Some(c) => c,
        None => {
            println!("ビデオデバイスが見つかりません（スキップ）");
            return;
        }
    };

    let frame_count = Arc::new(AtomicU32::new(0));
    let frame_count_clone = frame_count.clone();
    let first_frame_logged = Arc::new(AtomicBool::new(false));
    let first_frame_logged_clone = first_frame_logged.clone();

    let mut capture = match VideoCapture::new(config, move |frame: VideoFrame<'_>| {
        let count = frame_count_clone.fetch_add(1, Ordering::SeqCst) + 1;
        if !first_frame_logged_clone.swap(true, Ordering::SeqCst) {
            println!(
                "video capture first frame received: width={}, height={}, data_len={}, count={}",
                frame.width,
                frame.height,
                frame.data.len(),
                count
            );
        }

        // フレームの基本的な検証
        assert!(frame.width > 0, "幅が 0 以下");
        assert!(frame.height > 0, "高さが 0 以下");
        assert!(!frame.data.is_empty(), "データが空");
    }) {
        Ok(c) => c,
        Err(e) => {
            println!("キャプチャセッション作成に失敗（スキップ）: {:?}", e);
            return;
        }
    };

    let start_result = capture.start();
    assert!(
        start_result.is_ok(),
        "キャプチャ開始に失敗: {:?}",
        start_result.err()
    );
    println!(
        "video capture started: width={}, height={}, fps={}",
        capture.config().width,
        capture.config().height,
        capture.config().fps
    );

    let count = wait_for_frame_count(frame_count.as_ref(), Duration::from_secs(5));

    capture.stop();

    assert!(count > 0, "failed to receive video frame");
    println!("video capture frame count: {}", count);
}
