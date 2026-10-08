//! ACP terminal/* host: create, output, wait_for_exit, kill, release.
//!
//! Spec: https://agentclientprotocol.com/protocol/v1/terminals
//! Without this, Grok `run_terminal_command` hangs on `terminal/create`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::debug;
use uuid::Uuid;

use crate::error::{AcpError, Result};
use grok_cli_wrapper::process::{ProcessHandle, ProcessConfig, ProcessOutcome, ProcessEnd};

const DEFAULT_OUTPUT_LIMIT: usize = 1_048_576; // 1 MiB

struct ManagedTerminal {
    process: ProcessHandle,
    output_limit: usize,
}

fn cleanup_result(outcome: &ProcessOutcome) -> Result<()> {
    if !outcome.cleanup_complete {
        return Err(AcpError::Protocol(format!("terminal cleanup unresolved: {}",
            outcome.error.as_deref().unwrap_or("owned process or pipes still active"))));
    }
    Ok(())
}

fn wait_result(outcome: &ProcessOutcome) -> Value {
    json!({"exitCode":outcome.exit_code, "signal":outcome.signal.map(|value| value.to_string()),
        "cleanupComplete":outcome.cleanup_complete, "cleanupScope":outcome.cleanup_scope,
        "pipesComplete":outcome.pipes_complete,
        "end":outcome.end, "error":outcome.error})
}

/// In-memory terminal registry for one ACP client connection.
pub struct TerminalRegistry {
    terminals: Mutex<HashMap<String, ManagedTerminal>>,
    default_cwd: PathBuf,
    unrestricted: bool,
}

impl TerminalRegistry {
    pub fn new(default_cwd: PathBuf) -> Self {
        Self::with_policy(default_cwd, false)
    }

    pub fn with_policy(default_cwd: PathBuf, unrestricted: bool) -> Self {
        Self {
            terminals: Mutex::new(HashMap::new()),
            default_cwd,
            unrestricted,
        }
    }

    #[cfg(test)]
    pub(crate) async fn retain_fixture_process(&self, process: ProcessHandle) {
        self.terminals.lock().await.insert("generated-partial-cleanup".into(),
            ManagedTerminal { process, output_limit:DEFAULT_OUTPUT_LIMIT });
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
        // Terminals must stay inside the workspace — an agent-supplied cwd
        // outside it defeats the fs sandbox entirely.
        let workspace = self
            .default_cwd
            .canonicalize()
            .unwrap_or_else(|_| self.default_cwd.clone());
        let cwd_canon = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
        if cwd_canon != workspace && !cwd_canon.starts_with(&workspace) {
            return Err(AcpError::Protocol(format!(
                "terminal cwd outside workspace: {}",
                cwd.display()
            )));
        }
        let cwd = cwd_canon;

        let output_limit = p
            .get("outputByteLimit")
            .or_else(|| p.get("output_byte_limit"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_OUTPUT_LIMIT)
            .clamp(1024, DEFAULT_OUTPUT_LIMIT);

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

        let mut cmd = contained_command(build_command(&command, &args, &cwd, &env_pairs),
            &workspace, self.unrestricted)?;
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);

        debug!(%command, cwd = %cwd.display(), "terminal/create spawn");

        ProcessConfig::ensure_supported()?;
        let (child, group_proof) = grok_cli_wrapper::process::spawn_group(&mut cmd)
            .map_err(|error| AcpError::Protocol(format!("terminal/create spawn failed: {error}")))?;
        let process = ProcessHandle::adopt_attested(child, group_proof, ProcessConfig {
            timeout:None, output_limit, ..ProcessConfig::default()
        }, Vec::new())?;
        let terminal_id = format!("term_{}", Uuid::new_v4().simple());
        self.terminals.lock().await.insert(terminal_id.clone(), ManagedTerminal { process, output_limit });

        Ok(json!({ "terminalId": terminal_id }))
    }

    async fn process(&self, id: &str) -> Result<ProcessHandle> {
        self.terminals.lock().await.get(id).map(|term| term.process.clone())
            .ok_or_else(|| AcpError::Protocol(format!("unknown terminalId: {id}")))
    }

    async fn output(&self, params: &Option<Value>) -> Result<Value> {
        let id = terminal_id(params)?;
        let (process, limit) = {
            let terminals = self.terminals.lock().await;
            let terminal = terminals.get(&id).ok_or_else(|| AcpError::Protocol(format!("unknown terminalId: {id}")))?;
            (terminal.process.clone(), terminal.output_limit)
        };
        let output = process.output();
        let text = output.text();
        let mut cut = text.len().saturating_sub(limit);
        while !text.is_char_boundary(cut) { cut += 1; }
        let mut result = json!({"output":&text[cut..], "truncated":output.truncated || cut > 0});
        if let Some(outcome) = process.outcome() {
            result["exitStatus"] = json!({"exitCode":outcome.exit_code,
                "signal":outcome.signal.map(|value| value.to_string())});
            result["cleanupComplete"] = json!(outcome.cleanup_complete);
            result["cleanupScope"] = json!(outcome.cleanup_scope);
            result["pipesComplete"] = json!(outcome.pipes_complete);
            result["error"] = json!(outcome.error);
        }
        Ok(result)
    }

    async fn wait_for_exit(&self, params: &Option<Value>) -> Result<Value> {
        let process = self.process(&terminal_id(params)?).await?;
        let outcome = process.wait_outcome().await;
        cleanup_result(&outcome)?;
        if !outcome.pipes_complete || matches!(outcome.end, ProcessEnd::IoFailure | ProcessEnd::CleanupFailure | ProcessEnd::Interrupted) {
            return Err(AcpError::Protocol(format!("terminal completion unconfirmed ({:?}): {}", outcome.end,
                outcome.error.as_deref().unwrap_or("pipe drain not observed"))));
        }
        Ok(wait_result(&outcome))
    }

    async fn kill(&self, params: &Option<Value>) -> Result<Value> {
        let process = self.process(&terminal_id(params)?).await?;
        cleanup_result(&process.cancel().await)?;
        Ok(json!({}))
    }

    /// Retain terminal history and unresolved process owners after Stop.
    pub async fn kill_all(&self) -> Result<()> {
        let processes:Vec<_> = self.terminals.lock().await.values().map(|terminal| terminal.process.clone()).collect();
        let outcomes = futures::future::join_all(processes.iter().map(ProcessHandle::cancel)).await;
        for outcome in outcomes { cleanup_result(&outcome)?; }
        Ok(())
    }

    async fn release(&self, params: &Option<Value>) -> Result<Value> {
        let id = terminal_id(params)?;
        let process = match self.process(&id).await {
            Ok(process) => process,
            Err(_) => return Ok(json!({})),
        };
        cleanup_result(&process.cancel().await)?;
        // A failed cleanup never removes the retry handle or its output.
        self.terminals.lock().await.remove(&id);
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
                    let mut end = 100;
                    while !full.is_char_boundary(end) { end -= 1; }
                    format!("{}…", &full[..end])
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

/// The user's shell, falling back through common defaults ($SHELL → zsh →
/// bash → sh) so this works beyond zsh-only macOS setups.
pub(crate) fn user_shell() -> String {
    if let Ok(sh) = std::env::var("SHELL") {
        if !sh.trim().is_empty() && Path::new(&sh).exists() {
            return sh;
        }
    }
    for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        if Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    "/bin/sh".into()
}

/// Build a process command. If `args` is empty and `command` looks like a shell
/// snippet (spaces / metacharacters), run via `$SHELL -lc` so Grok's
/// `run_terminal_command` payloads work.
fn build_command(
    command: &str,
    args: &[String],
    cwd: &Path,
    env_pairs: &[(String, String)],
) -> Command {
    let mut cmd = if args.is_empty() && needs_shell(command) {
        let mut c = Command::new(user_shell());
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

/// cwd alone is not a sandbox: shells can otherwise write absolute paths or
/// follow workspace symlinks. No fallback silently launches uncontained code.
fn contained_command(command: Command, workspace: &Path, unrestricted: bool) -> Result<Command> {
    if unrestricted { return Ok(command); }
    #[cfg(target_os = "macos")]
    {
        let sandbox = Path::new("/usr/bin/sandbox-exec");
        if !sandbox.is_file() {
            return Err(AcpError::Protocol("workspace terminal isolation unavailable: sandbox-exec missing".into()));
        }
        let profile = workspace_profile(workspace)?;
        let original = command.as_std();
        let mut contained = Command::new(sandbox);
        contained.arg("-p").arg(profile).arg(original.get_program()).args(original.get_args());
        if let Some(cwd) = original.get_current_dir() { contained.current_dir(cwd); }
        for (key, value) in original.get_envs() {
            if let Some(value) = value { contained.env(key, value); }
            else { contained.env_remove(key); }
        }
        // Compiler scratch files must remain inside the writable workspace.
        contained.env("TMPDIR", workspace);
        Ok(contained)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = workspace;
        Err(AcpError::Protocol("workspace terminal isolation unavailable on this platform; an explicit unrestricted profile is required".into()))
    }
}

#[cfg(target_os = "macos")]
fn workspace_profile(workspace: &Path) -> Result<String> {
    let quote = |path: &Path| serde_json::to_string(&path.to_string_lossy())
        .map_err(AcpError::from);
    let mut readable = vec![workspace.to_path_buf(), PathBuf::from("/System"), PathBuf::from("/usr"),
        PathBuf::from("/bin"), PathBuf::from("/sbin"), PathBuf::from("/Library"),
        PathBuf::from("/private/etc"), PathBuf::from("/private/var/db/dyld"),
        PathBuf::from("/opt/homebrew"), PathBuf::from("/dev/fd")];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for path in [".rustup", ".cargo/bin", ".cargo/registry", ".cargo/git", ".local/bin", ".grok/bin"] {
            readable.push(home.join(path));
        }
    }
    // Apple dyld-support.sb requires opening / as an openat root. This is a
    // literal directory grant, never a recursive grant to its descendants.
    let mut profile = "(version 1)(allow default)(deny file-read*)(deny file-write*)(allow file-read-metadata)(allow file-read* (literal \"/\"))".to_string();
    for path in readable {
        let path = path.canonicalize().unwrap_or(path);
        profile.push_str(&format!("(allow file-read* (subpath {}))", quote(&path)?));
    }
    for device in ["/dev/null", "/dev/random", "/dev/urandom"] {
        profile.push_str(&format!("(allow file-read* (literal {device:?}))"));
    }
    profile.push_str(&format!("(allow file-write* (subpath {})(literal \"/dev/null\"))", quote(workspace)?));
    Ok(profile)
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
    async fn stop_interrupts_active_wait_and_preserves_settled_output_until_release() {
        let fixture = tempfile::tempdir().unwrap();
        let registry = std::sync::Arc::new(TerminalRegistry::with_policy(fixture.path().to_path_buf(), true));
        let created = registry.handle("terminal/create", &Some(json!({"command":"/bin/sh", "args":["-c","printf READY; sleep 30 & wait"]}))).await.unwrap();
        let id = created["terminalId"].as_str().unwrap().to_string();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let out = registry.handle("terminal/output", &Some(json!({"terminalId":id}))).await.unwrap();
                if out["output"].as_str().unwrap().contains("READY") { break; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        let waiting = registry.clone(); let wait_id = id.clone();
        let wait = tokio::spawn(async move { waiting.handle("terminal/wait_for_exit", &Some(json!({"terminalId":wait_id}))).await });
        tokio::time::timeout(std::time::Duration::from_secs(8), registry.handle("terminal/kill", &Some(json!({"terminalId":id})))).await.unwrap().unwrap();
        let settled = tokio::time::timeout(std::time::Duration::from_secs(2), wait).await.unwrap().unwrap().unwrap();
        assert_eq!(settled["cleanupComplete"], true);
        assert_eq!(settled["pipesComplete"], true);
        assert_eq!(settled["signal"], "9");
        let out = registry.handle("terminal/output", &Some(json!({"terminalId":id}))).await.unwrap();
        assert!(out["output"].as_str().unwrap().contains("READY"));
        registry.handle("terminal/release", &Some(json!({"terminalId":id}))).await.unwrap();
        assert!(registry.handle("terminal/output", &Some(json!({"terminalId":id}))).await.is_err());
    }

    #[tokio::test]
    async fn terminal_exit_settles_both_pipes_and_clamps_combined_output() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("payload"), "é".repeat(8192)).unwrap();
        let registry = TerminalRegistry::with_policy(fixture.path().to_path_buf(), true);
        let created = registry.handle("terminal/create", &Some(json!({"command":"/bin/sh",
            "args":["-c","cat payload; printf FINAL-OUT; cat payload >&2; printf FINAL-ERR >&2"], "outputByteLimit":1024}))).await.unwrap();
        let id = created["terminalId"].as_str().unwrap();
        let settled = registry.handle("terminal/wait_for_exit", &Some(json!({"terminalId":id}))).await.unwrap();
        assert_eq!(settled["exitCode"], 0);
        assert_eq!(settled["pipesComplete"], true);
        let out = registry.handle("terminal/output", &Some(json!({"terminalId":id}))).await.unwrap();
        let text = out["output"].as_str().unwrap();
        assert!(text.len() <= 1024);
        assert!(text.ends_with("FINAL-ERR"));
        assert!(!text.contains('�'));
        assert_eq!(out["truncated"], true);
    }

    #[tokio::test]
    async fn fast_terminal_exits_preserve_attested_group_and_observed_pipes() {
        let fixture = tempfile::tempdir().unwrap();
        let registry = TerminalRegistry::with_policy(fixture.path().to_path_buf(), true);
        for iteration in 0..64 {
            let created = registry.handle("terminal/create", &Some(json!({
                "command":"/bin/echo", "args":[format!("fast-fixture-{iteration}")]
            }))).await.unwrap();
            let id = created["terminalId"].as_str().unwrap();
            let settled = tokio::time::timeout(std::time::Duration::from_secs(12),
                registry.handle("terminal/wait_for_exit", &Some(json!({"terminalId":id}))))
                .await.unwrap().unwrap();
            assert_eq!(settled["exitCode"], 0, "iteration {iteration}: {settled}");
            assert_eq!(settled["cleanupComplete"], true);
            assert_eq!(settled["pipesComplete"], true);
            let output = registry.handle("terminal/output", &Some(json!({"terminalId":id}))).await.unwrap();
            assert_eq!(output["cleanupScope"], "dedicated_process_group");
            assert!(output["output"].as_str().unwrap().contains(&format!("fast-fixture-{iteration}")));
            registry.handle("terminal/release", &Some(json!({"terminalId":id}))).await.unwrap();
        }
    }

    #[tokio::test]
    async fn create_wait_output_echo() {
        // This tests terminal transport/output, not confinement (below).
        let reg = TerminalRegistry::with_policy(std::env::temp_dir(), true);
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
        let reg = TerminalRegistry::with_policy(std::env::temp_dir(), true);
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

    #[tokio::test]
    async fn actual_terminal_summary_handles_unicode_across_the_clip_boundary() {
        let fixture = tempfile::tempdir().unwrap();
        let registry = TerminalRegistry::with_policy(fixture.path().to_path_buf(), true);
        let params = Some(json!({"command":"echo", "args":["é".repeat(100)]}));
        let created = registry.handle("terminal/create", &params).await;
        let summary = TerminalRegistry::summary_line("terminal/create", &params, &created);
        assert!(summary.contains('…'));
        assert!(!summary.contains('�'));
        let id = created.unwrap()["terminalId"].as_str().unwrap().to_string();
        registry.handle("terminal/wait_for_exit", &Some(json!({"terminalId":id}))).await.unwrap();
        registry.handle("terminal/release", &Some(json!({"terminalId":id}))).await.unwrap();
    }

    #[test]
    fn needs_shell_detects_snippets() {
        assert!(needs_shell("pwd && ls"));
        assert!(needs_shell("echo hi"));
        assert!(!needs_shell("ls"));
        assert!(!needs_shell("/bin/echo"));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn workspace_sandbox_allows_local_work_but_blocks_absolute_and_symlink_escape() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("private-read"), "fixture secret").unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("linked-outside")).unwrap();
        let reg = TerminalRegistry::new(workspace.clone());
        let command = format!("printf inside > local; printf escaped > {}; printf escaped > linked-outside/symlink-write; cat {}; printf done",
            outside.join("absolute-write").display(), outside.join("private-read").display());
        let result = reg.handle("terminal/create", &Some(json!({"command":"/bin/sh", "args":["-c",command]}))).await.unwrap();
        let id = result["terminalId"].as_str().unwrap();
        let waited = reg.handle("terminal/wait_for_exit", &Some(json!({"terminalId":id}))).await.unwrap();
        let output = reg.handle("terminal/output", &Some(json!({"terminalId":id}))).await.unwrap();
        assert_eq!(waited["exitCode"], 0, "sandbox must initialize and run: {output}");
        assert_eq!(std::fs::read_to_string(workspace.join("local")).unwrap(), "inside");
        assert!(!outside.join("absolute-write").exists());
        assert!(!outside.join("symlink-write").exists());
        assert!(!output["output"].as_str().unwrap().contains("fixture secret"));
        assert!(output["output"].as_str().unwrap().contains("done"));
        reg.handle("terminal/release", &Some(json!({"terminalId":id}))).await.unwrap();
    }
}
