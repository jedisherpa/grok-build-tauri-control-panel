//! Which agent services are actually usable right now.
//!
//! Each backend owns its own credentials — we never store any. This module only
//! *asks*: is the CLI installed, and does it consider itself signed in?
//!
//! - grok:   `~/.grok/auth.json` (see [`crate::auth`]) or `XAI_API_KEY`
//! - claude: `claude auth status --json` or `ANTHROPIC_API_KEY`
//! - codex:  `codex login status` / `$CODEX_HOME/auth.json` or `OPENAI_API_KEY`

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::GrokCli;

/// How a backend proved it is signed in. `ApiKey` means an env var is doing the
/// work, so there is nothing to sign in or out of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Subscription,
    ApiKey,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendAuth {
    /// "grok" | "claude" | "codex"
    pub backend: String,
    pub display_name: String,
    /// The CLI that owns the credentials is on PATH.
    pub installed: bool,
    pub logged_in: bool,
    pub kind: AuthKind,
    /// Signed-in identity, when the CLI reports one.
    pub account: Option<String>,
    /// Plan / auth-method detail, e.g. "max" or "claude.ai".
    pub plan: Option<String>,
    /// One line for the UI: who you are, or why you cannot use this backend.
    pub message: String,
    /// Shell command that signs in, when a CLI is installed to run it.
    pub login_command: Option<String>,
    /// The panel can drive this login itself (device flow); otherwise we hand
    /// off to a terminal because the CLI needs a TTY + browser.
    pub in_app_login: bool,
    /// How to get the CLI when it is missing.
    pub install_hint: Option<String>,
}

impl BackendAuth {
    fn missing(backend: &str, display: &str, install: &str) -> Self {
        Self {
            backend: backend.into(),
            display_name: display.into(),
            installed: false,
            logged_in: false,
            kind: AuthKind::None,
            account: None,
            plan: None,
            message: format!("{display} CLI not installed"),
            login_command: None,
            in_app_login: false,
            install_hint: Some(install.into()),
        }
    }
}

fn which(bin: &str) -> Option<PathBuf> {
    which::which(bin).ok()
}

fn env_key(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.trim().is_empty())
}

/// Run a CLI's own status probe. Backends are slow to boot, so we cap the wait;
/// a timeout is reported as "unknown", never as "signed out".
async fn probe(bin: &PathBuf, args: &[&str]) -> Option<String> {
    let fut = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let out = tokio::time::timeout(Duration::from_secs(10), fut)
        .await
        .ok()?
        .ok()?;
    String::from_utf8(out.stdout).ok()
}

/// Status for all three backends, probed concurrently.
pub async fn all() -> Vec<BackendAuth> {
    let (grok, claude, codex) = tokio::join!(grok(), claude(), codex());
    vec![grok, claude, codex]
}

pub async fn grok() -> BackendAuth {
    let installed = which("grok").is_some() || GrokCli::auth_file_path().exists();
    if !installed {
        return BackendAuth::missing("grok", "Grok", "curl -fsSL https://grok.com/install.sh | sh");
    }
    // The grok backend accepts a raw xAI key with no login at all.
    if env_key("XAI_API_KEY") {
        return BackendAuth {
            backend: "grok".into(),
            display_name: "Grok".into(),
            installed: true,
            logged_in: true,
            kind: AuthKind::ApiKey,
            account: None,
            plan: Some("XAI_API_KEY".into()),
            message: "Using XAI_API_KEY from the environment".into(),
            login_command: None,
            in_app_login: false,
            install_hint: None,
        };
    }

    let st = GrokCli::auth_status();
    BackendAuth {
        backend: "grok".into(),
        display_name: "Grok".into(),
        installed: true,
        logged_in: st.logged_in,
        kind: if st.logged_in { AuthKind::Subscription } else { AuthKind::None },
        account: st.email.clone(),
        plan: st.auth_mode.clone(),
        message: if st.logged_in {
            st.email.clone().unwrap_or_else(|| "Signed in".into())
        } else {
            "Not signed in".into()
        },
        login_command: Some("grok login".into()),
        in_app_login: true, // device-code flow, driven by the panel
        install_hint: None,
    }
}

pub async fn claude() -> BackendAuth {
    let Some(bin) = which("claude") else {
        return BackendAuth::missing(
            "claude",
            "Claude Code",
            "npm i -g @anthropic-ai/claude-code",
        );
    };
    if env_key("ANTHROPIC_API_KEY") {
        return BackendAuth {
            backend: "claude".into(),
            display_name: "Claude Code".into(),
            installed: true,
            logged_in: true,
            kind: AuthKind::ApiKey,
            account: None,
            plan: Some("ANTHROPIC_API_KEY".into()),
            message: "Using ANTHROPIC_API_KEY from the environment".into(),
            login_command: None,
            in_app_login: false,
            install_hint: None,
        };
    }

    // `claude auth status --json` is the CLI's own answer; on macOS the token
    // lives in the Keychain, so there is no file we could read instead.
    let raw = probe(&bin, &["auth", "status", "--json"]).await;
    let v: Option<serde_json::Value> = raw.as_deref().and_then(|s| serde_json::from_str(s).ok());
    let logged_in = v
        .as_ref()
        .and_then(|v| v.get("loggedIn"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let account = v
        .as_ref()
        .and_then(|v| v.get("email"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let plan = v
        .as_ref()
        .and_then(|v| v.get("subscriptionType").or_else(|| v.get("authMethod")))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    BackendAuth {
        backend: "claude".into(),
        display_name: "Claude Code".into(),
        installed: true,
        logged_in,
        kind: if logged_in { AuthKind::Subscription } else { AuthKind::None },
        account: account.clone(),
        plan,
        message: if logged_in {
            account.unwrap_or_else(|| "Signed in".into())
        } else {
            "Not signed in".into()
        },
        login_command: Some("claude auth login".into()),
        in_app_login: false, // needs a TTY + browser: hand off to a terminal
        install_hint: None,
    }
}

pub async fn codex() -> BackendAuth {
    let Some(bin) = which("codex") else {
        return BackendAuth::missing("codex", "Codex", "npm i -g @openai/codex");
    };
    if env_key("OPENAI_API_KEY") {
        return BackendAuth {
            backend: "codex".into(),
            display_name: "Codex".into(),
            installed: true,
            logged_in: true,
            kind: AuthKind::ApiKey,
            account: None,
            plan: Some("OPENAI_API_KEY".into()),
            message: "Using OPENAI_API_KEY from the environment".into(),
            login_command: None,
            in_app_login: false,
            install_hint: None,
        };
    }

    // `codex login status` prints a human line ("Logged in using ChatGPT" /
    // "Not logged in"); the auth file is the durable fallback.
    let raw = probe(&bin, &["login", "status"]).await.unwrap_or_default();
    let low = raw.to_lowercase();
    let said_yes = low.contains("logged in") && !low.contains("not logged in");
    let logged_in = said_yes || codex_auth_file().is_some_and(|p| p.exists());

    let account = raw
        .split_whitespace()
        .find(|w| w.contains('@') && w.contains('.'))
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_graphic() || c == ',').to_string());

    BackendAuth {
        backend: "codex".into(),
        display_name: "Codex".into(),
        installed: true,
        logged_in,
        kind: if logged_in { AuthKind::Subscription } else { AuthKind::None },
        account: account.clone(),
        plan: logged_in.then(|| "ChatGPT".into()),
        message: if logged_in {
            account.unwrap_or_else(|| "Signed in".into())
        } else {
            "Not signed in".into()
        },
        login_command: Some("codex login".into()),
        in_app_login: false,
        install_hint: None,
    }
}

fn codex_auth_file() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))?;
    Some(home.join("auth.json"))
}

/// The sign-in command for a backend, or `None` if it has no CLI login (env-key
/// backends, or a CLI that is not installed).
pub fn login_command(backend: &str) -> Option<&'static str> {
    match backend {
        "grok" => Some("grok login"),
        "claude" => Some("claude auth login"),
        "codex" => Some("codex login"),
        _ => None,
    }
}

pub fn logout_command(backend: &str) -> Option<&'static str> {
    match backend {
        "grok" => Some("grok logout"),
        "claude" => Some("claude auth logout"),
        "codex" => Some("codex logout"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_backend_reports_install_hint() {
        let b = BackendAuth::missing("codex", "Codex", "npm i -g @openai/codex");
        assert!(!b.installed && !b.logged_in);
        assert_eq!(b.kind, AuthKind::None);
        assert!(b.install_hint.is_some());
        assert!(b.login_command.is_none(), "cannot log in without the CLI");
    }

    #[test]
    fn login_commands_cover_every_backend() {
        for b in ["grok", "claude", "codex"] {
            assert!(login_command(b).is_some(), "{b} has no login command");
            assert!(logout_command(b).is_some(), "{b} has no logout command");
        }
        assert!(login_command("nope").is_none());
    }
}
