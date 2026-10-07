//! Canonical workspace paths are opened relative to directory handles, without
//! following a symlink installed between validation and the actual operation.
use std::{fs::File, path::Path};
use crate::{AcpError, Result};

#[cfg(unix)]
pub(crate) fn open(root: &Path, absolute: &Path, write: bool) -> Result<File> {
    use std::{ffi::CString, os::unix::{ffi::OsStrExt, fs::MetadataExt, io::{AsRawFd, FromRawFd}}};
    let relative = absolute.strip_prefix(root)
        .map_err(|_| AcpError::Protocol("path outside workspace".into()))?;
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty() || parts.iter().any(|p| !matches!(p, std::path::Component::Normal(_))) {
        return Err(AcpError::Protocol("invalid workspace file path".into()));
    }
    fn cstr(path: &std::ffi::OsStr) -> Result<CString> {
        CString::new(path.as_bytes()).map_err(|_| AcpError::Protocol("path contains NUL".into()))
    }
    fn file(fd: i32) -> Result<File> {
        if fd < 0 { Err(std::io::Error::last_os_error().into()) }
        else { Ok(unsafe { File::from_raw_fd(fd) }) }
    }
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let mut directory = file(unsafe { libc::open(cstr(root.as_os_str())?.as_ptr(), flags) })?;
    for part in &parts[..parts.len() - 1] {
        let name = cstr(part.as_os_str())?;
        let mut fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 && write && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            let created = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o755) };
            if created != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
                return Err(std::io::Error::last_os_error().into());
            }
            fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        }
        directory = file(fd)?;
    }
    let name = cstr(parts.last().expect("nonempty checked").as_os_str())?;
    let flags = libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK
        | if write { libc::O_WRONLY | libc::O_CREAT } else { libc::O_RDONLY };
    let opened = file(unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o644) })?;
    let metadata = opened.metadata()?;
    if !metadata.is_file() || (write && metadata.nlink() != 1) {
        return Err(AcpError::Protocol("host file operation requires a regular, singly linked write target".into()));
    }
    // Check links/type BEFORE truncating: otherwise a pre-existing hard link
    // could alter a file outside the admitted workspace.
    if write { opened.set_len(0)?; }
    Ok(opened)
}

#[cfg(not(unix))]
pub(crate) fn open(_root: &Path, _absolute: &Path, _write: bool) -> Result<File> {
    Err(AcpError::Protocol("host filesystem isolation unavailable on this platform".into()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{io::{Read, Write}, os::unix::fs::symlink};
    #[test]
    fn parent_and_leaf_symlinks_cannot_escape_even_after_validation() {
        let inside = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = inside.path().canonicalize().unwrap();
        symlink(outside.path(), root.join("sub")).unwrap();
        assert!(open(&root, &root.join("sub/new.txt"), true).is_err());
        symlink(outside.path().join("new.txt"), root.join("leaf")).unwrap();
        assert!(open(&root, &root.join("leaf"), true).is_err());
        assert!(!outside.path().join("new.txt").exists());
    }
    #[test]
    fn hard_link_is_rejected_before_truncation() {
        let inside = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let original = outside.path().join("original");
        std::fs::write(&original, "preserved").unwrap();
        let root = inside.path().canonicalize().unwrap();
        std::fs::hard_link(&original, root.join("linked")).unwrap();
        assert!(open(&root, &root.join("linked"), true).is_err());
        assert_eq!(std::fs::read_to_string(original).unwrap(), "preserved");
    }
    #[test]
    fn regular_nested_write_and_read_work() {
        let inside = tempfile::tempdir().unwrap();
        let root = inside.path().canonicalize().unwrap();
        let path = root.join("a/b/file");
        open(&root, &path, true).unwrap().write_all(b"content").unwrap();
        let mut result = String::new();
        open(&root, &path, false).unwrap().read_to_string(&mut result).unwrap();
        assert_eq!(result, "content");
    }
}
