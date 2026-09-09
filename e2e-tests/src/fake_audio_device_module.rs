use std::f64::consts::PI;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use shiguredo_webrtc::{AudioDeviceModule, AudioDeviceModuleHandler, AudioTransportRef};

/// FakeAudioDeviceModule の設定。
#[derive(Clone)]
pub struct FakeAudioDeviceModuleConfig {
    /// サンプルレート (Hz)。デフォルト: 48000
    pub sample_rate: u32,
    /// チャンネル数。デフォルト: 2 (ステレオ)
    pub channels: usize,
    /// 1 フレームの長さ (ms)。デフォルト: 10
    pub frame_duration_ms: u64,
    /// 生成する正弦波の周波数 (Hz)。デフォルト: 440
    pub frequency_hz: f64,
    /// 振幅 (0.0〜1.0)。デフォルト: 0.5
    pub amplitude: f64,
}

impl Default for FakeAudioDeviceModuleConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            frame_duration_ms: 10,
            frequency_hz: 440.0,
            amplitude: 0.5,
        }
    }
}

struct FakeAudioDeviceModuleState {
    recording: Arc<AtomicBool>,
    audio_transport: Arc<Mutex<Option<AudioTransportRef>>>,
    config: FakeAudioDeviceModuleConfig,
    stop: Arc<AtomicBool>,
}

struct FakeAudioDeviceModuleHandler {
    recording: Arc<AtomicBool>,
    audio_transport: Arc<Mutex<Option<AudioTransportRef>>>,
}

impl AudioDeviceModuleHandler for FakeAudioDeviceModuleHandler {
    fn register_audio_callback(&self, transport: Option<AudioTransportRef>) -> i32 {
        let mut stored = self
            .audio_transport
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *stored = transport;
        0
    }

    fn init(&self) -> i32 {
        0
    }

    fn terminate(&self) -> i32 {
        0
    }

    fn initialized(&self) -> bool {
        true
    }

    fn recording_devices(&self) -> i16 {
        1
    }

    fn recording_device_name(&self, index: u16) -> Option<(String, String)> {
        if index == 0 {
            Some((
                "Fake Audio Input".to_string(),
                "fake-audio-input".to_string(),
            ))
        } else {
            None
        }
    }

    fn recording_is_available(&self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn init_recording(&self) -> i32 {
        0
    }

    fn recording_is_initialized(&self) -> bool {
        true
    }

    fn start_recording(&self) -> i32 {
        self.recording.store(true, Ordering::SeqCst);
        0
    }

    fn stop_recording(&self) -> i32 {
        self.recording.store(false, Ordering::SeqCst);
        0
    }

    fn recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst)
    }
}

/// マイク等が存在しない環境でも使えるダミーの AudioDeviceModule。
///
/// [`FakeVideoCapturer`](crate::FakeVideoCapturer) の音声版で、
/// バックグラウンドスレッドが指定周波数の正弦波を生成し、
/// WebRTC の音声入力 (AudioTransport) へ流し込む。
pub struct FakeAudioDeviceModule {
    adm: AudioDeviceModule,
    state: Arc<FakeAudioDeviceModuleState>,
    joiner: Option<JoinHandle<()>>,
}

impl FakeAudioDeviceModule {
    /// 設定を指定してダミーの AudioDeviceModule を生成する。
    pub fn new(config: FakeAudioDeviceModuleConfig) -> Self {
        let state = Arc::new(FakeAudioDeviceModuleState {
            recording: Arc::new(AtomicBool::new(false)),
            audio_transport: Arc::new(Mutex::new(None)),
            config,
            stop: Arc::new(AtomicBool::new(false)),
        });
        let adm = AudioDeviceModule::new_with_handler(Box::new(FakeAudioDeviceModuleHandler {
            recording: Arc::clone(&state.recording),
            audio_transport: Arc::clone(&state.audio_transport),
        }));
        let joiner = Self::spawn_feeding_thread(Arc::clone(&state));
        Self {
            adm,
            state,
            joiner: Some(joiner),
        }
    }

    /// 音声入力として利用する [AudioDeviceModule] を返す。
    pub fn audio_device_module(&self) -> AudioDeviceModule {
        self.adm.clone()
    }

    fn spawn_feeding_thread(state: Arc<FakeAudioDeviceModuleState>) -> JoinHandle<()> {
        thread::spawn(move || {
            let samples_per_channel = (state.config.sample_rate as usize
                * state.config.frame_duration_ms as usize)
                / 1000;
            let samples_per_channel = samples_per_channel.max(1);
            while !state.stop.load(Ordering::SeqCst) {
                if !state.recording.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                let transport = {
                    let stored = state
                        .audio_transport
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    *stored
                };
                let Some(transport) = transport else {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                };

                // チャンネルインターリーブした正弦波 1 フレーム分を生成する。
                let mut buffer = vec![0i16; samples_per_channel * state.config.channels];
                for i in 0..samples_per_channel {
                    let t = i as f64 / state.config.sample_rate as f64;
                    let value =
                        (t * state.config.frequency_hz * 2.0 * PI).sin() * state.config.amplitude;
                    let sample = (value * i16::MAX as f64) as i16;
                    for channel in 0..state.config.channels {
                        buffer[i * state.config.channels + channel] = sample;
                    }
                }

                let mut new_mic_level = 0u32;
                let _ = unsafe {
                    transport.recorded_data_is_available(
                        buffer.as_ptr() as *const u8,
                        samples_per_channel,
                        std::mem::size_of::<i16>(),
                        state.config.channels,
                        state.config.sample_rate,
                        0,
                        0,
                        0,
                        false,
                        &mut new_mic_level,
                        None,
                    )
                };

                thread::sleep(Duration::from_millis(state.config.frame_duration_ms));
            }
        })
    }
}

impl Drop for FakeAudioDeviceModule {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        self.state.recording.store(false, Ordering::SeqCst);
        if let Some(joiner) = self.joiner.take() {
            let _ = joiner.join();
        }
    }
}
