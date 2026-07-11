use thiserror::Error;

#[derive(Debug, Error)]
pub enum AcpError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("session not ready")]
    SessionNotReady,
    #[error("cancelled")]
    Cancelled,
    #[error("process exited unexpectedly")]
    ProcessExited,
    #[error("channel closed")]
    ChannelClosed,
}

pub type Result<T> = std::result::Result<T, AcpError>;
