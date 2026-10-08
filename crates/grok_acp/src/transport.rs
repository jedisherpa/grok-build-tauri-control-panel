//! Newline-delimited JSON transport over process stdio.

use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::error::{AcpError, Result};
use grok_cli_wrapper::process::DrainReporter;
use crate::messages::{
    id_key, IncomingAgentRequest, JsonRpcError, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse,
};

pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Never allocate an unbounded newline-free frame from an untrusted runner.
/// An oversize or invalid UTF-8 record ends the transport with an explicit
/// drain error; its supervisor is responsible for observed process cleanup.
pub(crate) async fn bounded_line<R: AsyncBufRead + Unpin>(reader: &mut R, bytes: &mut Vec<u8>) -> std::io::Result<usize> {
    bytes.clear();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() { return Ok(bytes.len()); }
        let count = available.iter().position(|byte| *byte == b'\n').map_or(available.len(), |index| index + 1);
        if bytes.len() + count > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "native ACP frame exceeds 1 MiB limit"));
        }
        let complete = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if complete { return Ok(bytes.len()); }
    }
}

pub(crate) async fn discard_to_eof<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt;
    let mut buffer = [0u8; 8192];
    while reader.read(&mut buffer).await? != 0 {}
    Ok(())
}

/// Local fences share the notification FIFO but cannot be supplied by the agent.
pub enum NotificationEvent {
    Notification(JsonRpcNotification),
    ScopedNotification(u64,JsonRpcNotification),
    Fence(oneshot::Sender<()>),
}

pub struct NdjsonTransport {
    turn_epoch: Option<Arc<AtomicU64>>,
    stdin: Mutex<Option<ChildStdin>>,
    poisoned: AtomicBool,
    write_timeout: Duration,
    drain_reporter: Mutex<Option<DrainReporter>>,
    stdout_eof_observed: AtomicBool,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    notification_tx: tokio::sync::mpsc::UnboundedSender<NotificationEvent>,
    agent_request_tx: tokio::sync::mpsc::UnboundedSender<IncomingAgentRequest>,
}

impl NdjsonTransport {
    pub fn new(
        stdin: ChildStdin,
        stdout: ChildStdout,
        notification_tx: tokio::sync::mpsc::UnboundedSender<NotificationEvent>,
        agent_request_tx: tokio::sync::mpsc::UnboundedSender<IncomingAgentRequest>,
    ) -> Arc<Self> {
        Self::new_with_drain(stdin, stdout, notification_tx, agent_request_tx, None)
    }

    pub(crate) fn new_with_drain(
        stdin: ChildStdin,
        stdout: ChildStdout,
        notification_tx: tokio::sync::mpsc::UnboundedSender<NotificationEvent>,
        agent_request_tx: tokio::sync::mpsc::UnboundedSender<IncomingAgentRequest>,
        drain_reporter: Option<DrainReporter>,
    ) -> Arc<Self> {
        Self::new_scoped(stdin,stdout,notification_tx,agent_request_tx,drain_reporter,None)
    }
    pub(crate) fn new_scoped(stdin:ChildStdin,stdout:ChildStdout,notification_tx:tokio::sync::mpsc::UnboundedSender<NotificationEvent>,agent_request_tx:tokio::sync::mpsc::UnboundedSender<IncomingAgentRequest>,drain_reporter:Option<DrainReporter>,turn_epoch:Option<Arc<AtomicU64>>)->Arc<Self> {
        let transport = Arc::new(Self {
            turn_epoch,
            stdin: Mutex::new(Some(stdin)),
            poisoned: AtomicBool::new(false),
            write_timeout: Duration::from_secs(5),
            drain_reporter: Mutex::new(drain_reporter),
            stdout_eof_observed: AtomicBool::new(false),
            pending: Arc::new(Mutex::new(HashMap::new())),
            notification_tx,
            agent_request_tx,
        });

        let reader_self = transport.clone();
        tokio::spawn(async move {
            if let Err(e) = reader_self.read_loop(stdout).await {
                warn!(error = %e, "ACP transport read loop ended");
            }
        });

        transport
    }

    fn scoped_notification(&self,notification:JsonRpcNotification)->NotificationEvent {
        match &self.turn_epoch {Some(epoch)=>NotificationEvent::ScopedNotification(epoch.load(Ordering::Acquire),notification),None=>NotificationEvent::Notification(notification)}
    }

    async fn read_loop(self: Arc<Self>, stdout: ChildStdout) -> Result<()> {
        let result = self.read_loop_inner(stdout).await;
        if let Some(reporter) = self.drain_reporter.lock().await.take() {
            reporter.complete(if self.stdout_eof_observed.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(result.as_ref().err().map(ToString::to_string).unwrap_or_else(|| "stdout ended without observed EOF".into()))
            });
        }
        // Fail every pending waiter immediately — otherwise callers block for
        // their full request timeout (up to minutes) after the process dies.
        let mut pending = self.pending.lock().await;
        for (_, tx) in pending.drain() {
            drop(tx); // closes the oneshot → callers get ChannelClosed now
        }
        result
    }

    async fn read_loop_inner(&self, stdout: ChildStdout) -> Result<()> {
        let mut reader = BufReader::new(stdout);
        let mut bytes = Vec::new();
        loop {
            let n = match bounded_line(&mut reader, &mut bytes).await {
                Ok(count) => count,
                Err(error) => return self.settle_read_error(&mut reader, error).await,
            };
            if n == 0 {
                self.stdout_eof_observed.store(true, Ordering::Release);
                return Err(AcpError::ProcessExited);
            }
            let line = match std::str::from_utf8(&bytes) {
                Ok(line) => line,
                Err(error) => return self.settle_read_error(&mut reader,
                    std::io::Error::new(std::io::ErrorKind::InvalidData, error)).await,
            };
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
                        .map(id_key)
                        .unwrap_or_default();
                    let mut pending = self.pending.lock().await;
                    if let Some(tx) = pending.remove(&id_key) {
                        let _ = tx.send(resp);
                    } else {
                        debug!(id = %id_key, "no pending request for response");
                    }
                }
                Ok(JsonRpcMessage::Notification(n)) => {
                    let _ = self.notification_tx.send(self.scoped_notification(n));
                }
                Ok(JsonRpcMessage::Request(req)) => {
                    // Agent → client request (fs/*, session/request_permission, …).
                    // MUST be answered or the agent turn hangs forever.
                    let _ = self.agent_request_tx.send(IncomingAgentRequest {
                        host_epoch:self.turn_epoch.as_ref().map(|epoch|epoch.load(Ordering::Acquire)),
                        id: req.id,
                        method: req.method,
                        params: req.params,
                    });
                }
                Err(e) => {
                    // Try looser parse: notification-shaped with extra fields.
                    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                        if v.get("method").is_some() && v.get("id").is_some() {
                            let _ = self.agent_request_tx.send(IncomingAgentRequest {
                                host_epoch:self.turn_epoch.as_ref().map(|epoch|epoch.load(Ordering::Acquire)),
                                id: v.get("id").cloned().unwrap_or(Value::Null),
                                method: v
                                    .get("method")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                params: v.get("params").cloned(),
                            });
                            continue;
                        }
                        if v.get("method").is_some() && v.get("id").is_none() {
                            let _ = self.notification_tx.send(self.scoped_notification(JsonRpcNotification {
                                jsonrpc: "2.0".into(),
                                method: v
                                    .get("method")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                params: v.get("params").cloned(),
                            }));
                            continue;
                        }
                    }
                    warn!(error = %e, line = %trimmed, "failed to parse ACP line");
                }
            }
        }
    }

    async fn settle_read_error<R: tokio::io::AsyncRead + Unpin>(&self, reader: &mut R, error: std::io::Error) -> Result<()> {
        if let Some(reporter) = self.drain_reporter.lock().await.as_ref() {
            reporter.fail(error.to_string());
        } else {
            // Standalone/fake transports do not own a native supervisor. There
            // is no external EOF contract to recover, so fail immediately.
            return Err(error.into());
        }
        if discard_to_eof(reader).await.is_ok() {
            self.stdout_eof_observed.store(true, Ordering::Release);
        }
        Err(error.into())
    }

    /// After a response arrives, every earlier wire notification is already in
    /// this FIFO. Completion must wait until its consumer has handled them all.
    /// The caller applies a bounded timeout; a closed consumer fails explicitly.
    pub async fn drain_notifications(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.notification_tx.send(NotificationEvent::Fence(tx))
            .map_err(|_| AcpError::ChannelClosed)?;
        rx.await.map_err(|_| AcpError::ChannelClosed)
    }

    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let rx = self.send_request(method, params).await?;
        let resp = rx.await.map_err(|_| AcpError::ChannelClosed)?;
        Self::unwrap_response(resp)
    }

    /// Send a request and return the oneshot receiver (caller applies timeout policy).
    pub async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<oneshot::Receiver<JsonRpcResponse>> {
        let id = Value::String(Uuid::new_v4().to_string());
        let id_str = id_key(&id);
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: method.to_string(),
            params,
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id_str.clone(), tx);
        }

        let line = serde_json::to_string(&req)? + "\n";
        debug!(method, %id_str, "acp send");
        let write_res = self.write_line(&line).await;
        if let Err(e) = write_res {
            // Failed write leaks the pending entry — remove it so a later
            // response for a reused id can't match, and callers fail fast.
            self.pending.lock().await.remove(&id_str);
            return Err(e);
        }
        Ok(rx)
    }

    pub async fn send_response(&self, id: Value, result: Value) -> Result<()> {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: Some(id),
            result: Some(result),
            error: None,
        };
        let line = serde_json::to_string(&resp)? + "\n";
        self.write_line(&line).await
    }

    pub async fn send_error_response(
        &self,
        id: Value,
        code: i64,
        message: impl Into<String>,
    ) -> Result<()> {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: Some(id),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        };
        let line = serde_json::to_string(&resp)? + "\n";
        self.write_line(&line).await
    }

    /// A stalled peer must not hold the authority gate forever. Any failed or
    /// interrupted line poisons the stream: a partial NDJSON record cannot be
    /// followed by a fresh permission grant. Drop stdin to close the pipe.
    async fn write_line(&self, line: &str) -> Result<()> {
        if self.poisoned.load(Ordering::Acquire) { return Err(AcpError::ChannelClosed); }
        let mut stdin = match tokio::time::timeout(self.write_timeout, self.stdin.lock()).await {
            Ok(guard) => guard,
            Err(_) => {
                self.poisoned.store(true, Ordering::Release);
                self.pending.lock().await.clear();
                return Err(AcpError::Timeout("ACP stdin lock; transport closed".into()));
            }
        };
        if self.poisoned.load(Ordering::Acquire) {
            stdin.take();
            return Err(AcpError::ChannelClosed);
        }
        struct IncompleteWrite<'a> {
            pipe: &'a mut Option<ChildStdin>,
            poisoned: &'a AtomicBool,
            completed: bool,
        }
        impl Drop for IncompleteWrite<'_> {
            fn drop(&mut self) {
                if !self.completed {
                    self.poisoned.store(true, Ordering::Release);
                    self.pipe.take();
                }
            }
        }
        let mut write = IncompleteWrite { pipe: &mut stdin, poisoned: &self.poisoned, completed: false };
        let result: Result<()> = match write.pipe.as_mut() {
            Some(pipe) => match tokio::time::timeout(self.write_timeout, async {
                pipe.write_all(line.as_bytes()).await?;
                pipe.flush().await
            }).await {
                Ok(result) => result.map_err(AcpError::Io),
                Err(_) => Err(AcpError::Timeout("ACP stdin write; transport closed".into())),
            },
            None => Err(AcpError::ChannelClosed),
        };
        write.completed = result.is_ok();
        drop(write);
        if result.is_err() || self.poisoned.load(Ordering::Acquire) {
            self.poisoned.store(true, Ordering::Release);
            stdin.take();
            self.pending.lock().await.clear();
            return result.and(Err(AcpError::ChannelClosed));
        }
        result
    }

    pub fn unwrap_response(resp: JsonRpcResponse) -> Result<Value> {
        if let Some(err) = resp.error {
            return Err(AcpError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    /// Wait for a pending response with an explicit timeout.
    pub async fn request_with_timeout(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: std::time::Duration,
    ) -> Result<Value> {
        let rx = self.send_request(method, params).await?;
        let resp = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| AcpError::Timeout(method.to_string()))?
            .map_err(|_| AcpError::ChannelClosed)?;
        Self::unwrap_response(resp)
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        let n = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&n)? + "\n";
        self.write_line(&line).await
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{process::Stdio, time::Duration};
    use tokio::sync::oneshot::error::TryRecvError;

    #[tokio::test]
    async fn queued_frames_keep_host_epoch_and_provider_cannot_choose_it() {
        let mut command=tokio::process::Command::new("/bin/sh");
        command.args(["-c",r#"printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"message":"generated old output","hostEpoch":9999}}' '{"jsonrpc":"2.0","id":"old-request","method":"fs/write_text_file","host_epoch":9999,"params":{"path":"generated","content":"test"}}'; sleep 30"#]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).kill_on_drop(true);
        let mut child=command.spawn().unwrap();let(tx,mut notifications)=tokio::sync::mpsc::unbounded_channel();let(requests,mut request_rx)=tokio::sync::mpsc::unbounded_channel();
        let epoch=Arc::new(AtomicU64::new(7));let _transport=NdjsonTransport::new_scoped(child.stdin.take().unwrap(),child.stdout.take().unwrap(),tx,requests,None,Some(epoch.clone()));
        let notification=tokio::time::timeout(Duration::from_secs(2),notifications.recv()).await.unwrap().unwrap();let request=tokio::time::timeout(Duration::from_secs(2),request_rx.recv()).await.unwrap().unwrap();
        epoch.store(9,Ordering::Release);assert!(matches!(notification,NotificationEvent::ScopedNotification(7,_)));assert_eq!(request.host_epoch,Some(7));child.kill().await.unwrap();
    }

    #[tokio::test]
    async fn oversized_newline_free_frame_has_a_fixed_allocation_bound() {
        let payload = vec![b'x'; MAX_FRAME_BYTES * 2];
        let mut reader = BufReader::new(std::io::Cursor::new(payload));
        let mut frame = Vec::new();
        let error = bounded_line(&mut reader, &mut frame).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(frame.len() <= MAX_FRAME_BYTES);
        assert!(frame.capacity() <= MAX_FRAME_BYTES);
    }

    #[tokio::test]
    async fn newline_free_native_flood_fails_and_reaches_verified_group_and_pipe_cleanup() {
        use grok_cli_wrapper::process::{ProcessHandle, ProcessConfig, ProcessEnd, DrainTicket};
        let mut child = tokio::process::Command::new("/bin/sh")
            .args(["-c", "head -c 2097152 /dev/zero; sleep 30"])
            .process_group(0).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .kill_on_drop(true).spawn().unwrap();
        let (notifications, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let (requests, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let (reporter, ticket) = DrainTicket::pair();
        let _transport = NdjsonTransport::new_with_drain(child.stdin.take().unwrap(), child.stdout.take().unwrap(), notifications, requests, Some(reporter));
        let process = ProcessHandle::adopt(child, ProcessConfig { timeout:None, ..Default::default() }, vec![ticket]).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(8), process.wait_outcome()).await.unwrap();
        assert_eq!(outcome.end, ProcessEnd::IoFailure, "{outcome:?}");
        assert!(outcome.cleanup_complete && outcome.pipes_complete, "{outcome:?}");
        assert!(outcome.error.unwrap().contains("frame exceeds"));
    }

    #[tokio::test]
    async fn malformed_utf8_native_frame_stops_runner_without_a_success_claim() {
        use grok_cli_wrapper::process::{ProcessHandle, ProcessConfig, ProcessEnd, DrainTicket};
        let mut child = tokio::process::Command::new("/bin/sh")
            .args(["-c", r#"printf '\377\n'; sleep 30"#])
            .process_group(0).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .kill_on_drop(true).spawn().unwrap();
        let (notifications, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let (requests, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let (reporter, ticket) = DrainTicket::pair();
        let _transport = NdjsonTransport::new_with_drain(child.stdin.take().unwrap(), child.stdout.take().unwrap(), notifications, requests, Some(reporter));
        let process = ProcessHandle::adopt(child, ProcessConfig { timeout:None, ..Default::default() }, vec![ticket]).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(8), process.wait_outcome()).await.unwrap();
        assert_eq!(outcome.end, ProcessEnd::IoFailure, "{outcome:?}");
        assert!(outcome.cleanup_complete && outcome.pipes_complete);
        assert!(outcome.error.unwrap().contains("utf-8"));
    }

    #[tokio::test]
    async fn non_draining_peer_times_out_and_cannot_receive_a_later_grant() {
        let mut child = tokio::process::Command::new("/bin/sleep").arg("30")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let (notifications, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let (requests, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let transport = NdjsonTransport::new(child.stdin.take().unwrap(), child.stdout.take().unwrap(), notifications, requests);
        let error = tokio::time::timeout(Duration::from_secs(7), transport.send_response(
            Value::String("blocked".into()), Value::String("x".repeat(2 * 1024 * 1024))))
            .await.unwrap().unwrap_err();
        assert!(matches!(error, AcpError::Timeout(_)));
        assert!(transport.stdin.lock().await.is_none());
        assert!(matches!(transport.send_response(Value::String("grant".into()), serde_json::json!({"allow":true})).await,
            Err(AcpError::ChannelClosed)));
        child.kill().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_partial_write_closes_and_poisons_the_stream() {
        let mut child = tokio::process::Command::new("/bin/sleep").arg("30")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let (notifications, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let (requests, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let transport = NdjsonTransport::new(child.stdin.take().unwrap(), child.stdout.take().unwrap(), notifications, requests);
        let writing = transport.clone();
        let blocked = tokio::spawn(async move { writing.send_response(Value::Null, Value::String("x".repeat(2 * 1024 * 1024))).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while transport.stdin.try_lock().is_ok() { tokio::task::yield_now().await; }
        }).await.unwrap();
        blocked.abort(); let _ = blocked.await;
        assert!(transport.stdin.lock().await.is_none());
        assert!(transport.poisoned.load(Ordering::Acquire));
        assert!(matches!(transport.notify("session/cancel", None).await, Err(AcpError::ChannelClosed)));
        child.kill().await.unwrap();
    }

    #[tokio::test]
    async fn rpc_completion_fence_cannot_overtake_prior_wire_notifications() {
        // The process returns the actual request ID and deliberately batches the
        // two native messages and response before the consumer handles anything.
        let mut child = tokio::process::Command::new("/usr/bin/awk")
            .arg(r#"{ match($0, /"id":"[^"]*"/); id=substr($0,RSTART+6,RLENGTH-7); print "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"message\":\"early PASS\"}}"; print "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"message\":\"final FAIL\"}}"; print "{\"jsonrpc\":\"2.0\",\"id\":\"" id "\",\"result\":{\"stopReason\":\"end_turn\"}}"; fflush(); }"#)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let (notifications, mut updates) = tokio::sync::mpsc::unbounded_channel();
        let (requests, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let transport = NdjsonTransport::new(child.stdin.take().unwrap(), child.stdout.take().unwrap(), notifications, requests);
        let response = transport.send_request("session/prompt", None).await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(3), response).await.unwrap().unwrap();
        assert_eq!(NdjsonTransport::unwrap_response(response).unwrap()["stopReason"], "end_turn");
        let waiting_transport = transport.clone();
        let (finished_tx, mut finished_rx) = oneshot::channel();
        let completion = tokio::spawn(async move {
            waiting_transport.drain_notifications().await.unwrap();
            let _ = finished_tx.send(());
        });
        let mut messages = Vec::new();
        for expected in ["early PASS", "final FAIL"] {
            match tokio::time::timeout(Duration::from_secs(3), updates.recv()).await.unwrap().unwrap() {
                NotificationEvent::Notification(notification) => {
                    let message = notification.params.unwrap()["message"].as_str().unwrap().to_owned();
                    assert_eq!(message, expected); messages.push(message);
                }
                NotificationEvent::Fence(_) => panic!("completion overtook native output"),
                NotificationEvent::ScopedNotification(..)=>panic!("fixture unexpectedly scoped"),
            }
            assert!(matches!(finished_rx.try_recv(), Err(TryRecvError::Empty)));
        }
        match tokio::time::timeout(Duration::from_secs(3), updates.recv()).await.unwrap().unwrap() {
            NotificationEvent::Fence(ack) => { assert_eq!(messages, vec!["early PASS", "final FAIL"]); ack.send(()).unwrap(); }
            NotificationEvent::Notification(_) | NotificationEvent::ScopedNotification(..) => panic!("unexpected native output"),
        }
        tokio::time::timeout(Duration::from_secs(3), finished_rx).await.unwrap().unwrap();
        completion.await.unwrap();
        child.kill().await.unwrap();
    }

    #[tokio::test]
    async fn closed_notification_consumer_cannot_acknowledge_completion() {
        let mut child = tokio::process::Command::new("/bin/cat")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let (notifications, receiver) = tokio::sync::mpsc::unbounded_channel(); drop(receiver);
        let (requests, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let transport = NdjsonTransport::new(child.stdin.take().unwrap(), child.stdout.take().unwrap(), notifications, requests);
        assert!(matches!(transport.drain_notifications().await, Err(AcpError::ChannelClosed)));
        child.kill().await.unwrap();
    }
}
