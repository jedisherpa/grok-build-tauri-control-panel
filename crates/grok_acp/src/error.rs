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
    /// The agent needs a sign-in the user has not chosen to start (e.g. an
    /// interactive browser login). The message is plain language for the UI.
    #[error("{0}")]
    AuthRequired(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("native startup failed and cleanup remains unresolved: {reason}")]
    StartupCleanup { reason: String, process: grok_cli_wrapper::process::ProcessHandle },
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
