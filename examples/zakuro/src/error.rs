use std::io;

#[derive(Debug, Clone)]
pub(crate) struct ErrorMessage {
    message: String,
}

impl ErrorMessage {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ErrorMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug)]
pub(crate) enum AppError {
    Args(noargs::Error),
    Sora(sora_sdk::Error),
    Message(ErrorMessage),
    Io(io::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Args(err) => write!(f, "{err:?}"),
            AppError::Sora(err) => write!(f, "AppError::Sora: {err}"),
            AppError::Message(err) => write!(f, "AppError::Message: {err}"),
            AppError::Io(err) => write!(f, "AppError::Io: {err}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Sora(err) => Some(err),
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

impl From<ErrorMessage> for AppError {
    fn from(err: ErrorMessage) -> Self {
        AppError::Message(err)
    }
}

impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        AppError::Io(err)
    }
}

pub(crate) type Result<T> = std::result::Result<T, AppError>;
