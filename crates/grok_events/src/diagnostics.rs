//! Diagnostic text hygiene for anything shown in the UI.
//!
//! Agent CLIs write coloured `tracing` output and raw provider error bodies to
//! stderr. Showing that verbatim leaks ANSI escape codes, xAI team IDs and API
//! key fragments, and buries the actual cause. Every user-facing error/stderr
//! line goes through [`sanitize_diagnostic`]; startup failures additionally get
//! a plain-language summary from [`summarize_cli_failure`].

use std::sync::OnceLock;

use regex::Regex;

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static regex"))
}

/// Remove ANSI CSI/OSC escape sequences (colours, cursor moves, hyperlinks).
pub fn strip_ansi(s: &str) -> String {
    static CSI: OnceLock<Regex> = OnceLock::new();
    static OSC: OnceLock<Regex> = OnceLock::new();
    let s = re(&OSC, r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)").replace_all(s, "");
    let s = re(&CSI, r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-Z\\-_]").replace_all(&s, "");
    s.replace('\x1b', "")
}

/// Redact credentials and account identifiers: xAI team IDs, API key
/// fragments (`xai-...KwZn`, full `xai-…` keys), `sk-…` keys and bearer tokens.
pub fn redact_secrets(s: &str) -> String {
    static TEAM: OnceLock<Regex> = OnceLock::new();
    static XAI: OnceLock<Regex> = OnceLock::new();
    static SK: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    let s = re(
        &TEAM,
        r"(?i)(/team/)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
    )
    .replace_all(s, "${1}[team]");
    let s = re(&XAI, r"xai-(?:\.{2,}|…)?[A-Za-z0-9_\-]{2,}").replace_all(&s, "xai-[redacted]");
    let s = re(&SK, r"\bsk-[A-Za-z0-9_\-]{8,}").replace_all(&s, "sk-[redacted]");
    let s =
        re(&BEARER, r"(?i)(bearer\s+)[A-Za-z0-9._~+/\-]{8,}=*").replace_all(&s, "${1}[redacted]");
    s.into_owned()
}

/// ANSI-strip then redact. Use for every diagnostic line shown in the UI.
pub fn sanitize_diagnostic(s: &str) -> String {
    redact_secrets(&strip_ansi(s))
}

/// True for structured `tracing` log lines (`2026-…Z  INFO target: …`).
fn is_log_noise(line: &str) -> bool {
    static LOG: OnceLock<Regex> = OnceLock::new();
    re(
        &LOG,
        r"^\s*\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z?\s+(?:TRACE|DEBUG|INFO)\b",
    )
    .is_match(line)
}

/// Plain-language reason for a failed agent CLI run (narrator call or ACP
/// startup), derived from its stderr. `api_key_set` says whether an
/// `XAI_API_KEY` was passed to the child. Returns `None` when nothing
/// recognizable is present; callers then fall back to the sanitized text.
pub fn summarize_cli_failure(stderr: &str, api_key_set: bool) -> Option<String> {
    let clean = sanitize_diagnostic(stderr);
    let lower = clean.to_ascii_lowercase();
    let fix = "Enable it or create a new key at console.x.ai → API keys (or use Log in with Grok in Services), then start the thread again.";
    if lower.contains("is disabled and cannot be used") {
        return Some(format!(
            "xAI rejected the API key in XAI_API_KEY: the key is disabled. {fix}"
        ));
    }
    if lower.contains("api key") && lower.contains("blocked") {
        return Some(format!(
            "xAI rejected the API key in XAI_API_KEY: the key or its team is blocked. {fix}"
        ));
    }
    if lower.contains("incorrect api key") || lower.contains("invalid api key") {
        return Some(format!(
            "xAI rejected the API key in XAI_API_KEY as invalid. {fix}"
        ));
    }
    if lower.contains("not signed in") {
        return Some(if api_key_set {
            format!("Grok couldn't sign in with the XAI_API_KEY that is set (xAI refused it — it may be disabled, expired or mistyped). {fix}")
        } else {
            "Grok isn't signed in. Use Log in with Grok in Services, or set XAI_API_KEY, then start the thread again.".to_string()
        });
    }
    // Last explicit error line, without log noise.
    clean
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_log_noise(l))
        .find(|l| l.starts_with("Error:") || l.starts_with("error:"))
        .map(|l| {
            let l = l
                .trim_start_matches("Error:")
                .trim_start_matches("error:")
                .trim();
            l.chars().take(300).collect()
        })
}

/// Sanitized, log-noise-free, length-capped version of a CLI failure for display.
pub fn display_cli_failure(raw: &str, api_key_set: bool) -> String {
    if let Some(s) = summarize_cli_failure(raw, api_key_set) {
        return s;
    }
    let clean = sanitize_diagnostic(raw);
    let kept: Vec<&str> = clean
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_log_noise(l))
        .collect();
    let joined = kept.join(" ");
    let mut out: String = joined.chars().take(400).collect();
    if joined.chars().count() > 400 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAY1: &str = "\x1b[2m2026-10-07T02:21:45.954046Z\x1b[0m \x1b[33m WARN\x1b[0m Failed to fetch models: 403 - {\"code\":\"permission-denied\",\"error\":\"The API key xai-...KwZn is disabled and cannot be used to perform requests. To enable the API key, go to https://console.x.ai/team/2c0a58ed-703f-434e-9191-a3016d3bc641/api-keys.\"}\n\x1b[2m2026-10-07T02:21:46.214881Z\x1b[0m \x1b[32m INFO\x1b[0m startup phase \x1b[3mphase\x1b[0m\x1b[2m=\x1b[0meager_auth\nError: Not signed in. To authenticate without a browser, run:\n  grok login --device-code";

    #[test]
    fn strips_ansi_sequences() {
        let s = strip_ansi("\x1b[32m INFO\x1b[0m hi \x1b]8;;http://x\x07link\x1b]8;;\x07");
        assert_eq!(s, " INFO hi link");
        assert!(!strip_ansi(PLAY1).contains('\x1b'));
    }

    #[test]
    fn redacts_team_ids_and_key_fragments() {
        let s = sanitize_diagnostic(PLAY1);
        assert!(!s.contains("2c0a58ed-703f-434e-9191-a3016d3bc641"), "{s}");
        assert!(!s.contains("KwZn"), "{s}");
        assert!(s.contains("/team/[team]/api-keys"));
        assert!(s.contains("xai-[redacted]"));
        let full = redact_secrets(
            "key=xai-AbCdEf0123456789 and Bearer abcdefghijkl.mn and sk-ABCDEFGHIJKL",
        );
        assert_eq!(
            full,
            "key=xai-[redacted] and Bearer [redacted] and sk-[redacted]"
        );
    }

    #[test]
    fn summarizes_disabled_key_first() {
        let s = summarize_cli_failure(PLAY1, true).unwrap();
        assert!(s.contains("the key is disabled"), "{s}");
        assert!(!s.contains("KwZn") && !s.contains("2c0a58ed"));
    }

    #[test]
    fn summarizes_not_signed_in_with_and_without_key() {
        let raw = "Not signed in. To authenticate without a browser, run:\n  grok login --device-code\nError: Not signed in.";
        assert!(summarize_cli_failure(raw, true)
            .unwrap()
            .contains("XAI_API_KEY that is set"));
        assert!(summarize_cli_failure(raw, false)
            .unwrap()
            .starts_with("Grok isn't signed in"));
    }

    #[test]
    fn display_drops_log_noise_and_caps_length() {
        let raw = format!(
            "\x1b[2m2026-10-07T02:21:45.862512Z\x1b[0m \x1b[32m INFO\x1b[0m startup phase\nsomething odd happened {}",
            "x".repeat(600)
        );
        let s = display_cli_failure(&raw, false);
        assert!(!s.contains("startup phase"));
        assert!(s.starts_with("something odd happened"));
        assert!(s.chars().count() <= 401);
        assert_eq!(
            summarize_cli_failure("Error: boom", false).as_deref(),
            Some("boom")
        );
    }
}
