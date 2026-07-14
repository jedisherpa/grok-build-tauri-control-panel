//! Grok authentication: status from ~/.grok/auth.json + interactive login with code paste.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::{CliError, GrokCli, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub logged_in: bool,
    pub email: Option<String>,
    pub auth_mode: Option<String>,
    pub team_id: Option<String>,
    pub first_name: Option<String>,
    pub expires_at: Option<String>,
    pub oidc_issuer: Option<String>,
    pub auth_file: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoginPhase {
    #[default]
    Starting,
    /// Browser open; show confirm code; accept paste-back code.
    AwaitingBrowser,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginSessionState {
    pub active: bool,
    pub phase: LoginPhase,
    pub method: String,
    pub login_url: Option<String>,
    /// Code to confirm **in the browser** (also shown large in Bomb Code).
    pub confirm_code: Option<String>,
    pub instructions: String,
    /// True while we accept a code pasted from the browser into Bomb Code.
    pub needs_paste: bool,
    pub status: AuthStatus,
    pub output_tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub status: AuthStatus,
    pub login_url: Option<String>,
    pub user_code: Option<String>,
    pub method: String,
    pub output: String,
}

#[derive(Default)]
struct SharedLoginData {
    output: String,
    login_url: Option<String>,
    confirm_code: Option<String>,
    phase: LoginPhase,
    done: bool,
    method: String,
}

struct ActiveLogin {
    child: Child,
    stdin: Option<ChildStdin>,
    shared: Arc<Mutex<SharedLoginData>>,
}

/// Interactive `grok login` session manager (device code + paste support).
pub struct LoginManager {
    active: Mutex<Option<ActiveLogin>>,
    grok_path: PathBuf,
}

impl LoginManager {
    pub fn new(grok_path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            active: Mutex::new(None),
            grok_path,
        })
    }

    pub async fn cancel(&self) {
        let mut g = self.active.lock().await;
        if let Some(mut active) = g.take() {
            let _ = active.child.kill().await;
            let _ = active.child.wait().await;
            let mut s = active.shared.lock().await;
            s.done = true;
            s.phase = LoginPhase::Failed;
        }
    }

    pub async fn state(&self) -> LoginSessionState {
        let mut g = self.active.lock().await;
        let Some(ref mut active) = *g else {
            let status = GrokCli::auth_status();
            return LoginSessionState {
                active: false,
                phase: if status.logged_in {
                    LoginPhase::Completed
                } else {
                    LoginPhase::Failed
                },
                method: String::new(),
                login_url: None,
                confirm_code: None,
                instructions: if status.logged_in {
                    status.message.clone()
                } else {
                    "Not signed in. Click Log in with Grok.".into()
                },
                needs_paste: false,
                status,
                output_tail: String::new(),
            };
        };

        // Detect process exit. Don't sleep while holding the shared lock —
        // it blocks every concurrent grok_login_status/submit_code call.
        if let Ok(Some(status)) = active.child.try_wait() {
            let first_observer = {
                let mut s = active.shared.lock().await;
                if s.done {
                    false
                } else {
                    s.done = true;
                    s.output
                        .push_str(&format!("\n[login process exited: {status}]\n"));
                    true
                }
            };
            if first_observer {
                // Give the CLI a beat to flush its credential cache.
                tokio::time::sleep(Duration::from_millis(250)).await;
                let auth = GrokCli::auth_status();
                let mut s = active.shared.lock().await;
                s.phase = if auth.logged_in {
                    LoginPhase::Completed
                } else {
                    LoginPhase::Failed
                };
            }
        }

        let auth = GrokCli::auth_status();
        let s = active.shared.lock().await;
        let mut phase = s.phase.clone();
        if auth.logged_in {
            phase = LoginPhase::Completed;
        }

        let needs_paste = matches!(phase, LoginPhase::AwaitingBrowser | LoginPhase::Starting);
        let instructions = match phase {
            LoginPhase::Starting => "Starting Grok login…".into(),
            LoginPhase::AwaitingBrowser => {
                if let Some(ref code) = s.confirm_code {
                    format!(
                        "Confirm this code in your browser: {code}\n\
                         If the browser shows a code to paste back into the app, enter it below and press Submit."
                    )
                } else {
                    "Complete sign-in in the browser.\n\
                     If it shows a code to paste into Bomb Code, enter it below."
                        .into()
                }
            }
            LoginPhase::Completed => auth.message.clone(),
            LoginPhase::Failed => {
                "Login did not complete. Try Log in with Grok again.".into()
            }
        };

        LoginSessionState {
            active: !matches!(phase, LoginPhase::Completed | LoginPhase::Failed) || !s.done,
            phase,
            method: s.method.clone(),
            login_url: s.login_url.clone(),
            confirm_code: s.confirm_code.clone(),
            instructions,
            needs_paste,
            status: auth,
            output_tail: tail(&s.output, 1500),
        }
    }

    pub async fn start_device_login(self: &Arc<Self>) -> Result<LoginSessionState> {
        self.start_login(&["login", "--device-auth"], "device").await
    }

    pub async fn start_oauth_login(self: &Arc<Self>) -> Result<LoginSessionState> {
        self.start_login(&["login", "--oauth"], "oauth").await
    }

    async fn start_login(self: &Arc<Self>, args: &[&str], method: &str) -> Result<LoginSessionState> {
        self.cancel().await;

        if !self.grok_path.exists() {
            return Err(CliError::BinaryNotFound(
                self.grok_path.display().to_string(),
            ));
        }

        let mut cmd = Command::new(&self.grok_path);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let path = std::env::var("PATH").unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        cmd.env(
            "PATH",
            format!(
                "{home}/.grok/bin:{home}/.cargo/bin:{home}/.local/bin:/opt/homebrew/bin:/usr/local/bin:{path}"
            ),
        );
        if !home.is_empty() {
            cmd.env("HOME", &home);
        }
        if let Ok(user) = std::env::var("USER") {
            cmd.env("USER", user);
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CliError::InvalidArg("missing stdout".into()))?;
        let stderr = child.stderr.take();

        let shared = Arc::new(Mutex::new(SharedLoginData {
            phase: LoginPhase::Starting,
            method: method.to_string(),
            ..Default::default()
        }));

        // Background: read stdout
        let shared_out = shared.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                info!(target: "grok_login", "{l}");
                let mut s = shared_out.lock().await;
                s.output.push_str(&l);
                s.output.push('\n');
                cap_output(&mut s.output);
                apply_line(&l, &mut s);
                if s.login_url.is_some() {
                    s.phase = LoginPhase::AwaitingBrowser;
                }
            }
            let mut s = shared_out.lock().await;
            s.done = true;
        });

        // Background: read stderr
        if let Some(err) = stderr {
            let shared_err = shared.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    warn!(target: "grok_login", "stderr: {l}");
                    let mut s = shared_err.lock().await;
                    s.output.push_str(&l);
                    s.output.push('\n');
                    cap_output(&mut s.output);
                    apply_line(&l, &mut s);
                    if s.login_url.is_some() {
                        s.phase = LoginPhase::AwaitingBrowser;
                    }
                }
            });
        }

        // Wait briefly for URL/code to appear
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let s = shared.lock().await;
            if s.login_url.is_some() || s.confirm_code.is_some() || s.done {
                break;
            }
        }

        // Open browser once
        {
            let s = shared.lock().await;
            if let Some(ref url) = s.login_url {
                let _ = open_url(url).await;
            }
        }

        {
            let mut s = shared.lock().await;
            if s.login_url.is_some() || s.confirm_code.is_some() {
                s.phase = LoginPhase::AwaitingBrowser;
            }
        }

        *self.active.lock().await = Some(ActiveLogin {
            child,
            stdin,
            shared,
        });

        Ok(self.state().await)
    }

    /// Paste a verification code from the browser into the login process.
    pub async fn submit_code(&self, code: &str) -> Result<LoginSessionState> {
        let code = code.trim();
        if code.is_empty() {
            return Err(CliError::InvalidArg("code is empty".into()));
        }

        {
            let mut g = self.active.lock().await;
            let Some(ref mut active) = *g else {
                return Err(CliError::InvalidArg(
                    "No login in progress. Click Log in with Grok first.".into(),
                ));
            };

            let Some(ref mut stdin) = active.stdin else {
                return Err(CliError::InvalidArg(
                    "Login process is not accepting input. Try logging in again.".into(),
                ));
            };

            // Send code; also try common variants (with/without newline already)
            let payload = format!("{code}\n");
            stdin.write_all(payload.as_bytes()).await?;
            let _ = stdin.flush().await;
            let mut s = active.shared.lock().await;
            s.output
                .push_str("\n[Bomb Code] submitted verification code\n");
            info!("submitted verification code to grok login");
        }

        // Poll for success
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let auth = GrokCli::auth_status();
            if auth.logged_in {
                if let Some(ref mut a) = *self.active.lock().await {
                    let mut s = a.shared.lock().await;
                    s.phase = LoginPhase::Completed;
                    s.done = true;
                }
                break;
            }
            let st = self.state().await;
            if matches!(st.phase, LoginPhase::Completed | LoginPhase::Failed) {
                break;
            }
        }

        Ok(self.state().await)
    }

    pub async fn open_login_url(&self) -> Result<Option<String>> {
        let st = self.state().await;
        if let Some(ref url) = st.login_url {
            open_url(url).await?;
            return Ok(Some(url.clone()));
        }
        Ok(None)
    }
}

impl GrokCli {
    pub fn auth_file_path() -> PathBuf {
        directories::UserDirs::new()
            .map(|u| u.home_dir().join(".grok").join("auth.json"))
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".grok").join("auth.json"))
            })
            .unwrap_or_else(|| PathBuf::from(".grok/auth.json"))
    }

    pub fn auth_status() -> AuthStatus {
        let path = Self::auth_file_path();
        let auth_file = path.display().to_string();
        if !path.exists() {
            return AuthStatus {
                logged_in: false,
                auth_file,
                message: "Not signed in. Use Log in with Grok.".into(),
                ..Default::default()
            };
        }

        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                return AuthStatus {
                    logged_in: false,
                    auth_file,
                    message: format!("Could not read auth file: {e}"),
                    ..Default::default()
                };
            }
        };

        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return AuthStatus {
                logged_in: false,
                auth_file,
                message: "Auth file is not valid JSON.".into(),
                ..Default::default()
            };
        };

        let entry = value
            .as_object()
            .and_then(|m| m.values().next())
            .cloned()
            .or(Some(value));

        let Some(obj) = entry.as_ref().and_then(|v| v.as_object()) else {
            return AuthStatus {
                logged_in: false,
                auth_file,
                message: "No credentials found.".into(),
                ..Default::default()
            };
        };

        let email = obj
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let has_key = obj
            .get("key")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
            || obj
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());

        let logged_in = has_key || email.is_some();
        AuthStatus {
            logged_in,
            email: email.clone(),
            auth_mode: obj
                .get("auth_mode")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            team_id: obj
                .get("team_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            first_name: obj
                .get("first_name")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            expires_at: obj
                .get("expires_at")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            oidc_issuer: obj
                .get("oidc_issuer")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            auth_file,
            message: if logged_in {
                format!(
                    "Signed in as {}",
                    email.unwrap_or_else(|| "Grok user".into())
                )
            } else {
                "Not signed in.".into()
            },
        }
    }

    pub async fn logout(&self) -> Result<AuthStatus> {
        if !self.grok_path.exists() {
            return Err(CliError::BinaryNotFound(
                self.grok_path.display().to_string(),
            ));
        }
        let mut cmd = Command::new(&self.grok_path);
        cmd.args(["logout"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        self.apply_env(&mut cmd);

        match tokio::time::timeout(Duration::from_secs(30), cmd.output()).await {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                info!(%stdout, %stderr, "grok logout finished");
            }
            Ok(Err(e)) => warn!(error = %e, "grok logout failed"),
            Err(_) => warn!("grok logout timed out"),
        }
        Ok(Self::auth_status())
    }
}

/// Keep the captured login output bounded — a chatty CLI must not grow
/// this buffer without limit for the lifetime of the login session.
fn cap_output(out: &mut String) {
    const MAX: usize = 64 * 1024;
    if out.len() > MAX {
        let keep = out.len() - MAX / 2;
        let cut = out
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= keep)
            .unwrap_or(0);
        out.replace_range(..cut, "[…truncated…]\n");
    }
}

fn apply_line(l: &str, s: &mut SharedLoginData) {
    if let Some(url) = extract_url(l) {
        if s.login_url.is_none() {
            s.login_url = Some(url.clone());
        }
        if let Some(c) = extract_user_code(&url) {
            s.confirm_code = Some(c);
        }
    }
    if let Some(c) = extract_code_line(l) {
        s.confirm_code = Some(c);
    }
    let t = l.trim();
    if s.confirm_code.is_none() && looks_like_device_code(t) {
        s.confirm_code = Some(t.to_string());
    }
}

fn extract_url(line: &str) -> Option<String> {
    let line = line.trim();
    for prefix in ["https://", "http://"] {
        if let Some(idx) = line.find(prefix) {
            let rest = &line[idx..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
    }
    None
}

fn extract_user_code(url: &str) -> Option<String> {
    url.split("user_code=")
        .nth(1)
        .map(|s| s.split('&').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_code_line(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    for key in ["user_code", "device code", "code"] {
        if let Some(idx) = lower.find(key) {
            let rest = line[idx + key.len()..].trim_start_matches([' ', ':', '=', '-']);
            let code = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
            if looks_like_device_code(code) {
                return Some(code.to_string());
            }
        }
    }
    None
}

fn looks_like_device_code(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 5 || s.len() > 24 {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && s.contains('-')
}

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        s[s.len() - max..].to_string()
    }
}

async fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open").arg(url).status().await?;
        if !status.success() {
            warn!(%url, "open failed");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("xdg-open").arg(url).status().await?;
        if !status.success() {
            warn!(%url, "xdg-open failed");
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()
            .await?;
        if !status.success() {
            warn!(%url, "start failed");
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_url() {
        let u = "https://accounts.x.ai/oauth2/device?user_code=TNM4-KB9X";
        assert_eq!(extract_user_code(u).as_deref(), Some("TNM4-KB9X"));
        assert!(looks_like_device_code("TNM4-KB9X"));
    }

    #[test]
    fn auth_status_reads_file_or_empty() {
        let s = GrokCli::auth_status();
        assert!(!s.auth_file.is_empty());
    }
}
