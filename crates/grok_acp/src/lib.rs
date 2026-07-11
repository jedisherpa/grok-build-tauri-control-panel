//! ACP client for `grok agent stdio` (JSON-RPC 2.0 over newline-delimited stdio).
//!
//! Preferred path for interactive long-lived sessions (vs headless `-p`).

mod client;
mod error;
mod messages;
mod transport;

pub use client::{AcpClient, AcpClientConfig, SpawnOptions as AcpSpawnOptions};
pub use error::{AcpError, Result};
pub use messages::{
    AuthenticateParams, ClientCapabilities, ClientInfo, IncomingAgentRequest, InitializeParams,
    InitializeResult, JsonRpcError, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, PromptContent, SessionNewParams, SessionPromptParams,
};
pub use transport::NdjsonTransport;
