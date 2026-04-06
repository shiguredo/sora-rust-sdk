use shiguredo_webrtc::VideoTrackSource;
use sora_sdk::Mp4VideoCapturer;

use crate::error::Result;
use crate::fake::FakeVideoCapturer;

#[cfg(feature = "media-device")]
pub(crate) fn list_devices() -> Result<()> {
    let video_device_list = shiguredo_video_device::VideoDeviceList::enumerate()?;
    let audio_device_list = shiguredo_audio_device::AudioDeviceList::enumerate_input()?;

    let json = nojson::json(|f| {
        f.set_indent_size(2);
        f.set_spacing(true);
        f.object(|f| {
            f.member(
                "video_devices",
                nojson::array(|f| {
                    for device in video_device_list.devices() {
                        let name = device.name().unwrap_or_else(|_| String::new());
                        let unique_id = device.unique_id().unwrap_or_else(|_| String::new());
                        let formats = device.formats();
                        f.element(nojson::object(|f| {
                            f.member("name", name.as_str())?;
                            f.member("unique_id", unique_id.as_str())?;
                            f.member(
                                "formats",
                                nojson::array(|f| {
                                    for format in &formats {
                                        f.element(nojson::object(|f| {
                                            f.member("width", format.width)?;
                                            f.member("height", format.height)?;
                                            f.member("min_fps", format.min_fps)?;
                                            f.member("max_fps", format.max_fps)?;
                                            f.member("pixel_format", format.pixel_format.name())
                                        }))?;
                                    }
                                    Ok(())
                                }),
                            )
                        }))?;
                    }
                    Ok(())
                }),
            )?;
            f.member(
                "audio_devices",
                nojson::array(|f| {
                    for device in audio_device_list.devices() {
                        let name = device.name().unwrap_or_else(|_| String::new());
                        let unique_id = device.unique_id().unwrap_or_else(|_| String::new());
                        let channels = device.channels();
                        let sample_rate = device.sample_rate();
                        f.element(nojson::object(|f| {
                            f.member("name", name.as_str())?;
                            f.member("unique_id", unique_id.as_str())?;
                            f.member("channels", channels)?;
                            f.member("sample_rate", sample_rate)
                        }))?;
                    }
                    Ok(())
                }),
            )
        })
    });

    shiguredo_webrtc::rtc_log_info!("{}", json);
    Ok(())
}

#[cfg(feature = "media-device")]
pub(crate) struct VideoDeviceCapturer {
    capture: shiguredo_video_device::VideoCapture,
    video_source: VideoTrackSource,
}

#[cfg(feature = "media-device")]
impl VideoDeviceCapturer {
    pub(crate) fn new(device_id: Option<String>) -> Result<Self> {
        use std::sync::{Arc, Mutex};

        use shiguredo_webrtc::{
            AdaptFrameResult, AdaptedVideoTrackSource, I420Buffer, TimestampAligner, VideoFrame,
        };

        let source = AdaptedVideoTrackSource::new();
        let video_source = source.cast_to_video_track_source();
        let timestamp_aligner = TimestampAligner::new();

        let shared = Arc::new(Mutex::new((source, timestamp_aligner)));

        let config = shiguredo_video_device::VideoCaptureConfig {
            device_id,
            width: 640,
            height: 480,
            fps: 30,
            pixel_format: None,
        };

        let shared_clone = shared.clone();
        let capture = shiguredo_video_device::VideoCapture::new(config, move |frame| {
            let buffer = match frame.pixel_format {
                shiguredo_video_device::PixelFormat::Nv12 => {
                    let uv = frame.uv_data.unwrap_or(&[]);
                    let mut buffer = I420Buffer::new(frame.width, frame.height);
                    let dst_stride_y = buffer.stride_y();
                    let dst_stride_u = buffer.stride_u();
                    let dst_stride_v = buffer.stride_v();
                    let (dst_y, dst_u, dst_v) = buffer.planes_mut();
                    if !shiguredo_webrtc::nv12_to_i420(
                        frame.data,
                        frame.stride,
                        uv,
                        frame.stride_uv,
                        dst_y,
                        dst_stride_y,
                        dst_u,
                        dst_stride_u,
                        dst_v,
                        dst_stride_v,
                        frame.width,
                        frame.height,
                    ) {
                        return;
                    }
                    buffer
                }
                shiguredo_video_device::PixelFormat::Yuy2 => {
                    let mut buffer = I420Buffer::new(frame.width, frame.height);
                    let dst_stride_y = buffer.stride_y();
                    let dst_stride_u = buffer.stride_u();
                    let dst_stride_v = buffer.stride_v();
                    let (dst_y, dst_u, dst_v) = buffer.planes_mut();
                    if !shiguredo_webrtc::yuy2_to_i420(
                        frame.data,
                        frame.stride,
                        dst_y,
                        dst_stride_y,
                        dst_u,
                        dst_stride_u,
                        dst_v,
                        dst_stride_v,
                        frame.width,
                        frame.height,
                    ) {
                        return;
                    }
                    buffer
                }
                _ => return,
            };
            let Ok(mut guard) = shared_clone.lock() else {
                return;
            };
            let (ref mut source, ref mut aligner) = *guard;

            let AdaptFrameResult { applied, size } =
                source.adapt_frame(frame.width, frame.height, frame.timestamp_us);
            if !applied {
                return;
            }

            let ts = aligner.translate(frame.timestamp_us, shiguredo_webrtc::time_millis() * 1000);

            let video_frame =
                if size.adapted_width != frame.width || size.adapted_height != frame.height {
                    let mut scaled = I420Buffer::new(size.adapted_width, size.adapted_height);
                    scaled.scale_from(&buffer);
                    VideoFrame::builder(&scaled.cast_to_video_frame_buffer())
                        .set_timestamp_us(ts)
                        .set_rtp_timestamp(0)
                        .build()
                } else {
                    VideoFrame::builder(&buffer.cast_to_video_frame_buffer())
                        .set_timestamp_us(ts)
                        .set_rtp_timestamp(0)
                        .build()
                };

            source.on_frame(&video_frame);
        })?;

        Ok(Self {
            capture,
            video_source,
        })
    }

    pub(crate) fn start(&mut self) -> Result<()> {
        self.capture.start()?;
        Ok(())
    }

    pub(crate) fn video_source(&self) -> VideoTrackSource {
        self.video_source.clone()
    }
}

pub(crate) enum VideoCapturerHolder {
    Mp4(Mp4VideoCapturer),
    Fake(FakeVideoCapturer),
    #[cfg(feature = "media-device")]
    Device(VideoDeviceCapturer),
}

impl VideoCapturerHolder {
    pub(crate) fn start(&mut self) -> Result<()> {
        match self {
            VideoCapturerHolder::Mp4(_) => {
                // Mp4VideoCapturer はコンストラクタでスレッドを開始済み
            }
            VideoCapturerHolder::Fake(capturer) => capturer.start()?,
            #[cfg(feature = "media-device")]
            VideoCapturerHolder::Device(capturer) => capturer.start()?,
        }
        Ok(())
    }

    pub(crate) fn video_source(&self) -> VideoTrackSource {
        match self {
            VideoCapturerHolder::Mp4(capturer) => capturer.video_source(),
            VideoCapturerHolder::Fake(capturer) => capturer.video_source(),
            #[cfg(feature = "media-device")]
            VideoCapturerHolder::Device(capturer) => capturer.video_source(),
        }
    }
}
