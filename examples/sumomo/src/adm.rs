use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use shiguredo_webrtc::{AudioDeviceModule, AudioDeviceModuleHandler, AudioTransportPtr};

#[derive(Clone)]
pub(crate) struct SumomoAdmState {
    recording: Arc<AtomicBool>,
    audio_transport: Arc<Mutex<Option<AudioTransportPtr>>>,
}

impl SumomoAdmState {
    pub(crate) fn on_recorded_data(
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
            let stored = self
                .audio_transport
                .lock()
                .expect("BUG: audio_transport mutex poisoned (another thread panicked while holding the lock)");
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
    audio_transport: Arc<Mutex<Option<AudioTransportPtr>>>,
}

impl AudioDeviceModuleHandler for SumomoAdmHandler {
    fn register_audio_callback(&mut self, transport: Option<AudioTransportPtr>) -> i32 {
        let mut stored = self.audio_transport.lock().expect(
            "BUG: audio_transport mutex poisoned (another thread panicked while holding the lock)",
        );
        *stored = transport;
        0
    }

    fn init(&mut self) -> i32 {
        0
    }

    fn terminate(&mut self) -> i32 {
        0
    }

    fn initialized(&mut self) -> bool {
        true
    }

    fn recording_devices(&mut self) -> i16 {
        1
    }

    fn recording_device_name(&mut self, index: u16) -> Option<(String, String)> {
        if index == 0 {
            Some((
                "External Recording".to_string(),
                "external-recording".to_string(),
            ))
        } else {
            None
        }
    }

    fn recording_is_available(&mut self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn init_recording(&mut self) -> i32 {
        0
    }

    fn recording_is_initialized(&mut self) -> bool {
        true
    }

    fn start_recording(&mut self) -> i32 {
        self.recording.store(true, Ordering::SeqCst);
        0
    }

    fn stop_recording(&mut self) -> i32 {
        self.recording.store(false, Ordering::SeqCst);
        0
    }

    fn recording(&mut self) -> bool {
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
