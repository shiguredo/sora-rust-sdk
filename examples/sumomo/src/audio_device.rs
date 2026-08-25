use crate::adm::SumomoAdmState;
use crate::error::Result;

pub(crate) struct AudioDeviceCapturer {
    capture: shiguredo_audio_device::AudioCapture,
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
