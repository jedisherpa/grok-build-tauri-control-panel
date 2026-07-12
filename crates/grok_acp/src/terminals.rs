//! ACP terminal/* host: create, output, wait_for_exit, kill, release.
//!
//! Spec: https://agentclientprotocol.com/protocol/v1/terminals
//! Without this, Grok `run_terminal_command` hangs on `terminal/create`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, watch};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::error::{AcpError, Result};

const DEFAULT_OUTPUT_LIMIT: usize = 1_048_576; // 1 MiB

#[derive(Debug)]
struct TerminalState {
    output: String,
    truncated: bool,
    output_limit: usize,
    exit_code: Option<i32>,
    signal: Option<String>,
    finished: bool,
}

impl TerminalState {
    fn push_bytes(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        let s = String::from_utf8_lossy(chunk);
        self.output.push_str(&s);
        if self.output.len() > self.output_limit {
            // Truncate from the beginning at a char boundary.
            let excess = self.output.len() - self.output_limit;
            let mut cut = excess.min(self.output.len());
            while cut < self.output.len() && !self.output.is_char_boundary(cut) {
                cut += 1;
            }
            self.output = self.output[cut..].to_string();
            self.truncated = true;
        }
    }

    fn to_output_result(&self) -> Value {
        let mut out = json!({
            "output": self.output,
            "truncated": self.truncated,
        });
        if self.finished {
            out["exitStatus"] = json!({
                "exitCode": self.exit_code,
                "signal": self.signal,
            });
        }
        out
    }

    fn to_wait_result(&self) -> Value {
        json!({
            "exitCode": self.exit_code,
            "signal": self.signal,
        })
    }
}

struct ManagedTerminal {
    state: Arc<Mutex<TerminalState>>,
    /// `true` once the process has exited (watch avoids lost-wakeup races).
    finished_rx: watch::Receiver<bool>,
    child: Arc<Mutex<Option<Child>>>,
}

/// In-memory terminal registry for one ACP client connection.
pub struct TerminalRegistry {
    terminals: Mutex<HashMap<String, ManagedTerminal>>,
    default_cwd: PathBuf,
}

impl TerminalRegistry {
    pub fn new(default_cwd: PathBuf) -> Self {
        Self {
            terminals: Mutex::new(HashMap::new()),
            default_cwd,
        }
    }

    pub async fn handle(&self, method: &str, params: &Option<Value>) -> Result<Value> {
        match method {
            "terminal/create" => self.create(params).await,
            "terminal/output" => self.output(params).await,
            "terminal/wait_for_exit" | "terminal/waitForExit" => self.wait_for_exit(params).await,
            "terminal/kill" => self.kill(params).await,
            "terminal/release" => self.release(params).await,
            other => Err(AcpError::Protocol(format!(
                "unknown terminal method: {other}"
            ))),
        }
    }

    async fn create(&self, params: &Option<Value>) -> Result<Value> {
        let p = params
            .as_ref()
            .ok_or_else(|| AcpError::Protocol("terminal/create missing params".into()))?;

        let command = p
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AcpError::Protocol("terminal/create missing command".into()))?
            .to_string();

        let args: Vec<String> = p
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let cwd = p
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.default_cwd.clone());

        let output_limit = p
            .get("outputByteLimit")
            .or_else(|| p.get("output_byte_limit"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_OUTPUT_LIMIT)
            .max(1024);

        let env_pairs: Vec<(String, String)> = p
            .get("env")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let name = item.get("name")?.as_str()?.to_string();
                        let value = item.get("value")?.as_str()?.to_string();
                        Some((name, value))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut cmd = build_command(&command, &args, &cwd, &env_pairs);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        debug!(%command, cwd = %cwd.display(), "terminal/create spawn");

        let mut child = cmd
            .spawn()
            .map_err(|e| AcpError::Protocol(format!("terminal/create spawn failed: {e}")))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let terminal_id = format!("term_{}", Uuid::new_v4().simple());
        let state = Arc::new(Mutex::new(TerminalState {
            output: String::new(),
            truncated: false,
            output_limit,
            exit_code: None,
            signal: None,
            finished: false,
        }));
        let (finished_tx, finished_rx) = watch::channel(false);
        let child_slot = Arc::new(Mutex::new(Some(child)));

        // Pump stdout
        if let Some(out) = stdout {
            let st = state.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(out);
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => st.lock().await.push_bytes(&buf[..n]),
                        Err(e) => {
                            warn!(error = %e, "terminal stdout read error");
                            break;
                        }
                    }
                }
            });
        }

        // Pump stderr into the same output buffer (matches shell UX).
        if let Some(err) = stderr {
            let st = state.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(err);
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => st.lock().await.push_bytes(&buf[..n]),
                        Err(e) => {
                            warn!(error = %e, "terminal stderr read error");
                            break;
                        }
                    }
                }
            });
        }

        // Wait for process exit
        {
            let st = state.clone();
            let child_slot = child_slot.clone();
            tokio::spawn(async move {
                let status = {
                    let mut guard = child_slot.lock().await;
                    if let Some(mut child) = guard.take() {
                        match child.wait().await {
                            Ok(s) => Some(s),
                            Err(e) => {
                                warn!(error = %e, "terminal wait failed");
                                None
                            }
                        }
                    } else {
                        None
                    }
                };
                {
                    let mut s = st.lock().await;
                    if let Some(status) = status {
                        s.exit_code = status.code();
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::ExitStatusExt;
                            if status.code().is_none() {
                                if let Some(sig) = status.signal() {
                                    s.signal = Some(sig.to_string());
                                }
                            }
                        }
                    } else {
                        s.exit_code = Some(-1);
                    }
                    s.finished = true;
                }
                let _ = finished_tx.send(true);
            });
        }

        self.terminals.lock().await.insert(
            terminal_id.clone(),
            ManagedTerminal {
                state,
                finished_rx,
                child: child_slot,
            },
        );

        Ok(json!({ "terminalId": terminal_id }))
    }

    async fn output(&self, params: &Option<Value>) -> Result<Value> {
        let id = terminal_id(params)?;
        let map = self.terminals.lock().await;
        let term = map
            .get(&id)
            .ok_or_else(|| AcpError::Protocol(format!("unknown terminalId: {id}")))?;
        let state = term.state.lock().await;
        Ok(state.to_output_result())
    }

    async fn wait_for_exit(&self, params: &Option<Value>) -> Result<Value> {
        let id = terminal_id(params)?;
        let (mut finished_rx, state) = {
            let map = self.terminals.lock().await;
            let term = map
                .get(&id)
                .ok_or_else(|| AcpError::Protocol(format!("unknown terminalId: {id}")))?;
            (term.finished_rx.clone(), term.state.clone())
        };

        // watch::Receiver already holds current value — no lost-wakeup race.
        if !*finished_rx.borrow() {
            while !*finished_rx.borrow() {
                if finished_rx.changed().await.is_err() {
                    break;
                }
            }
        }

        let s = state.lock().await;
        Ok(s.to_wait_result())
    }

    async fn kill(&self, params: &Option<Value>) -> Result<Value> {
        let id = terminal_id(params)?;
        let map = self.terminals.lock().await;
        let term = map
            .get(&id)
            .ok_or_else(|| AcpError::Protocol(format!("unknown terminalId: {id}")))?;
        let mut child_guard = term.child.lock().await;
        if let Some(child) = child_guard.as_mut() {
            let _ = child.kill().await;
        }
        Ok(json!({}))
    }

    async fn release(&self, params: &Option<Value>) -> Result<Value> {
        let id = terminal_id(params)?;
        let mut map = self.terminals.lock().await;
        if let Some(term) = map.remove(&id) {
            let mut child_guard = term.child.lock().await;
            if let Some(mut child) = child_guard.take() {
                let _ = child.kill().await;
            }
        }
        Ok(json!({}))
    }

    /// Short human line for the center-column terminal mirror.
    pub fn summary_line(method: &str, params: &Option<Value>, result: &Result<Value>) -> String {
        match method {
            "terminal/create" => {
                let cmd = params
                    .as_ref()
                    .and_then(|p| p.get("command"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let args = params
                    .as_ref()
                    .and_then(|p| p.get("args"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                let full = if args.is_empty() {
                    cmd.to_string()
                } else {
                    format!("{cmd} {args}")
                };
                let short = if full.len() > 100 {
                    format!("{}…", &full[..100])
                } else {
                    full
                };
                match result {
                    Ok(v) => {
                        let id = v
                            .get("terminalId")
                            .and_then(|x| x.as_str())
                            .unwrap_or("?");
                        format!("$ {short}  [{id}]")
                    }
                    Err(e) => format!("$ {short}  [spawn failed: {e}]"),
                }
            }
            "terminal/wait_for_exit" | "terminal/waitForExit" => match result {
                Ok(v) => {
                    let code = v
                        .get("exitCode")
                        .and_then(|c| c.as_i64())
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".into());
                    format!("· terminal exit {code}")
                }
                Err(e) => format!("· terminal wait error: {e}"),
            },
            "terminal/kill" => "· terminal kill".into(),
            "terminal/release" => "· terminal release".into(),
            "terminal/output" => "· terminal output".into(),
            other => format!("· {other}"),
        }
    }
}

fn terminal_id(params: &Option<Value>) -> Result<String> {
    params
        .as_ref()
        .and_then(|p| p.get("terminalId").or_else(|| p.get("terminal_id")))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| AcpError::Protocol("missing terminalId".into()))
}

/// Build a process command. If `args` is empty and `command` looks like a shell
/// snippet (spaces / metacharacters), run via `/bin/zsh -lc` so Grok's
/// `run_terminal_command` payloads work.
fn build_command(
    command: &str,
    args: &[String],
    cwd: &Path,
    env_pairs: &[(String, String)],
) -> Command {
    let mut cmd = if args.is_empty() && needs_shell(command) {
        let mut c = Command::new("/bin/zsh");
        c.arg("-lc").arg(command);
        c
    } else {
        let mut c = Command::new(command);
        for a in args {
            c.arg(a);
        }
        c
    };

    cmd.current_dir(cwd);

    // GUI apps often lack a login-shell PATH; ensure common tool locations.
    let path = std::env::var("PATH").unwrap_or_default();
    let augmented = if path.is_empty() {
        "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
    } else if !path.contains("/opt/homebrew/bin") {
        format!("/opt/homebrew/bin:/usr/local/bin:{path}")
    } else {
        path
    };
    cmd.env("PATH", augmented);
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }
    cmd
}

fn needs_shell(command: &str) -> bool {
    command.contains(' ')
        || command.contains('|')
        || command.contains('&')
        || command.contains(';')
        || command.contains('>')
        || command.contains('<')
        || command.contains('$')
        || command.contains('`')
        || command.contains('\n')
        || command.contains('(')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_wait_output_echo() {
        let reg = TerminalRegistry::new(std::env::temp_dir());
        let create = reg
            .handle(
                "terminal/create",
                &Some(json!({
                    "command": "echo",
                    "args": ["hello-acp-term"],
                    "cwd": std::env::temp_dir().to_string_lossy(),
                })),
            )
            .await
            .expect("create");
        let id = create["terminalId"].as_str().unwrap().to_string();

        let wait = reg
            .handle(
                "terminal/wait_for_exit",
                &Some(json!({ "terminalId": id })),
            )
            .await
            .expect("wait");
        assert_eq!(wait["exitCode"], 0);

        let out = reg
            .handle("terminal/output", &Some(json!({ "terminalId": id })))
            .await
            .expect("output");
        let text = out["output"].as_str().unwrap_or("");
        assert!(
            text.contains("hello-acp-term"),
            "output was: {text:?}"
        );
        assert_eq!(out["truncated"], false);
        assert_eq!(out["exitStatus"]["exitCode"], 0);

        reg.handle("terminal/release", &Some(json!({ "terminalId": id })))
            .await
            .expect("release");
    }

    #[tokio::test]
    async fn shell_snippet_via_zsh() {
        let reg = TerminalRegistry::new(std::env::temp_dir());
        let create = reg
            .handle(
                "terminal/create",
                &Some(json!({
                    "command": "echo hi && echo there",
                })),
            )
            .await
            .expect("create");
        let id = create["terminalId"].as_str().unwrap().to_string();
        let wait = reg
            .handle(
                "terminal/wait_for_exit",
                &Some(json!({ "terminalId": id })),
            )
            .await
            .expect("wait");
        assert_eq!(wait["exitCode"], 0);
        let out = reg
            .handle("terminal/output", &Some(json!({ "terminalId": id })))
            .await
            .expect("output");
        let text = out["output"].as_str().unwrap_or("");
        assert!(text.contains("hi"), "{text:?}");
        assert!(text.contains("there"), "{text:?}");
    }

    #[test]
    fn needs_shell_detects_snippets() {
        assert!(needs_shell("pwd && ls"));
        assert!(needs_shell("echo hi"));
        assert!(!needs_shell("ls"));
        assert!(!needs_shell("/bin/echo"));
    }
}
