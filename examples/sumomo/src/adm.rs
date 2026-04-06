use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use shiguredo_webrtc::{AudioDeviceModule, AudioDeviceModuleHandler, AudioTransportRef};

use crate::error::Result;

pub(crate) struct AudioDeviceCapturer {
    capture: shiguredo_audio_device::AudioCapture,
}

#[derive(Clone)]
pub(crate) struct SumomoAdmState {
    recording: Arc<AtomicBool>,
    audio_transport: Arc<Mutex<Option<AudioTransportRef>>>,
}

impl SumomoAdmState {
    fn on_recorded_data(
        &self,
        audio_data: *const u8,
        n_samples: usize,
        n_bytes_per_sample: usize,
        n_channels: usize,
        samples_per_sec: u32,
    ) {
        if !self.recording.load(Ordering::SeqCst) {
            return;
        }
        let transport = {
            let stored = self.audio_transport.lock().unwrap();
            *stored
        };
        let transport = match transport {
            Some(transport) => transport,
            None => return,
        };
        let mut new_mic_level = 0;
        let _ = unsafe {
            transport.recorded_data_is_available(
                audio_data,
                n_samples,
                n_bytes_per_sample,
                n_channels,
                samples_per_sec,
                0,
                0,
                0,
                false,
                &mut new_mic_level,
                None,
            )
        };
    }
}

#[derive(Clone)]
pub(crate) struct SumomoAdm {
    adm: AudioDeviceModule,
    state: SumomoAdmState,
}

struct SumomoAdmHandler {
    recording: Arc<AtomicBool>,
    audio_transport: Arc<Mutex<Option<AudioTransportRef>>>,
}

impl AudioDeviceModuleHandler for SumomoAdmHandler {
    fn register_audio_callback(&self, transport: Option<AudioTransportRef>) -> i32 {
        let mut stored = self.audio_transport.lock().unwrap();
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
                "External Recording".to_string(),
                "external-recording".to_string(),
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

impl SumomoAdm {
    pub(crate) fn new() -> Self {
        let state = SumomoAdmState {
            recording: Arc::new(AtomicBool::new(false)),
            audio_transport: Arc::new(Mutex::new(None)),
        };
        let adm = AudioDeviceModule::new_with_handler(Box::new(SumomoAdmHandler {
            recording: Arc::clone(&state.recording),
            audio_transport: Arc::clone(&state.audio_transport),
        }));
        Self { adm, state }
    }

    pub(crate) fn audio_device_module(&self) -> AudioDeviceModule {
        self.adm.clone()
    }

    pub(crate) fn state(&self) -> SumomoAdmState {
        self.state.clone()
    }
}

impl AudioDeviceCapturer {
    pub(crate) fn new(device_id: Option<String>, external_state: SumomoAdmState) -> Result<Self> {
        let config = shiguredo_audio_device::AudioCaptureConfig {
            device_id,
            ..Default::default()
        };

        let capture = shiguredo_audio_device::AudioCapture::new(config, move |frame| {
            let state = &external_state;
            let n_channels = frame.channels as usize;
            let samples_per_sec = frame.sample_rate as u32;
            match frame.format {
                shiguredo_audio_device::AudioFormat::S16 => {
                    let n_samples = frame.frames as usize;
                    let n_bytes_per_sample = 2 * n_channels;
                    state.on_recorded_data(
                        frame.data.as_ptr(),
                        n_samples,
                        n_bytes_per_sample,
                        n_channels,
                        samples_per_sec,
                    );
                }
                shiguredo_audio_device::AudioFormat::F32 => {
                    // WebRTC の RecordedDataIsAvailable は S16 を期待するため、
                    // F32 から S16 に変換する
                    if let Some(f32_data) = frame.as_f32() {
                        let s16_data: Vec<i16> = f32_data
                            .iter()
                            .map(|&s| {
                                let clamped = s.clamp(-1.0, 1.0);
                                (clamped * i16::MAX as f32) as i16
                            })
                            .collect();
                        let n_samples = frame.frames as usize;
                        let n_bytes_per_sample = 2 * n_channels;
                        state.on_recorded_data(
                            s16_data.as_ptr() as *const u8,
                            n_samples,
                            n_bytes_per_sample,
                            n_channels,
                            samples_per_sec,
                        );
                    }
                }
            }
        })?;

        Ok(Self { capture })
    }

    pub(crate) fn start(&mut self) -> Result<()> {
        self.capture.start()?;
        Ok(())
    }
}
