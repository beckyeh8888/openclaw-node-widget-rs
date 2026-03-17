use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("config error: {0}")]
    Config(String),
    #[error("gateway error: {0}")]
    Gateway(String),
    #[error("process error: {0}")]
    Process(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tray error: {0}")]
    Tray(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
