use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("session not found: {0}")]
    SessionNotFound(Uuid),
    #[error("max concurrent sessions reached ({0})")]
    MaxSessions(usize),
    #[error("acp error: {0}")]
    Acp(#[from] grok_acp::AcpError),
    #[error("cli error: {0}")]
    Cli(#[from] grok_cli_wrapper::CliError),
    #[error("config error: {0}")]
    Config(#[from] grok_config::ConfigError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid spawn options: {0}")]
    InvalidOptions(String),
    #[error("session not in acp mode")]
    NotAcp,
    #[error("session not in headless mode")]
    NotHeadless,
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
