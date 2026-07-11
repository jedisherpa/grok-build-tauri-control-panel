//! Sandbox profile definitions for agent spawn isolation.

use serde::{Deserialize, Serialize};

/// Sandbox profiles mapped from the research report.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProfile {
    /// Full workspace read/write (default interactive).
    #[default]
    Workspace,
    /// Read-only workspace; writes require approval or fail.
    ReadOnly,
    /// Strict: minimal FS, no network (where OS supports it).
    Strict,
    /// Unrestricted — only for trusted repos with explicit user consent.
    Unrestricted,
}

impl SandboxProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::ReadOnly => "read_only",
            Self::Strict => "strict",
            Self::Unrestricted => "unrestricted",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "workspace" => Self::Workspace,
            "read_only" | "readonly" | "read-only" => Self::ReadOnly,
            "strict" => Self::Strict,
            "unrestricted" | "none" | "off" => Self::Unrestricted,
            _ => Self::Workspace,
        }
    }

    /// Whether writes are allowed without extra approval.
    pub fn allows_writes(self) -> bool {
        matches!(self, Self::Workspace | Self::Unrestricted)
    }

    /// Whether network is expected to be restricted at OS level.
    pub fn restricts_network(self) -> bool {
        matches!(self, Self::Strict | Self::ReadOnly)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_profiles() {
        assert_eq!(SandboxProfile::from_str_lossy("strict"), SandboxProfile::Strict);
        assert_eq!(SandboxProfile::from_str_lossy("read-only"), SandboxProfile::ReadOnly);
        assert!(!SandboxProfile::Strict.allows_writes());
        assert!(SandboxProfile::Workspace.allows_writes());
    }
}
