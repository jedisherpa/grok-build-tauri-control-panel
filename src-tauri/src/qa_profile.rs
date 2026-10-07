//! Operator-selected QA stores must never alias normal native/browser stores.
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
#[cfg(not(target_os = "macos"))]
use std::path::PathBuf;

pub(crate) struct BrowserIsolation {
    #[cfg(not(target_os = "macos"))]
    pub directory: PathBuf,
    pub identifier: [u8; 16],
}

pub(crate) fn discover() -> Result<Option<BrowserIsolation>> {
    let Some(profile) = std::env::var_os("C3_PROFILE_DIR") else {
        return Ok(None);
    };
    // Path discovery reads no configuration and creates no stores.
    let paths = grok_config::GrokPaths::discover(None)?;
    #[cfg(target_os = "macos")]
    let macos_major = {
        let result = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .context("identify macOS before creating a QA webview")?;
        if !result.status.success() || result.stdout.len() > 64 {
            bail!("cannot verify macOS support for isolated QA browser storage");
        }
        Some(parse_macos_major(std::str::from_utf8(&result.stdout)?)?)
    };
    #[cfg(not(target_os = "macos"))]
    let macos_major = None;
    for_profile(Path::new(&profile), &paths.home_dir, macos_major).map(Some)
}

fn for_profile(profile: &Path, home: &Path, macos_major: Option<u32>) -> Result<BrowserIsolation> {
    if !profile.is_absolute() || !profile.is_dir() {
        bail!("C3_PROFILE_DIR must be an existing absolute QA directory; create it explicitly before launch");
    }
    // Wry falls back to the default store before macOS 14: reject that fallback.
    if macos_major.is_some_and(|version| version < 14) {
        bail!("isolated persistent QA browser storage requires macOS 14 or later; use an isolated macOS account for older systems");
    }
    let root = profile.canonicalize().context("resolve QA profile")?;
    let home = home.canonicalize().context("resolve normal home")?;
    if home.starts_with(&root) {
        bail!("QA profile must not be the normal home directory or one of its ancestors");
    }
    for relative in [
        ".grok",
        ".grok/control-panel",
        ".grok/worktrees",
        ".grok/control-panel/config.toml",
        ".grok/control-panel/sessions",
        ".grok/control-panel/sessions/control_panel.db",
        ".grok/control-panel/sessions/control_panel.db-wal",
        ".grok/control-panel/sessions/control_panel.db-shm",
        ".grok/control-panel/memory",
        ".grok/control-panel/memory/memory.json",
        ".grok/config.toml",
        ".grok/mcp_credentials.json",
        ".webview",
    ] {
        validate_existing_path(&root, &root.join(relative))?;
    }
    validate_existing_tree(&root, &root.join(".grok"))?;
    let mut hasher = Sha256::new();
    hasher.update(b"see-cubed/qa-browser-store/v1\0");
    // Use OS bytes so different non-UTF-8 roots cannot collapse via lossy display.
    hasher.update(root.as_os_str().as_encoded_bytes());
    let hash = hasher.finalize();
    let mut identifier = [0; 16];
    identifier.copy_from_slice(&hash[..16]);
    identifier[6] = (identifier[6] & 0x0f) | 0x50;
    identifier[8] = (identifier[8] & 0x3f) | 0x80;
    Ok(BrowserIsolation {
        #[cfg(not(target_os = "macos"))]
        directory: root.join(".webview"),
        identifier,
    })
}

fn validate_existing_tree(root: &Path, subtree: &Path) -> Result<()> {
    // Include future/optional native stores, not merely today's known leaves.
    // Bound traversal and resolve in-profile aliases without following cycles.
    let mut pending = vec![subtree.to_path_buf()];
    let mut visited = std::collections::HashSet::new();
    let mut inspected = 0;
    while let Some(path) = pending.pop() {
        if std::fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            continue;
        }
        inspected += 1;
        if inspected > 100_000 {
            bail!("QA profile exceeds the verified native-store traversal budget");
        }
        validate_existing_path(root, &path)?;
        let canonical = path.canonicalize()?;
        if canonical.is_dir() && visited.insert(canonical.clone()) {
            for child in std::fs::read_dir(canonical)? {
                pending.push(child?.path());
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_worktree_root(
    paths: &grok_config::GrokPaths,
    configured: &Path,
) -> Result<()> {
    if std::env::var_os("C3_PROFILE_DIR").is_none() {
        return Ok(());
    }
    let root = paths
        .grok_dir
        .parent()
        .context("QA profile has no root")?
        .canonicalize()?;
    validate_configured_root(&root, configured)
}

fn validate_configured_root(root: &Path, configured: &Path) -> Result<()> {
    // Resolve existing ancestors so an alias of the selected profile works.
    // Reject parent components before a missing descendant can hide traversal.
    if !configured.is_absolute()
        || configured
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("QA worktrees_root must stay inside C3_PROFILE_DIR");
    }
    validate_existing_path(root, configured)
}

fn validate_existing_path(root: &Path, candidate: &Path) -> Result<()> {
    let mut existing = candidate;
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .context("QA path has no existing ancestor")?;
            }
            Err(error) => return Err(error).context("inspect QA store path"),
        }
    }
    let resolved = existing.canonicalize().context("resolve QA store path")?;
    if !resolved.starts_with(root) {
        bail!(
            "QA store resolves outside its selected profile: {}",
            candidate.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&resolved)?;
        if metadata.is_file() && metadata.nlink() > 1 {
            bail!(
                "QA store has multiple hard links; independent ownership cannot be verified: {}",
                candidate.display()
            );
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_major(value: &str) -> Result<u32> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || value
            .split('.')
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        bail!("unrecognized macOS version; refusing shared QA browser fallback");
    }
    value
        .split('.')
        .next()
        .context("missing macOS version")?
        .parse()
        .context("invalid macOS major version")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qa_identity_is_stable_distinct_and_alias_aware() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        for root in [&home, &first, &second] {
            std::fs::create_dir(root).unwrap();
        }
        let a = for_profile(&first, &home, Some(14)).unwrap();
        assert_eq!(
            a.identifier,
            for_profile(&first, &home, Some(26)).unwrap().identifier
        );
        assert_ne!(
            a.identifier,
            for_profile(&second, &home, Some(14)).unwrap().identifier
        );
        assert!(for_profile(&home, &home, Some(14)).is_err());
        assert!(for_profile(&first, &home, Some(13)).is_err());
        #[cfg(unix)]
        {
            let alias = dir.path().join("alias");
            std::os::unix::fs::symlink(&first, &alias).unwrap();
            assert_eq!(
                a.identifier,
                for_profile(&alias, &home, Some(14)).unwrap().identifier
            );
            std::os::unix::fs::symlink(home.join(".grok"), first.join(".grok")).unwrap();
            assert!(for_profile(&first, &home, Some(14)).is_err());
            assert!(!home.join(".grok").exists());
        }
    }
    #[test]
    fn invalid_version_and_missing_profile_fail_closed() {
        for text in ["", "unknown", "14.beta", "14..0", "-1", "14\n15"] {
            assert!(parse_macos_major(text).is_err());
        }
        assert_eq!(parse_macos_major("26.2.0\n").unwrap(), 26);
        let dir = tempfile::tempdir().unwrap();
        assert!(for_profile(&dir.path().join("missing"), dir.path(), Some(14)).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn existing_database_symlink_and_hardlink_cannot_alias_another_store() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let profile = dir.path().join("profile");
        let sessions = profile.join(".grok/control-panel/sessions");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir_all(&sessions).unwrap();
        let original = home.join("original.db");
        std::fs::write(&original, b"preserve original bytes").unwrap();
        let target = sessions.join("control_panel.db");
        std::os::unix::fs::symlink(&original, &target).unwrap();
        assert!(for_profile(&profile, &home, Some(14)).is_err());
        std::fs::remove_file(&target).unwrap();
        std::fs::hard_link(&original, &target).unwrap();
        assert!(for_profile(&profile, &home, Some(14)).is_err());
        assert_eq!(
            std::fs::read(&original).unwrap(),
            b"preserve original bytes"
        );
    }
    #[cfg(unix)]
    #[test]
    fn optional_future_stores_and_home_ancestors_are_checked_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let profile = dir.path().join("profile");
        let library = profile.join(".grok/control-panel/history");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        let original = home.join("original.sqlite");
        std::fs::write(&original, b"unchanged").unwrap();
        std::os::unix::fs::symlink(&original, library.join("library.sqlite")).unwrap();
        assert!(for_profile(&profile, &home, Some(14)).is_err());
        assert!(for_profile(dir.path(), &home, Some(14)).is_err());
        assert_eq!(std::fs::read(original).unwrap(), b"unchanged");
    }
    #[cfg(unix)]
    #[test]
    fn worktree_configuration_accepts_profile_alias_but_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("profile");
        let alias = dir.path().join("alias");
        std::fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        let canonical = root.canonicalize().unwrap();
        assert!(validate_configured_root(&canonical, &alias.join("new/worktrees")).is_ok());
        assert!(validate_configured_root(&canonical, &root.join("missing/../../outside")).is_err());
        assert!(validate_configured_root(&canonical, &dir.path().join("outside")).is_err());
        std::os::unix::fs::symlink(dir.path(), root.join("escape")).unwrap();
        assert!(validate_configured_root(&canonical, &root.join("escape/new")).is_err());
    }
    #[test]
    fn config_cannot_create_a_default_browser_before_isolation() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows = config["app"]["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0]["create"], false);
        assert_eq!(windows[0]["label"], "main");
    }
}
