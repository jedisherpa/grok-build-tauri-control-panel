// Deep Code Sketch: grok_acp/src/acp_client.rs
// Full ACP client for grok agent stdio (JSON-RPC 2.0 over newline-delimited stdio)
// Based on agentclientprotocol.com spec and report flow

use std::process::Stdio;
use tokio::process::{Command, ChildStdin, ChildStdout};
use tokio::io::{AsyncBufReadExt, BufReader, AsyncWriteExt};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use anyhow::{Result, Context};
use uuid::Uuid;

pub struct AcpClient {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    session_id: Option<String>,
    // Capabilities cache, etc.
}

impl AcpClient {
    pub async fn new(grok_path: &str, cwd: &str, opts: &crate::core::SpawnOptions) -> Result<Self> {
        let mut child = Command::new(grok_path)
            .args(["agent", "stdio"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("XAI_API_KEY", std::env::var("XAI_API_KEY").unwrap_or_default())
            .spawn()
            .context("Failed to spawn grok agent stdio")?;

        let stdin = child.stdin.take().context("No stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("No stdout")?);

        let mut client = Self {
            stdin,
            stdout,
            session_id: None,
        };

        // Core flow from report
        client.initialize().await?;
        client.authenticate().await?;
        client.session_new(cwd, &opts).await?;

        Ok(client)
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = Uuid::new_v4().to_string();
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let line = serde_json::to_string(&request)? + "\n";
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        // Read response (simplified; in prod handle notifications separately)
        let mut line = String::new();
        self.stdout.read_line(&mut line).await?;
        let response: Value = serde_json::from_str(&line)?;
        Ok(response)
    }

    async fn initialize(&mut self) -> Result<()> {
        let params = json!({
            "clientInfo": {"name": "GrokBuildTauriControlPanel", "version": "1.0"},
            "clientCapabilities": {"fs": {"readTextFile": true, "writeTextFile": true}, "terminal": true}
        });
        let _resp = self.send_request("initialize", params).await?;
        // Cache agentCapabilities from resp
        Ok(())
    }

    async fn authenticate(&mut self) -> Result<()> {
        // From report: methodId from advertised, often cached_token or xai.api_key
        let params = json!({
            "methodId": "xai.api_key", // or cached
            "_meta": {"headless": false}
        });
        let _resp = self.send_request("authenticate", params).await?;
        Ok(())
    }

    async fn session_new(&mut self, cwd: &str, opts: &crate::core::SpawnOptions) -> Result<()> {
        let params = json!({
            "cwd": cwd,
            "mcpServers": [], // From config
            "rules": opts.rules,
            // model, effort, etc.
        });
        let resp = self.send_request("session/new", params).await?;
        if let Some(sid) = resp.get("sessionId").and_then(|v| v.as_str()) {
            self.session_id = Some(sid.to_string());
        }
        Ok(())
    }

    pub async fn send_prompt(&mut self, prompt: &str) -> Result<()> {
        let params = json!({
            "sessionId": self.session_id,
            "prompt": [{"type": "text", "text": prompt}]
        });
        let _resp = self.send_request("session/prompt", params).await?;
        Ok(())
    }

    pub async fn run_event_loop(&self, event_tx: broadcast::Sender<Value>) -> Result<()> {
        // In real: Spawn task to read stdout continuously for notifications
        // Parse "session/update", tool calls (already resolved), plan updates, etc.
        // Emit to event_tx for UI / registry
        // Handle plan mode approvals via client responses
        loop {
            // Simplified read loop
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            // Actual: Parse NDJSON, match on "sessionUpdate": "tool_call", "plan", etc.
            // event_tx.send(json!({"type": "tool_call", "data": ...})).ok();
        }
    }

    pub async fn cancel(&self) -> Result<()> {
        // session/cancel
        Ok(())
    }

    // Additional: set_mode for plan/always-approve, set_config_option, etc.
}

