// shiguredo_audio_device は macOS のみ対応
#![cfg(target_os = "macos")]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use e2e_tests::is_running_on_ci;
use shiguredo_audio_device::{
    AudioCapture, AudioCaptureConfig, AudioDeviceList, AudioFormat, AudioFrame,
};

/// テスト用: 最初のオーディオデバイスの ID を含むキャプチャ設定を取得する。
/// デバイスがない場合は None を返す。
fn first_audio_device_config() -> Option<AudioCaptureConfig> {
    let device_list = AudioDeviceList::enumerate_input().ok()?;
    if device_list.is_empty() {
        return None;
    }

    let device = &device_list.devices()[0];
    let device_id = device.unique_id().ok();

    Some(AudioCaptureConfig {
        device_id,
        sample_rate: 48000,
        channels: 1,
    })
}

#[test]
fn test_audio_device_enumerate() {
    let result = AudioDeviceList::enumerate_input();
    assert!(result.is_ok(), "デバイス列挙に失敗: {:?}", result.err());

    let device_list = result.unwrap();
    println!("検出されたオーディオデバイス数: {}", device_list.len());

    for device in device_list.devices() {
        if let Ok(name) = device.name() {
            println!("  デバイス名: {}", name);
        }
        if let Ok(id) = device.unique_id() {
            println!("  デバイス ID: {}", id);
        }
        println!(
            "  チャンネル数: {}, サンプルレート: {}",
            device.channels(),
            device.sample_rate()
        );
    }
}

#[test]
fn test_audio_device_info() {
    let device_list = match AudioDeviceList::enumerate_input() {
        Ok(list) => list,
        Err(e) => {
            println!("デバイス列挙に失敗（スキップ）: {:?}", e);
            return;
        }
    };

    if device_list.is_empty() {
        println!("オーディオデバイスが見つかりません（スキップ）");
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

    let channels = device.channels();
    assert!(channels > 0, "チャンネル数が 0 以下");

    let sample_rate = device.sample_rate();
    assert!(sample_rate > 0, "サンプルレートが 0 以下");

    println!(
        "デバイス名: {}, ID: {}, チャンネル: {}, サンプルレート: {}",
        name, unique_id, channels, sample_rate
    );
}

#[test]
fn test_audio_capture_config_default() {
    let config = AudioCaptureConfig::default();
    assert!(config.device_id.is_none());
    assert_eq!(config.sample_rate, 48000);
    assert_eq!(config.channels, 1);
}

#[test]
fn test_audio_capture_session_create() {
    if cfg!(target_os = "macos") && is_running_on_ci() {
        println!("CI 環境のためスキップ");
        return;
    }

    let config = match first_audio_device_config() {
        Some(c) => c,
        None => {
            println!("オーディオデバイスが見つかりません（スキップ）");
            return;
        }
    };

    let capture = AudioCapture::new(config, |_frame: AudioFrame<'_>| {});
    assert!(
        capture.is_ok(),
        "キャプチャセッション作成に失敗: {:?}",
        capture.err()
    );

    let capture = capture.unwrap();
    assert!(capture.sample_rate() > 0);
    assert!(capture.channels() > 0);

    println!(
        "実際のサンプルレート: {}, チャンネル: {}",
        capture.sample_rate(),
        capture.channels()
    );
}

#[test]
fn test_audio_capture_start_stop() {
    if cfg!(target_os = "macos") && is_running_on_ci() {
        println!("CI 環境のためスキップ");
        return;
    }

    let config = match first_audio_device_config() {
        Some(c) => c,
        None => {
            println!("オーディオデバイスが見つかりません（スキップ）");
            return;
        }
    };

    let mut capture = match AudioCapture::new(config, |_frame: AudioFrame<'_>| {}) {
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
fn test_audio_capture_frame_received() {
    if cfg!(target_os = "macos") && is_running_on_ci() {
        println!("CI 環境のためスキップ");
        return;
    }

    let config = match first_audio_device_config() {
        Some(c) => c,
        None => {
            println!("オーディオデバイスが見つかりません（スキップ）");
            return;
        }
    };

    let frame_count = Arc::new(AtomicU32::new(0));
    let frame_count_clone = frame_count.clone();

    let mut capture = match AudioCapture::new(config, move |frame: AudioFrame<'_>| {
        frame_count_clone.fetch_add(1, Ordering::SeqCst);

        // フレームの基本的な検証
        assert!(frame.frames > 0, "フレーム数が 0 以下");
        assert!(frame.channels > 0, "チャンネル数が 0 以下");
        assert!(frame.sample_rate > 0, "サンプルレートが 0 以下");
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

    // フレームを受け取る時間を確保（1秒）
    std::thread::sleep(Duration::from_secs(1));

    capture.stop();

    let count = frame_count.load(Ordering::SeqCst);
    assert!(count > 0, "フレームを受信できませんでした");
    println!("受信したフレーム数: {}", count);
}

#[test]
fn test_audio_frame_format_conversion() {
    if cfg!(target_os = "macos") && is_running_on_ci() {
        println!("CI 環境のためスキップ");
        return;
    }

    let config = match first_audio_device_config() {
        Some(c) => c,
        None => {
            println!("オーディオデバイスが見つかりません（スキップ）");
            return;
        }
    };

    let s16_count = Arc::new(AtomicU32::new(0));
    let f32_count = Arc::new(AtomicU32::new(0));
    let s16_count_clone = s16_count.clone();
    let f32_count_clone = f32_count.clone();

    let mut capture = match AudioCapture::new(config, move |frame: AudioFrame<'_>| {
        if frame.format == AudioFormat::S16
            && let Some(samples) = frame.as_s16()
        {
            let expected_len = (frame.frames * frame.channels) as usize;
            assert_eq!(samples.len(), expected_len, "S16 サンプル数が一致しません");
            s16_count_clone.fetch_add(1, Ordering::SeqCst);
        }
        if frame.format == AudioFormat::F32
            && let Some(samples) = frame.as_f32()
        {
            let expected_len = (frame.frames * frame.channels) as usize;
            assert_eq!(samples.len(), expected_len, "F32 サンプル数が一致しません");
            f32_count_clone.fetch_add(1, Ordering::SeqCst);
        }
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

    std::thread::sleep(Duration::from_secs(1));

    capture.stop();

    let s16 = s16_count.load(Ordering::SeqCst);
    let f32_val = f32_count.load(Ordering::SeqCst);
    println!("S16 フレーム数: {}, F32 フレーム数: {}", s16, f32_val);
    assert!(
        s16 + f32_val > 0,
        "S16 または F32 のいずれのフォーマットでもフレームを受信できませんでした"
    );
}
