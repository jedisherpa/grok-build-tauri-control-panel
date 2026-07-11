//! JSON-RPC 2.0 and ACP method payloads.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC id may be string or number on the wire.
pub fn id_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    /// Wire id (string or number).
    pub id: Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    /// Must try response first (has result/error).
    Response(JsonRpcResponse),
    /// Request has method + id.
    Request(JsonRpcRequest),
    /// Notification has method, no id.
    Notification(JsonRpcNotification),
}

/// Agent → client JSON-RPC request that needs a response.
#[derive(Debug, Clone)]
pub struct IncomingAgentRequest {
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FsCapabilities {
    #[serde(rename = "readTextFile", default)]
    pub read_text_file: bool,
    #[serde(rename = "writeTextFile", default)]
    pub write_text_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientCapabilities {
    #[serde(default)]
    pub fs: FsCapabilities,
    #[serde(default)]
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    /// Required by Grok Build ACP (missing → JSON-RPC -32602 Invalid params).
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
    #[serde(rename = "clientCapabilities")]
    pub client_capabilities: ClientCapabilities,
}

impl InitializeParams {
    pub fn new(client_info: ClientInfo, client_capabilities: ClientCapabilities) -> Self {
        Self {
            protocol_version: 1,
            client_info,
            client_capabilities,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion", default)]
    pub protocol_version: Option<u32>,
    #[serde(rename = "agentCapabilities", default)]
    pub agent_capabilities: Option<Value>,
    #[serde(rename = "authMethods", default)]
    pub auth_methods: Option<Value>,
    #[serde(rename = "serverInfo", default)]
    pub server_info: Option<Value>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateParams {
    #[serde(rename = "methodId")]
    pub method_id: String,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNewParams {
    pub cwd: String,
    #[serde(rename = "mcpServers", default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPromptParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub prompt: Vec<PromptContent>,
}
