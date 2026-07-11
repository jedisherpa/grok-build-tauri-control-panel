//! Runtime environment bootstrap for GUI launches (macOS .app has a minimal PATH).

use std::env;
use std::path::PathBuf;

/// Ensure HOME/PATH are usable when launched from Finder / Dock.
pub fn bootstrap_process_env() {
    if env::var_os("HOME").is_none() {
        if let Some(home) = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()) {
            // SAFETY: process-global env mutation at startup only.
            env::set_var("HOME", home);
        }
    }

    let home = env::var("HOME").unwrap_or_default();
    let extras = [
        format!("{home}/.grok/bin"),
        format!("{home}/.cargo/bin"),
        format!("{home}/.local/bin"),
        "/opt/homebrew/bin".to_string(),
        "/opt/homebrew/sbin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/bin".to_string(),
        "/bin".to_string(),
        "/usr/sbin".to_string(),
        "/sbin".to_string(),
    ];

    let current = env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = current
        .split(':')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();

    for extra in extras {
        if PathBuf::from(&extra).is_dir() && !parts.iter().any(|p| p == &extra) {
            parts.insert(0, extra);
        }
    }

    env::set_var("PATH", parts.join(":"));
}

/// Preferred absolute path to the grok binary for GUI apps.
pub fn preferred_grok_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs_home() {
        out.push(home.join(".grok").join("bin").join("grok"));
        out.push(home.join(".cargo").join("bin").join("grok"));
        out.push(home.join(".local").join("bin").join("grok"));
    }
    out.push(PathBuf::from("/opt/homebrew/bin/grok"));
    out.push(PathBuf::from("/usr/local/bin/grok"));
    out
}

fn dirs_home() -> Option<PathBuf> {
    directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
}

/// Build an augmented PATH string for child processes.
pub fn child_path_env() -> String {
    bootstrap_process_env();
    env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
}
