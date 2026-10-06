//! ACP adapters and hosted terminals have their own Unix process groups so
//! shutdown also stops native engine/tool descendants before scopes release.
use crate::{AcpError, Result};
use tokio::process::Child;
use std::time::Duration;
pub(crate) async fn terminate(child: &mut Child) -> Result<()> {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // The caller created this child with process_group(0); it is never the app group.
        let result = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) { return Err(error.into()); }
        }
    }
    #[cfg(not(unix))]
    if child.id().is_some() { child.start_kill()?; }
    tokio::time::timeout(Duration::from_secs(10), child.wait()).await
        .map_err(|_| AcpError::Timeout("native process shutdown".into()))??;
    Ok(())
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shutdown_waits_for_a_dedicated_process_group() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "sleep 120 & wait"]).process_group(0).kill_on_drop(true);
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        terminate(&mut child).await.unwrap();
        assert!(child.id().is_none());
        assert_ne!(unsafe { libc::kill(-(pid as i32), 0) }, 0);
    }
}
