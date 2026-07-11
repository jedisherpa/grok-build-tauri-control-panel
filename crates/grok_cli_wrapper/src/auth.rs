//! Grok authentication helpers (status from ~/.grok/auth.json + CLI login/logout).

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub status: AuthStatus,
    pub login_url: Option<String>,
    pub user_code: Option<String>,
    pub method: String,
    pub output: String,
}

impl GrokCli {
    pub fn auth_file_path() -> PathBuf {
        directories::UserDirs::new()
            .map(|u| u.home_dir().join(".grok").join("auth.json"))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".grok").join("auth.json")))
            .unwrap_or_else(|| PathBuf::from(".grok/auth.json"))
    }

    /// Read non-secret identity fields from ~/.grok/auth.json.
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

        // File is a map of issuer::client_id -> profile objects
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
        let has_key = obj.get("key").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty())
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

    /// Browser OAuth via `grok login --oauth`. Opens/prints a URL; waits for completion.
    pub async fn login_oauth(&self, timeout: Duration) -> Result<LoginResult> {
        self.run_login(&["login", "--oauth"], "oauth", timeout).await
    }

    /// Device-code login via `grok login --device-auth` (good for GUI clients).
    pub async fn login_device(&self, timeout: Duration) -> Result<LoginResult> {
        self.run_login(&["login", "--device-auth"], "device", timeout)
            .await
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

    async fn run_login(
        &self,
        args: &[&str],
        method: &str,
        timeout: Duration,
    ) -> Result<LoginResult> {
        if !self.grok_path.exists() {
            return Err(CliError::BinaryNotFound(
                self.grok_path.display().to_string(),
            ));
        }

        let mut cmd = Command::new(&self.grok_path);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        self.apply_env(&mut cmd);

        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CliError::InvalidArg("missing stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CliError::InvalidArg("missing stderr".into()))?;

        let mut reader = BufReader::new(stdout).lines();
        let mut err_reader = BufReader::new(stderr).lines();
        let mut output = String::new();
        let mut login_url: Option<String> = None;
        let mut user_code: Option<String> = None;

        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                let _ = child.kill().await;
                return Err(CliError::Timeout(timeout));
            }

            tokio::select! {
                line = reader.next_line() => {
                    match line? {
                        Some(l) => {
                            output.push_str(&l);
                            output.push('\n');
                            info!(target: "grok_login", "{l}");
                            if let Some(url) = extract_url(&l) {
                                login_url = Some(url.clone());
                                if let Some(code) = extract_user_code(&url) {
                                    user_code = Some(code);
                                }
                                // Open browser as soon as we have a URL.
                                let _ = open_url(&url).await;
                            }
                            if user_code.is_none() {
                                if let Some(code) = extract_standalone_code(&l) {
                                    user_code = Some(code);
                                }
                            }
                        }
                        None => break,
                    }
                }
                line = err_reader.next_line() => {
                    if let Ok(Some(l)) = line {
                        output.push_str(&l);
                        output.push('\n');
                        warn!(target: "grok_login", "stderr: {l}");
                        if login_url.is_none() {
                            if let Some(url) = extract_url(&l) {
                                login_url = Some(url.clone());
                                let _ = open_url(&url).await;
                            }
                        }
                    }
                }
                status = child.wait() => {
                    let _ = status?;
                    break;
                }
            }
        }

        // Drain remaining output, then ensure process is reaped
        while let Ok(Some(l)) = reader.next_line().await {
            output.push_str(&l);
            output.push('\n');
            if login_url.is_none() {
                if let Some(url) = extract_url(&l) {
                    login_url = Some(url.clone());
                    let _ = open_url(&url).await;
                }
            }
        }
        let _ = child.wait().await;

        // Give auth.json a moment to flush
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut status = Self::auth_status();
        if status.logged_in {
            status.message = format!("Signed in as {}", status.email.clone().unwrap_or_default());
        } else if login_url.is_some() {
            status.message =
                "Login started — complete sign-in in the browser, then refresh status.".into();
        }

        Ok(LoginResult {
            status,
            login_url,
            user_code,
            method: method.to_string(),
            output,
        })
    }
}

fn extract_url(line: &str) -> Option<String> {
    let line = line.trim();
    if let Some(idx) = line.find("https://") {
        let rest = &line[idx..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        return Some(rest[..end].to_string());
    }
    if let Some(idx) = line.find("http://") {
        let rest = &line[idx..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        return Some(rest[..end].to_string());
    }
    None
}

fn extract_user_code(url: &str) -> Option<String> {
    // https://accounts.x.ai/oauth2/device?user_code=TNM4-KB9X
    url.split("user_code=")
        .nth(1)
        .map(|s| {
            s.split('&')
                .next()
                .unwrap_or(s)
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
}

fn extract_standalone_code(line: &str) -> Option<String> {
    // e.g. "code: ABCD-EFGH" or "user_code: ABCD-EFGH"
    let lower = line.to_ascii_lowercase();
    for key in ["user_code", "code"] {
        if let Some(idx) = lower.find(key) {
            let rest = line[idx + key.len()..].trim_start_matches([' ', ':', '=']);
            let code = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
            if code.len() >= 4 {
                return Some(code.to_string());
            }
        }
    }
    None
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
        assert!(extract_url(&format!("  {u}  ")).unwrap().starts_with("https://"));
    }

    #[test]
    fn auth_status_reads_file_or_empty() {
        let s = GrokCli::auth_status();
        // Should not panic; logged_in depends on machine
        assert!(!s.auth_file.is_empty());
    }
}
