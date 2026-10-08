//! A cooperative store lock cannot control an older, nonparticipating binary.
//! Reject known legacy product processes before activating the normal profile.
use anyhow::{bail, Context, Result};

pub(crate) async fn require_closed_legacy_product() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let mut command = tokio::process::Command::new("/bin/ps");
        command.args(["-axo", "pid=,comm="]);
        let output = bounded_inventory(&mut command, 1024*1024, 8192, std::time::Duration::from_secs(5)).await?;
        check_process_inventory(std::str::from_utf8(&output.stdout)?,std::process::id())?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
async fn bounded_inventory(command: &mut tokio::process::Command, stdout_cap: usize, stderr_cap: usize, deadline: std::time::Duration) -> Result<std::process::Output> {
    use std::process::Stdio;
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command.spawn().context("cannot start process ownership inspection")?;
    let stdout = child.stdout.take().context("inspection stdout missing")?;
    let stderr = child.stderr.take().context("inspection stderr missing")?;
    // Read both pipes concurrently and reject at the cap, including a flood
    // without newlines. No complete unbounded output is ever allocated.
    let observed = tokio::time::timeout(deadline, async {
        tokio::try_join!(bounded_pipe(stdout, stdout_cap), bounded_pipe(stderr, stderr_cap), async { child.wait().await.map_err(anyhow::Error::from) })
    }).await;
    let failure = match observed {
        Ok(Ok((stdout, stderr, status))) => {
            if !status.success() || !stderr.is_empty() {
                bail!("cannot verify process ownership: inspection failed or produced diagnostics; use an isolated C3_PROFILE_DIR for testing");
            }
            return Ok(std::process::Output { status, stdout, stderr });
        }
        Ok(Err(error)) => error.to_string(),
        Err(_) => "process ownership inspection timed out".into(),
    };
    // The inspected command is /bin/ps, not an arbitrary process tree. Still
    // observe its owned PID after stopping it instead of relying on Drop.
    let _ = child.start_kill();
    match tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await {
        Ok(Ok(_)) => bail!("cannot verify process ownership: {failure}"),
        other => bail!("cannot verify process ownership: {failure}; inspector cleanup unconfirmed: {other:?}"),
    }
}

#[cfg(any(target_os = "macos", test))]
async fn bounded_pipe(mut pipe: impl tokio::io::AsyncRead + Unpin, cap: usize) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::with_capacity(cap.min(8192));
    let mut chunk = [0u8; 8192];
    loop {
        let count = pipe.read(&mut chunk).await?;
        if count == 0 { return Ok(bytes); }
        if bytes.len().saturating_add(count) > cap { bail!("inspection pipe exceeded its byte limit"); }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

#[cfg(any(target_os = "macos",test))]
fn check_process_inventory(inventory:&str,current:u32)->Result<()> {
    for line in inventory.lines().filter(|line|!line.trim().is_empty()) {
        let line=line.trim_start();
        let split=line.find(char::is_whitespace).context("invalid process ownership inventory")?;
        let pid:u32=line[..split].parse().context("invalid process identity in ownership inventory")?;
        if pid==current {continue;}
        let path=line[split..].trim();
        let name=std::path::Path::new(path).file_name().and_then(|name|name.to_str()).unwrap_or("");
        if matches!(name,"BombCode"|"Bomb Code"|"grok-build-control-panel"|"See Cubed"|"C3") {
            bail!("Close the other Bomb Code/C3 process (PID {pid}) before opening the normal store. A legacy binary cannot be fenced by the new ownership lock. Use an isolated C3_PROFILE_DIR for parallel qualification.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_alias_is_refused_but_current_process_and_unrelated_apps_are_allowed() {
        assert!(check_process_inventory("42 /Applications/Bomb Code.app/Contents/MacOS/BombCode\n43 /usr/bin/other\n",42).is_ok());
        assert!(check_process_inventory("42 /Applications/Bomb Code.app/Contents/MacOS/BombCode\n",43).is_err());
        assert!(check_process_inventory("42 /private/tmp/copy.app/Contents/MacOS/grok-build-control-panel\n",43).is_err());
        assert!(check_process_inventory("missingPID /Applications/BombCode\n",43).is_err());
    }

    #[tokio::test]
    async fn both_inventory_pipes_are_bounded_before_allocation() {
        for script in ["import os; os.write(1,b'x'*200000)", "import os; os.write(2,b'x'*200000)"] {
            let mut command=tokio::process::Command::new("/usr/bin/python3");
            command.args(["-c",script]);
            let error=bounded_inventory(&mut command,1024,1024,std::time::Duration::from_secs(2)).await.unwrap_err();
            assert!(error.to_string().contains("byte limit"),"{error}");
        }
    }

    #[tokio::test]
    async fn inventory_timeout_stops_owned_inspector_and_diagnostics_fail_closed() {
        let mut command=tokio::process::Command::new("/bin/sleep");command.arg("30");
        let before=std::time::Instant::now();
        let error=bounded_inventory(&mut command,1024,1024,std::time::Duration::from_millis(100)).await.unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(!error.to_string().contains("cleanup unconfirmed"));
        assert!(before.elapsed()<std::time::Duration::from_secs(2));
        let mut command=tokio::process::Command::new("/usr/bin/python3");command.args(["-c","import os; os.write(2,b'warning')"]);
        assert!(bounded_inventory(&mut command,1024,1024,std::time::Duration::from_secs(2)).await.is_err());
    }
}
