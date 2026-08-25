#[derive(Debug)]
pub(crate) enum AppError {
    Args(noargs::Error),
    Sora(sora_sdk::Error),
    #[cfg(feature = "media-device")]
    Video(shiguredo_video_device::Error),
    #[cfg(feature = "media-device")]
    Audio(shiguredo_audio_device::Error),
    #[cfg(feature = "raw-player")]
    RawPlayer(raw_player::Error),
    Io(std::io::Error),
    Pem(rustls_pki_types::pem::Error),
    /// ANSI レンダラーの描画処理が失敗した。
    Ansi(String),
    /// 接続終了処理が 10 秒以内に完了しなかった。
    ConnectionShutdownTimeout,
    /// raw-player のワーカースレッドまたは run タスクが panic した。
    WorkerPanic,
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Args(err) => write!(f, "{err:?}"),
            AppError::Sora(err) => write!(f, "AppError::Sora: {err}"),
            #[cfg(feature = "media-device")]
            AppError::Video(err) => write!(f, "AppError::Video: {err}"),
            #[cfg(feature = "media-device")]
            AppError::Audio(err) => write!(f, "AppError::Audio: {err}"),
            #[cfg(feature = "raw-player")]
            AppError::RawPlayer(err) => write!(f, "AppError::RawPlayer: {err}"),
            AppError::Io(err) => write!(f, "AppError::Io: {err}"),
            AppError::Pem(err) => write!(f, "AppError::Pem: {err}"),
            AppError::Ansi(message) => write!(f, "AppError::Ansi: {message}"),
            AppError::ConnectionShutdownTimeout => {
                write!(f, "AppError::ConnectionShutdownTimeout")
            }
            AppError::WorkerPanic => write!(f, "AppError::WorkerPanic"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Sora(err) => Some(err),
            AppError::Io(err) => Some(err),
            AppError::Pem(err) => Some(err),
            _ => None,
        }
    }
}

impl From<noargs::Error> for AppError {
    fn from(err: noargs::Error) -> Self {
        AppError::Args(err)
    }
}

impl From<sora_sdk::Error> for AppError {
    fn from(err: sora_sdk::Error) -> Self {
        AppError::Sora(err)
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err)
    }
}

impl From<rustls_pki_types::pem::Error> for AppError {
    fn from(err: rustls_pki_types::pem::Error) -> Self {
        AppError::Pem(err)
    }
}

#[cfg(feature = "raw-player")]
impl From<raw_player::Error> for AppError {
    fn from(err: raw_player::Error) -> Self {
        AppError::RawPlayer(err)
    }
}

#[cfg(feature = "media-device")]
impl From<shiguredo_video_device::Error> for AppError {
    fn from(err: shiguredo_video_device::Error) -> Self {
        AppError::Video(err)
    }
}

#[cfg(feature = "media-device")]
impl From<shiguredo_audio_device::Error> for AppError {
    fn from(err: shiguredo_audio_device::Error) -> Self {
        AppError::Audio(err)
    }
}

pub(crate) type Result<T> = std::result::Result<T, AppError>;
