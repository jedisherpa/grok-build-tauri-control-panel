//! Multi-session orchestration for the Grok Build control panel.

mod error;
mod handle;
mod options;
mod registry;

pub use error::{CoreError, Result};
pub use handle::{AgentHandle, AgentHandleSnapshot, SessionMetadata};
pub use options::{AgentMode, SpawnOptions};
pub use registry::SessionRegistry;

pub use grok_events::SessionStatus;
