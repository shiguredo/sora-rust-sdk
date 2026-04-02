use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
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
    // 一致しない場合があるため、範囲内の 30fps 相当値を使用する
    let min_fps = format.min_fps.ceil().max(1.0) as i32;
    let max_fps = format.max_fps.floor().max(min_fps as f32) as i32;
    let selected_fps = 30_i32.clamp(min_fps, max_fps);

    Some(VideoCaptureConfig {
        device_id,
        width: format.width,
        height: format.height,
        fps: selected_fps,
        pixel_format: Some(format.pixel_format),
    })
}

/// テスト用: 指定したフレーム数に到達するまでタイムアウト付きで待機する。
fn wait_for_frame_count(
    frame_count: &AtomicU32,
    target_count: u32,
    timeout: Duration,
    phase: &str,
) -> Option<(u32, Duration)> {
    let started_at = Instant::now();
    let poll_interval = Duration::from_millis(100);
    let log_interval = Duration::from_millis(500);
    let mut next_log_at = log_interval;

    loop {
        let count = frame_count.load(Ordering::SeqCst);
        if count >= target_count {
            println!(
                "video capture target reached: phase={}, count={}, target_count={}, elapsed_ms={}",
                phase,
                count,
                target_count,
                started_at.elapsed().as_millis()
            );
            return Some((count, started_at.elapsed()));
        }

        let elapsed = started_at.elapsed();
        if elapsed >= timeout {
            println!(
                "video capture timed out: phase={}, count={}, target_count={}, timeout_ms={}",
                phase,
                count,
                target_count,
                timeout.as_millis()
            );
            return None;
        }

        if elapsed >= next_log_at {
            println!(
                "video capture waiting for frame: phase={}, elapsed_ms={}, count={}, target_count={}",
                phase,
                elapsed.as_millis(),
                count,
                target_count
            );
            next_log_at += log_interval;
        }

        std::thread::sleep(poll_interval);
    }
}

/// テスト用: 最初のデバイスから、相性違いに備えた複数のキャプチャ設定候補を作る。
fn first_device_capture_config_candidates(max_configs: usize) -> Option<Vec<VideoCaptureConfig>> {
    if max_configs == 0 {
        return Some(Vec::new());
    }

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

    let preferred_index = formats
        .iter()
        .position(|f| f.width == 640 && f.height == 480)
        .unwrap_or(0);

    let mut ordered_formats = Vec::new();
    ordered_formats.push(&formats[preferred_index]);
    for (index, format) in formats.iter().enumerate() {
        if index != preferred_index {
            ordered_formats.push(format);
        }
    }

    let mut configs = Vec::new();
    for format in ordered_formats.into_iter().take(max_configs) {
        let min_fps = format.min_fps.ceil().max(1.0) as i32;
        let max_fps = format.max_fps.floor().max(min_fps as f32) as i32;
        let selected_fps = 30_i32.clamp(min_fps, max_fps);
        configs.push(VideoCaptureConfig {
            device_id: device_id.clone(),
            width: format.width,
            height: format.height,
            fps: selected_fps,
            pixel_format: Some(format.pixel_format),
        });
    }

    Some(configs)
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
    const MAX_ATTEMPTS: usize = 6;
    let mut attempt_errors = Vec::new();

    let configs = match first_device_capture_config_candidates(MAX_ATTEMPTS) {
        Some(c) if !c.is_empty() => c,
        Some(_) | None => {
            println!("ビデオデバイスが見つかりません（スキップ）");
            return;
        }
    };
    println!("video capture config candidates: {}", configs.len());

    for (attempt_index, config) in configs.into_iter().enumerate() {
        let attempt = attempt_index + 1;
        let width = config.width;
        let height = config.height;
        let fps = config.fps;
        let pixel_format_name = match config.pixel_format {
            Some(format) => format.name(),
            None => "auto",
        };

        println!(
            "video capture attempt started: attempt={}, width={}, height={}, fps={}, pixel_format={}",
            attempt, width, height, fps, pixel_format_name
        );

        let frame_count = Arc::new(AtomicU32::new(0));
        let frame_count_clone = frame_count.clone();

        let mut capture = match VideoCapture::new(config, move |frame: VideoFrame<'_>| {
            frame_count_clone.fetch_add(1, Ordering::SeqCst);

            // フレームの基本的な検証
            assert!(frame.width > 0, "幅が 0 以下");
            assert!(frame.height > 0, "高さが 0 以下");
            assert!(!frame.data.is_empty(), "データが空");
        }) {
            Ok(c) => c,
            Err(e) => {
                println!(
                    "video capture session creation failed: attempt={}, error={:?}",
                    attempt, e
                );
                attempt_errors.push(format!(
                    "attempt={} config={}x{}@{} {} session_creation_error={:?}",
                    attempt, width, height, fps, pixel_format_name, e
                ));
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        let start_result = capture.start();
        if let Err(e) = start_result {
            capture.stop();
            println!(
                "video capture start failed: attempt={}, error={:?}",
                attempt, e
            );
            attempt_errors.push(format!(
                "attempt={} config={}x{}@{} {} start_error={:?}",
                attempt, width, height, fps, pixel_format_name, e
            ));
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }

        let first = wait_for_frame_count(frame_count.as_ref(), 1, Duration::from_secs(12), "first");
        if first.is_none() {
            capture.stop();
            let current = frame_count.load(Ordering::SeqCst);
            println!(
                "video capture first frame timeout: attempt={}, frame_count={}",
                attempt, current
            );
            attempt_errors.push(format!(
                "attempt={} config={}x{}@{} {} first_frame_timeout frame_count={}",
                attempt, width, height, fps, pixel_format_name, current
            ));
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        let (first_count, first_elapsed) = first.unwrap();
        println!(
            "video capture first frame summary: attempt={}, count={}, elapsed_ms={}",
            attempt,
            first_count,
            first_elapsed.as_millis()
        );

        let next_target = first_count.saturating_add(1);
        let second = wait_for_frame_count(
            frame_count.as_ref(),
            next_target,
            Duration::from_secs(3),
            "continuous",
        );
        if second.is_none() {
            capture.stop();
            let current = frame_count.load(Ordering::SeqCst);
            println!(
                "video capture continuous frame timeout: attempt={}, frame_count={}, target_count={}",
                attempt, current, next_target
            );
            attempt_errors.push(format!(
                "attempt={} config={}x{}@{} {} continuous_frame_timeout frame_count={} target_count={}",
                attempt, width, height, fps, pixel_format_name, current, next_target
            ));
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        let (second_count, second_elapsed) = second.unwrap();
        capture.stop();
        let final_count = frame_count.load(Ordering::SeqCst);
        println!(
            "video capture final summary: attempt={}, first_count={}, second_count={}, final_count={}, second_elapsed_ms={}",
            attempt,
            first_count,
            second_count,
            final_count,
            second_elapsed.as_millis()
        );
        return;
    }

    panic!(
        "failed to receive video frame after retries: {}",
        attempt_errors.join(" | ")
    );
}
