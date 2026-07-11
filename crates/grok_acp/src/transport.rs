//! Newline-delimited JSON transport over process stdio.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::error::{AcpError, Result};
use crate::messages::{JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

pub struct NdjsonTransport {
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    notification_tx: tokio::sync::mpsc::UnboundedSender<JsonRpcNotification>,
}

impl NdjsonTransport {
    pub fn new(
        stdin: ChildStdin,
        stdout: ChildStdout,
        notification_tx: tokio::sync::mpsc::UnboundedSender<JsonRpcNotification>,
    ) -> Arc<Self> {
        let transport = Arc::new(Self {
            stdin: Mutex::new(stdin),
            pending: Arc::new(Mutex::new(HashMap::new())),
            notification_tx,
        });

        let reader_self = transport.clone();
        tokio::spawn(async move {
            if let Err(e) = reader_self.read_loop(stdout).await {
                warn!(error = %e, "ACP transport read loop ended");
            }
        });

        transport
    }

    async fn read_loop(self: Arc<Self>, stdout: ChildStdout) -> Result<()> {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Err(AcpError::ProcessExited);
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            debug!(payload = %trimmed, "acp recv");
            match serde_json::from_str::<JsonRpcMessage>(trimmed) {
                Ok(JsonRpcMessage::Response(resp)) => {
                    let id_key = resp
                        .id
                        .as_ref()
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default();
                    let mut pending = self.pending.lock().await;
                    if let Some(tx) = pending.remove(&id_key) {
                        let _ = tx.send(resp);
                    } else {
                        debug!(id = %id_key, "no pending request for response");
                    }
                }
                Ok(JsonRpcMessage::Notification(n)) => {
                    let _ = self.notification_tx.send(n);
                }
                Ok(JsonRpcMessage::Request(req)) => {
                    // Server-initiated requests (e.g. fs/read) — surface as notifications-like events
                    let _ = self.notification_tx.send(JsonRpcNotification {
                        jsonrpc: "2.0".into(),
                        method: format!("client/request/{}", req.method),
                        params: req.params,
                    });
                }
                Err(e) => {
                    warn!(error = %e, line = %trimmed, "failed to parse ACP line");
                }
            }
        }
    }

    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = Uuid::new_v4().to_string();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id.clone(), tx);
        }

        let line = serde_json::to_string(&req)? + "\n";
        debug!(method, %id, "acp send");
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }

        let resp = rx.await.map_err(|_| AcpError::ChannelClosed)?;
        if let Some(err) = resp.error {
            return Err(AcpError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        let n = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&n)? + "\n";
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }
}
