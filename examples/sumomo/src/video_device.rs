use shiguredo_webrtc::VideoTrackSource;
#[cfg(feature = "libcamera")]
use sora_sdk::LibcameraVideoCapturer;
use sora_sdk::Mp4VideoCapturer;

#[cfg(feature = "media-device")]
use std::sync::atomic::{AtomicI64, Ordering};

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
                    for device in &video_device_list {
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
                    for device in &audio_device_list {
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
        let last_logged = Arc::new(AtomicI64::new(0));
        let capture = shiguredo_video_device::VideoCapture::new(config, move |frame| {
            let buffer = match convert_frame(&frame) {
                Ok(buffer) => buffer,
                Err(error) => {
                    let now = shiguredo_webrtc::time_millis();
                    if should_log(last_logged.load(Ordering::Relaxed), now, LOG_RATE_LIMIT_MS) {
                        last_logged.store(now, Ordering::Relaxed);
                        shiguredo_webrtc::rtc_log_error!(
                            "sumomo: failed to convert video frame: width={} height={} {}",
                            frame.width,
                            frame.height,
                            error
                        );
                    }
                    return;
                }
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

/// エラーログのレート制限間隔（ミリ秒）。2 秒ごとに 1 回を既定値とする。
///
/// 非対応 pixel format と変換失敗の両方で同じ間隔・同じカウンタを使う（どちらも capture 異常の兆候）。
#[cfg(feature = "media-device")]
const LOG_RATE_LIMIT_MS: i64 = 2000;

/// 前回ログから `interval` ミリ秒経過していれば true を返す。
///
/// `last_logged == 0` は未ログのセンチネルであり、常に true を返す（初回の失敗は必ずログする）。
/// 恒久的な失敗が「1 回目だけ」で抑圧されて無言破棄に戻らないようにするための特別扱い。
#[cfg(feature = "media-device")]
fn should_log(last_logged: i64, now: i64, interval: i64) -> bool {
    last_logged == 0 || now.saturating_sub(last_logged) >= interval
}

/// 変換失敗箇所。Y プレーン（packed 形式の data も含む）か UV プレーンかを区別する。
#[cfg(feature = "media-device")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plane {
    Y,
    Uv,
}

#[cfg(feature = "media-device")]
impl std::fmt::Display for Plane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Plane::Y => write!(f, "Y"),
            Plane::Uv => write!(f, "UV"),
        }
    }
}

/// フレーム変換の失敗理由。ログ出力のため Copy 可能な形にしている（PixelFormat は Copy、長さは数値）。
#[cfg(feature = "media-device")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConvertError {
    /// 非対応の pixel format（防御的な分岐。現構成の全バックエンドでは callback 到達前に除去されるが、
    /// shiguredo_video_device の mjpeg feature 有効時など到達し得る構成もあるため、除去されない前提にしない）
    Unsupported(shiguredo_video_device::PixelFormat),
    /// バッファ長不足。expected_len は失敗した検証に対応する必要長（I420 の分割境界または libyuv の必要長式）
    BufferTooShort {
        pixel_format: shiguredo_video_device::PixelFormat,
        plane: Plane,
        actual_len: usize,
        expected_len: usize,
    },
    /// 次元・ストライドが非正（I420Buffer::new が panic する前に検出する）
    InvalidDimension {
        pixel_format: shiguredo_video_device::PixelFormat,
        name: &'static str,
        value: i32,
    },
    /// 長さ計算のオーバーフロー。長さが計算できないため、失敗した次元値を保持する
    Overflow {
        pixel_format: shiguredo_video_device::PixelFormat,
        name: &'static str,
        value: i32,
    },
    /// NV12 / I420 で uv_data が None
    UvDataMissing(shiguredo_video_device::PixelFormat),
    /// libyuv 変換関数（nv12_to_i420 / yuy2_to_i420 / i420_copy）が false を返した
    LibyuvFailed(shiguredo_video_device::PixelFormat),
}

#[cfg(feature = "media-device")]
impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvertError::Unsupported(pixel_format) => {
                // Unknown は生値 (FourCC) が出るよう Display を使う（name() は "Unknown" を返すだけ）
                write!(f, "pixel format: {pixel_format}, unsupported")
            }
            ConvertError::BufferTooShort {
                pixel_format,
                plane,
                actual_len,
                expected_len,
            } => {
                write!(
                    f,
                    "pixel format: {pixel_format}, {plane} plane too short: actual={actual_len} expected={expected_len}"
                )
            }
            ConvertError::InvalidDimension {
                pixel_format,
                name,
                value,
            } => {
                write!(f, "pixel format: {pixel_format}, invalid {name}: {value}")
            }
            ConvertError::Overflow {
                pixel_format,
                name,
                value,
            } => {
                write!(f, "pixel format: {pixel_format}, {name} overflow: {value}")
            }
            ConvertError::UvDataMissing(pixel_format) => {
                write!(f, "pixel format: {pixel_format}, uv_data is missing")
            }
            ConvertError::LibyuvFailed(pixel_format) => {
                write!(f, "pixel format: {pixel_format}, libyuv conversion failed")
            }
        }
    }
}

/// libyuv.rs の `required_plane_len`（`stride * (rows - 1) + row_bytes`）と同じ必要長を返す。
///
/// shiguredo_webrtc の同関数は private のため式を自前実装する。変換関数が false を返す条件と
/// 本関数の判定が一致するよう、libyuv の実装と同じ基準を保つこと（依存バージョン更新時に確認する）。
#[cfg(feature = "media-device")]
fn required_plane_len(stride: i32, rows: i32, row_bytes: i32) -> Option<usize> {
    if stride <= 0 || rows <= 0 || row_bytes <= 0 {
        return None;
    }
    let stride = stride as usize;
    let rows = rows as usize;
    let row_bytes = row_bytes as usize;
    let last_row_offset = stride.checked_mul(rows.checked_sub(1)?)?;
    last_row_offset.checked_add(row_bytes)
}

/// libyuv.rs の `chroma_dimension`（`(value + 1) / 2`）と同じ chroma サイズを返す。
#[cfg(feature = "media-device")]
fn chroma_dimension(value: i32) -> Option<i32> {
    value.checked_add(1).map(|v| v / 2)
}

/// width / height / stride / stride_uv が正値であることを検証する。非正の値は InvalidDimension を返す。
#[cfg(feature = "media-device")]
fn validate_positive(
    pixel_format: shiguredo_video_device::PixelFormat,
    name: &'static str,
    value: i32,
) -> std::result::Result<(), ConvertError> {
    if value <= 0 {
        Err(ConvertError::InvalidDimension {
            pixel_format,
            name,
            value,
        })
    } else {
        Ok(())
    }
}

/// video frame を I420Buffer へ変換する pure helper。
///
/// 変換可能なのは NV12 / YUY2 / I420 のみで、それ以外は Unsupported で拒否する。
/// callback は FFI 境界（V4L2 / AVF / PipeWire）または Rust thread（MF）から呼ばれるため、
/// panic すると FFI 越えの abort や capture thread の永続喪失になる。本関数は検証済みの
/// 長さ・次元のみで変換を行う total な関数であり、I420Buffer::new や split_at はパニックしない。
#[cfg(feature = "media-device")]
fn convert_frame(
    frame: &shiguredo_video_device::VideoFrame<'_>,
) -> std::result::Result<shiguredo_webrtc::I420Buffer, ConvertError> {
    match frame.pixel_format {
        shiguredo_video_device::PixelFormat::Nv12 => convert_nv12(frame),
        shiguredo_video_device::PixelFormat::Yuy2 => convert_yuy2(frame),
        shiguredo_video_device::PixelFormat::I420 => convert_i420(frame),
        other => Err(ConvertError::Unsupported(other)),
    }
}

#[cfg(feature = "media-device")]
fn convert_nv12(
    frame: &shiguredo_video_device::VideoFrame<'_>,
) -> std::result::Result<shiguredo_webrtc::I420Buffer, ConvertError> {
    let pixel_format = frame.pixel_format;
    validate_positive(pixel_format, "width", frame.width)?;
    validate_positive(pixel_format, "height", frame.height)?;
    validate_positive(pixel_format, "stride", frame.stride)?;
    validate_positive(pixel_format, "stride_uv", frame.stride_uv)?;

    let Some(uv_data) = frame.uv_data else {
        return Err(ConvertError::UvDataMissing(pixel_format));
    };

    let y_expected = required_plane_len(frame.stride, frame.height, frame.width).ok_or(
        ConvertError::Overflow {
            pixel_format,
            name: "stride",
            value: frame.stride,
        },
    )?;
    if frame.data.len() < y_expected {
        return Err(ConvertError::BufferTooShort {
            pixel_format,
            plane: Plane::Y,
            actual_len: frame.data.len(),
            expected_len: y_expected,
        });
    }

    // NV12 の UV 必要長は libyuv.rs の nv12_to_i420 と同じ式（chroma_width * 2 が row_bytes）
    let chroma_width = chroma_dimension(frame.width).ok_or(ConvertError::Overflow {
        pixel_format,
        name: "width",
        value: frame.width,
    })?;
    let chroma_height = chroma_dimension(frame.height).ok_or(ConvertError::Overflow {
        pixel_format,
        name: "height",
        value: frame.height,
    })?;
    let uv_row_bytes = chroma_width.checked_mul(2).ok_or(ConvertError::Overflow {
        pixel_format,
        name: "chroma_width",
        value: chroma_width,
    })?;
    let uv_expected = required_plane_len(frame.stride_uv, chroma_height, uv_row_bytes).ok_or(
        ConvertError::Overflow {
            pixel_format,
            name: "stride_uv",
            value: frame.stride_uv,
        },
    )?;
    if uv_data.len() < uv_expected {
        return Err(ConvertError::BufferTooShort {
            pixel_format,
            plane: Plane::Uv,
            actual_len: uv_data.len(),
            expected_len: uv_expected,
        });
    }

    let mut buffer = shiguredo_webrtc::I420Buffer::new(frame.width, frame.height);
    let dst_stride_y = buffer.stride_y();
    let dst_stride_u = buffer.stride_u();
    let dst_stride_v = buffer.stride_v();
    let (dst_y, dst_u, dst_v) = buffer.planes_mut();
    if !shiguredo_webrtc::nv12_to_i420(
        frame.data,
        frame.stride,
        uv_data,
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
        return Err(ConvertError::LibyuvFailed(pixel_format));
    }
    Ok(buffer)
}

#[cfg(feature = "media-device")]
fn convert_yuy2(
    frame: &shiguredo_video_device::VideoFrame<'_>,
) -> std::result::Result<shiguredo_webrtc::I420Buffer, ConvertError> {
    let pixel_format = frame.pixel_format;
    validate_positive(pixel_format, "width", frame.width)?;
    validate_positive(pixel_format, "height", frame.height)?;
    validate_positive(pixel_format, "stride", frame.stride)?;

    // YUY2 は packed 形式のため stride_uv は使わない（0 が契約）。data 全体を 1 枚のバッファとして検証する
    let row_bytes = frame.width.checked_mul(2).ok_or(ConvertError::Overflow {
        pixel_format,
        name: "width",
        value: frame.width,
    })?;
    let expected = required_plane_len(frame.stride, frame.height, row_bytes).ok_or(
        ConvertError::Overflow {
            pixel_format,
            name: "stride",
            value: frame.stride,
        },
    )?;
    if frame.data.len() < expected {
        return Err(ConvertError::BufferTooShort {
            pixel_format,
            plane: Plane::Y,
            actual_len: frame.data.len(),
            expected_len: expected,
        });
    }

    let mut buffer = shiguredo_webrtc::I420Buffer::new(frame.width, frame.height);
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
        return Err(ConvertError::LibyuvFailed(pixel_format));
    }
    Ok(buffer)
}

#[cfg(feature = "media-device")]
fn convert_i420(
    frame: &shiguredo_video_device::VideoFrame<'_>,
) -> std::result::Result<shiguredo_webrtc::I420Buffer, ConvertError> {
    let pixel_format = frame.pixel_format;
    validate_positive(pixel_format, "width", frame.width)?;
    validate_positive(pixel_format, "height", frame.height)?;
    validate_positive(pixel_format, "stride", frame.stride)?;
    validate_positive(pixel_format, "stride_uv", frame.stride_uv)?;

    let Some(uv_data) = frame.uv_data else {
        return Err(ConvertError::UvDataMissing(pixel_format));
    };

    let y_expected = required_plane_len(frame.stride, frame.height, frame.width).ok_or(
        ConvertError::Overflow {
            pixel_format,
            name: "stride",
            value: frame.stride,
        },
    )?;
    if frame.data.len() < y_expected {
        return Err(ConvertError::BufferTooShort {
            pixel_format,
            plane: Plane::Y,
            actual_len: frame.data.len(),
            expected_len: y_expected,
        });
    }

    // shiguredo_video_device の仕様上、frame.uv_data は U/V を連結した UV バッファで、
    // U / V の分割境界は stride_uv * ceil(height / 2) の位置にある。
    let uv_rows = (frame.height as usize).div_ceil(2);
    let uv_plane_len =
        (frame.stride_uv as usize)
            .checked_mul(uv_rows)
            .ok_or(ConvertError::Overflow {
                pixel_format,
                name: "stride_uv",
                value: frame.stride_uv,
            })?;
    let uv_total_len = uv_plane_len.checked_mul(2).ok_or(ConvertError::Overflow {
        pixel_format,
        name: "stride_uv",
        value: frame.stride_uv,
    })?;
    if uv_data.len() < uv_total_len {
        return Err(ConvertError::BufferTooShort {
            pixel_format,
            plane: Plane::Uv,
            actual_len: uv_data.len(),
            expected_len: uv_total_len,
        });
    }

    let (src_u, src_v) = uv_data.split_at(uv_plane_len);

    // libyuv.rs の i420_copy と同じ必要長式でも U / V を検証する
    let chroma_width = chroma_dimension(frame.width).ok_or(ConvertError::Overflow {
        pixel_format,
        name: "width",
        value: frame.width,
    })?;
    let chroma_height = chroma_dimension(frame.height).ok_or(ConvertError::Overflow {
        pixel_format,
        name: "height",
        value: frame.height,
    })?;
    let uv_expected = required_plane_len(frame.stride_uv, chroma_height, chroma_width).ok_or(
        ConvertError::Overflow {
            pixel_format,
            name: "stride_uv",
            value: frame.stride_uv,
        },
    )?;
    if src_u.len() < uv_expected {
        return Err(ConvertError::BufferTooShort {
            pixel_format,
            plane: Plane::Uv,
            actual_len: src_u.len(),
            expected_len: uv_expected,
        });
    }

    let mut buffer = shiguredo_webrtc::I420Buffer::new(frame.width, frame.height);
    let dst_stride_y = buffer.stride_y();
    let dst_stride_u = buffer.stride_u();
    let dst_stride_v = buffer.stride_v();
    let (dst_y, dst_u, dst_v) = buffer.planes_mut();
    if !shiguredo_webrtc::i420_copy(
        frame.data,
        frame.stride,
        src_u,
        frame.stride_uv,
        src_v,
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
        return Err(ConvertError::LibyuvFailed(pixel_format));
    }
    Ok(buffer)
}

pub(crate) enum VideoCapturerHolder {
    Mp4(Mp4VideoCapturer),
    Fake(FakeVideoCapturer),
    #[cfg(feature = "libcamera")]
    Libcamera(LibcameraVideoCapturer),
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
            #[cfg(feature = "libcamera")]
            VideoCapturerHolder::Libcamera(capturer) => capturer.start()?,
            #[cfg(feature = "media-device")]
            VideoCapturerHolder::Device(capturer) => capturer.start()?,
        }
        Ok(())
    }

    pub(crate) fn video_source(&self) -> VideoTrackSource {
        match self {
            VideoCapturerHolder::Mp4(capturer) => capturer.video_source(),
            VideoCapturerHolder::Fake(capturer) => capturer.video_source(),
            #[cfg(feature = "libcamera")]
            VideoCapturerHolder::Libcamera(capturer) => capturer.video_source(),
            #[cfg(feature = "media-device")]
            VideoCapturerHolder::Device(capturer) => capturer.video_source(),
        }
    }
}

#[cfg(all(test, feature = "media-device"))]
mod tests {
    use shiguredo_webrtc::I420Buffer;

    use super::*;

    fn make_frame<'a>(
        pixel_format: shiguredo_video_device::PixelFormat,
        data: &'a [u8],
        uv_data: Option<&'a [u8]>,
        width: i32,
        height: i32,
        stride: i32,
        stride_uv: i32,
    ) -> shiguredo_video_device::VideoFrame<'a> {
        shiguredo_video_device::VideoFrame {
            data,
            uv_data,
            width,
            height,
            stride,
            stride_uv,
            pixel_format,
            timestamp_us: 0,
            pixel_buffer: None,
        }
    }

    fn assert_plane_content(plane: &[u8], stride: usize, rows: usize, cols: usize, expected: u8) {
        for row in 0..rows {
            for col in 0..cols {
                assert_eq!(
                    plane[row * stride + col],
                    expected,
                    "plane[{row}][{col}] の値が期待と異なる"
                );
            }
        }
    }

    fn assert_buffer_content(
        buffer: &mut I420Buffer,
        expected_y: u8,
        expected_u: u8,
        expected_v: u8,
    ) {
        let width = buffer.width() as usize;
        let height = buffer.height() as usize;
        let chroma_width = buffer.chroma_width() as usize;
        let chroma_height = buffer.chroma_height() as usize;
        let stride_y = buffer.stride_y() as usize;
        let stride_u = buffer.stride_u() as usize;
        let stride_v = buffer.stride_v() as usize;
        let (y, u, v) = buffer.planes_mut();
        assert_plane_content(y, stride_y, height, width, expected_y);
        assert_plane_content(u, stride_u, chroma_height, chroma_width, expected_u);
        assert_plane_content(v, stride_v, chroma_height, chroma_width, expected_v);
    }

    #[test]
    fn nv12_converts_yuv_planes() {
        // 4x2、stride=4、stride_uv=4。UV は U/V が交互に並ぶインターリーブ
        let width = 4;
        let height = 2;
        let stride = 4;
        let stride_uv = 4;
        let y_size = (stride * height) as usize;
        let uv_size = (stride_uv * ((height + 1) / 2)) as usize;

        let data = vec![0x11; y_size];
        // 偶数の index が U、奇数の index が V。U/V の反転を検出するため別の定数にする
        let uv: Vec<u8> = (0..uv_size)
            .map(|i| if i % 2 == 0 { 0x2A } else { 0x5A })
            .collect();

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::Nv12,
            &data,
            Some(&uv),
            width,
            height,
            stride,
            stride_uv,
        );

        let mut buffer = convert_frame(&frame).expect("NV12 の変換は成功するはず");
        assert_eq!(buffer.width(), width);
        assert_eq!(buffer.height(), height);
        assert_buffer_content(&mut buffer, 0x11, 0x2A, 0x5A);
    }

    #[test]
    fn yuy2_converts_yuv_planes() {
        // 2x1、stride=4、stride_uv=0（packed 形式の契約）。data は [Y0, U0, Y1, V0]
        let width = 2;
        let height = 1;
        let stride = 4;
        let data = [0x10, 0x20, 0x10, 0x21];

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::Yuy2,
            &data,
            None,
            width,
            height,
            stride,
            0,
        );

        let mut buffer = convert_frame(&frame).expect("YUY2 の変換は成功するはず");
        assert_eq!(buffer.width(), width);
        assert_eq!(buffer.height(), height);
        assert_buffer_content(&mut buffer, 0x10, 0x20, 0x21);
    }

    #[test]
    fn i420_converts_yuv_planes() {
        // 4x2、stride=4、stride_uv=2。uv_data は U 連結 V の連結バッファ
        let width = 4;
        let height = 2;
        let stride = 4;
        let stride_uv = 2;
        let y_size = (stride * height) as usize;
        let uv_plane_len = (stride_uv * ((height + 1) / 2)) as usize;

        let data = vec![0x11; y_size];
        let mut uv = vec![0x00; uv_plane_len * 2];
        uv[..uv_plane_len].fill(0x2A);
        uv[uv_plane_len..].fill(0x5A);

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::I420,
            &data,
            Some(&uv),
            width,
            height,
            stride,
            stride_uv,
        );

        let mut buffer = convert_frame(&frame).expect("I420 の変換は成功するはず");
        assert_eq!(buffer.width(), width);
        assert_eq!(buffer.height(), height);
        assert_buffer_content(&mut buffer, 0x11, 0x2A, 0x5A);
    }

    #[test]
    fn i420_odd_dimensions_are_convertible() {
        // 奇数幅・奇数高さの 5x3。stride_uv=3 は ceil(5/2)。chroma の切り上げと分割式の一致を検証する
        let width = 5;
        let height = 3;
        let stride = 5;
        let stride_uv = 3;
        let y_size = (stride * height) as usize;
        let uv_plane_len = (stride_uv * ((height + 1) / 2)) as usize;

        let data = vec![0x11; y_size];
        let mut uv = vec![0x00; uv_plane_len * 2];
        uv[..uv_plane_len].fill(0x2A);
        uv[uv_plane_len..].fill(0x5A);

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::I420,
            &data,
            Some(&uv),
            width,
            height,
            stride,
            stride_uv,
        );

        let mut buffer = convert_frame(&frame).expect("奇数幅・奇数高さの I420 は変換できるはず");
        assert_eq!(buffer.width(), width);
        assert_eq!(buffer.height(), height);
        assert_buffer_content(&mut buffer, 0x11, 0x2A, 0x5A);
    }

    #[test]
    fn nv12_odd_width_is_rejected() {
        // 奇数幅 NV12 は stride_uv == width の正規化では UV が libyuv の必要長に届かず変換失敗になる既知のケース
        let width = 3;
        let height = 2;
        let stride = 3;
        let stride_uv = 3;
        let y_size = (stride * height) as usize;
        let uv_size = (stride_uv * ((height + 1) / 2)) as usize;

        let data = vec![0x00; y_size];
        let uv = vec![0x00; uv_size];

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::Nv12,
            &data,
            Some(&uv),
            width,
            height,
            stride,
            stride_uv,
        );

        // 期待長は libyuv の必要長式: stride_uv * (chroma_height - 1) + chroma_width * 2 = 3*0 + 2*2 = 4
        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::BufferTooShort {
                pixel_format: shiguredo_video_device::PixelFormat::Nv12,
                plane: Plane::Uv,
                actual_len: 3,
                expected_len: 4,
            })
        ));
    }

    #[test]
    fn unsupported_formats_are_rejected() {
        for pixel_format in [
            shiguredo_video_device::PixelFormat::Mjpeg,
            shiguredo_video_device::PixelFormat::Unknown(0x1234),
        ] {
            let frame = make_frame(pixel_format, &[], None, 640, 480, 640, 0);
            assert!(
                matches!(
                    convert_frame(&frame),
                    Err(ConvertError::Unsupported(p)) if p == pixel_format
                ),
                "非対応 format は Unsupported で拒否されるはず"
            );
        }
    }

    #[test]
    fn nv12_y_plane_too_short_is_rejected() {
        let width = 4;
        let height = 2;
        let stride = 4;
        let stride_uv = 4;
        let data = vec![0x00; 7];
        let uv = vec![0x00; 4];

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::Nv12,
            &data,
            Some(&uv),
            width,
            height,
            stride,
            stride_uv,
        );

        // 期待長は libyuv の必要長式: stride * (height - 1) + width = 4*1 + 4 = 8
        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::BufferTooShort {
                pixel_format: shiguredo_video_device::PixelFormat::Nv12,
                plane: Plane::Y,
                actual_len: 7,
                expected_len: 8,
            })
        ));
    }

    #[test]
    fn nv12_uv_plane_too_short_is_rejected() {
        let width = 4;
        let height = 2;
        let stride = 4;
        let stride_uv = 4;
        let data = vec![0x00; 8];
        let uv = vec![0x00; 3];

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::Nv12,
            &data,
            Some(&uv),
            width,
            height,
            stride,
            stride_uv,
        );

        // 期待長は libyuv の必要長式: stride_uv * (chroma_height - 1) + chroma_width * 2 = 4*0 + 2*2 = 4
        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::BufferTooShort {
                pixel_format: shiguredo_video_device::PixelFormat::Nv12,
                plane: Plane::Uv,
                actual_len: 3,
                expected_len: 4,
            })
        ));
    }

    #[test]
    fn yuy2_data_too_short_is_rejected() {
        let width = 2;
        let height = 1;
        let stride = 4;
        let data = [0x10, 0x20, 0x11];

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::Yuy2,
            &data,
            None,
            width,
            height,
            stride,
            0,
        );

        // 期待長は libyuv の必要長式: stride * (height - 1) + width * 2 = 4*0 + 4 = 4
        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::BufferTooShort {
                pixel_format: shiguredo_video_device::PixelFormat::Yuy2,
                plane: Plane::Y,
                actual_len: 3,
                expected_len: 4,
            })
        ));
    }

    #[test]
    fn i420_uv_data_too_short_for_split_is_rejected() {
        let width = 4;
        let height = 2;
        let stride = 4;
        let stride_uv = 2;
        let data = vec![0x00; 8];
        let uv = vec![0x00; 3];

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::I420,
            &data,
            Some(&uv),
            width,
            height,
            stride,
            stride_uv,
        );

        // 期待長は分割境界: 2 * stride_uv * ceil(height / 2) = 2 * 2 * 1 = 4
        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::BufferTooShort {
                pixel_format: shiguredo_video_device::PixelFormat::I420,
                plane: Plane::Uv,
                actual_len: 3,
                expected_len: 4,
            })
        ));
    }

    #[test]
    fn i420_uv_half_too_short_for_libyuv_is_rejected() {
        // stride_uv が chroma_width 未満のとき、分割境界は満たすが libyuv の必要長には届かないケース
        let width = 5;
        let height = 3;
        let stride = 5;
        let stride_uv = 2;
        let data = vec![0x00; 15];
        let uv = vec![0x00; 8];

        let frame = make_frame(
            shiguredo_video_device::PixelFormat::I420,
            &data,
            Some(&uv),
            width,
            height,
            stride,
            stride_uv,
        );

        // 期待長は libyuv の必要長式: stride_uv * (chroma_height - 1) + chroma_width = 2*1 + 3 = 5
        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::BufferTooShort {
                pixel_format: shiguredo_video_device::PixelFormat::I420,
                plane: Plane::Uv,
                actual_len: 4,
                expected_len: 5,
            })
        ));
    }

    #[test]
    fn nv12_uv_data_missing_is_rejected() {
        let frame = make_frame(
            shiguredo_video_device::PixelFormat::Nv12,
            &[0x00; 8],
            None,
            4,
            2,
            4,
            4,
        );

        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::UvDataMissing(
                shiguredo_video_device::PixelFormat::Nv12
            ))
        ));
    }

    #[test]
    fn i420_uv_data_missing_is_rejected() {
        let frame = make_frame(
            shiguredo_video_device::PixelFormat::I420,
            &[0x00; 8],
            None,
            4,
            2,
            4,
            2,
        );

        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::UvDataMissing(
                shiguredo_video_device::PixelFormat::I420
            ))
        ));
    }

    #[test]
    fn invalid_dimensions_are_rejected() {
        let pixel_format = shiguredo_video_device::PixelFormat::Nv12;
        for (name, width, height, stride, stride_uv) in [
            ("width", 0, 2, 4, 4),
            ("height", 4, 0, 4, 4),
            ("stride", 4, 2, 0, 4),
            ("stride_uv", 4, 2, 4, 0),
        ] {
            let data = vec![0x00; 8];
            let uv = vec![0x00; 4];
            let frame = make_frame(
                pixel_format,
                &data,
                Some(&uv),
                width,
                height,
                stride,
                stride_uv,
            );
            let value = match name {
                "width" => width,
                "height" => height,
                "stride" => stride,
                "stride_uv" => stride_uv,
                _ => unreachable!(),
            };
            assert!(
                matches!(
                    convert_frame(&frame),
                    Err(ConvertError::InvalidDimension {
                        pixel_format: p,
                        name: n,
                        value: v,
                    }) if p == pixel_format && n == name && v == value
                ),
                "{name} の非正値は InvalidDimension で拒否されるはず"
            );
        }
    }

    #[test]
    fn overflow_is_rejected() {
        // YUY2 は row_bytes = width * 2 の計算で i32 オーバーフローする（width = i32::MAX）。
        // row_bytes の Overflow 判定はバッファ長検証より先に走るため、データが空のままで
        // Overflow が返ることを確認する。
        let frame = make_frame(
            shiguredo_video_device::PixelFormat::Yuy2,
            &[],
            None,
            i32::MAX,
            1,
            i32::MAX,
            0,
        );

        assert!(matches!(
            convert_frame(&frame),
            Err(ConvertError::Overflow {
                pixel_format: shiguredo_video_device::PixelFormat::Yuy2,
                name: "width",
                value: i32::MAX,
            })
        ));
    }

    #[test]
    fn should_log_first_failure_is_always_logged() {
        // last_logged == 0 は未ログのセンチネルであり、どの時刻でも出力する
        assert!(should_log(0, 1, LOG_RATE_LIMIT_MS));
        assert!(should_log(0, LOG_RATE_LIMIT_MS - 1, LOG_RATE_LIMIT_MS));
    }

    #[test]
    fn should_log_suppresses_within_interval() {
        let last_logged = 1_000;
        assert!(!should_log(last_logged, 2_999, LOG_RATE_LIMIT_MS));
    }

    #[test]
    fn should_log_outputs_again_after_interval() {
        let last_logged = 1_000;
        // 間隔ちょうど（now - last == interval）で再出力する
        assert!(should_log(last_logged, 3_000, LOG_RATE_LIMIT_MS));
    }
}
