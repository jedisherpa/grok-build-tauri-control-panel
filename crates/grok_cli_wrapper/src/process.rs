//! One child owner for existing native runners. Completion means observed exit,
//! pipe settlement and quiescence of the dedicated Unix process group. It does
//! not claim containment of descendants which deliberately leave that group.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Child,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

#[derive(Clone, Debug)]
pub struct ProcessConfig {
    pub timeout: Option<Duration>,
    pub cleanup_timeout: Duration,
    pub output_limit: usize,
}
impl ProcessConfig {
    pub fn ensure_supported() -> io::Result<()> {
        if cfg!(target_os = "macos") {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "descendant process supervision unavailable on this platform",
            ))
        }
    }
}
impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(120)),
            cleanup_timeout: Duration::from_secs(5),
            output_limit: 256 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessEnd {
    Success,
    Nonzero,
    Signal,
    Cancelled,
    TimedOut,
    IoFailure,
    Interrupted,
    CleanupFailure,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}
impl ProcessOutput {
    pub fn text(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupScope {
    DedicatedProcessGroup,
    #[default]
    Unverified,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessOutcome {
    pub end: ProcessEnd,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// Only the dedicated group; setsid/setpgid escape is not certified containment.
    #[serde(default)]
    pub cleanup_scope: CleanupScope,
    pub cleanup_complete: bool,
    pub pipes_complete: bool,
    pub output: ProcessOutput,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub error: Option<String>,
}
/// A transport that takes a pipe must explicitly report its observed settlement.
#[derive(Clone, Debug)]
pub struct DrainTicket(
    watch::Receiver<Option<std::result::Result<(), String>>>,
    Arc<Mutex<Option<String>>>,
);
pub struct DrainReporter(
    Option<watch::Sender<Option<std::result::Result<(), String>>>>,
    Arc<Mutex<Option<String>>>,
);
impl DrainTicket {
    pub fn pair() -> (DrainReporter, Self) {
        let (tx, rx) = watch::channel(None);
        {
            let failure = Arc::new(Mutex::new(None));
            (DrainReporter(Some(tx), failure.clone()), Self(rx, failure))
        }
    }
    async fn wait(&mut self) -> std::result::Result<(), String> {
        loop {
            if let Some(Ok(())) = self.0.borrow().clone() {
                return Ok(());
            }
            self.0
                .changed()
                .await
                .map_err(|_| "pipe owner disappeared before settlement".to_string())?;
        }
    }
}
impl DrainReporter {
    pub fn fail(&self, message: impl Into<String>) {
        let message = message.into();
        let mut sticky = self.1.lock().unwrap_or_else(|error| error.into_inner());
        if sticky.is_none() {
            *sticky = Some(message.clone());
        }
        if let Some(tx) = &self.0 {
            tx.send_replace(Some(Err(message)));
        }
    }
    pub fn complete(mut self, result: std::result::Result<(), String>) {
        if let Err(error) = &result {
            self.fail(error.clone());
        }
        if let Some(tx) = self.0.take() {
            tx.send_replace(Some(result));
        }
    }
}
impl Drop for DrainReporter {
    fn drop(&mut self) {
        if self.0.is_some() {
            self.fail("pipe owner dropped before EOF");
        }
        if let Some(tx) = self.0.take() {
            tx.send_replace(Some(Err("pipe owner dropped before EOF".into())));
        }
    }
}
#[derive(Default)]
struct Tail {
    bytes: Vec<u8>,
    truncated: bool,
}
impl Tail {
    fn append(&mut self, data: &[u8], limit: usize) {
        if data.len() >= limit {
            self.bytes.clear();
            self.bytes.extend_from_slice(&data[data.len() - limit..]);
            self.truncated = true;
        } else {
            let overflow = (self.bytes.len() + data.len()).saturating_sub(limit);
            if overflow > 0 {
                self.bytes.drain(..overflow);
                self.truncated = true;
            }
            self.bytes.extend_from_slice(data);
        }
    }
    fn text(&self) -> String {
        let start = self
            .bytes
            .iter()
            .position(|byte| byte & 0xc0 != 0x80)
            .unwrap_or(self.bytes.len());
        let bytes = &self.bytes[start..];
        // Omit an incomplete trailing character until its next bytes arrive.
        let end = match std::str::from_utf8(bytes) {
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            _ => bytes.len(),
        };
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }
}
#[derive(Default)]
struct Output {
    stdout: Tail,
    stderr: Tail,
}
impl Output {
    fn snapshot(&self) -> ProcessOutput {
        ProcessOutput {
            stdout: self.stdout.text(),
            stderr: self.stderr.text(),
            truncated: self.stdout.truncated || self.stderr.truncated,
        }
    }
}
/// Host-only one-use launch attestation. Its private PID is produced only by
/// the helper that requests a dedicated group before the kernel spawns Child.
#[derive(Debug)]
pub struct GroupProof {
    pid: Option<i32>,
}
pub fn spawn_group(command: &mut tokio::process::Command) -> io::Result<(Child, GroupProof)> {
    ProcessConfig::ensure_supported()?;
    #[cfg(unix)]
    command.process_group(0);
    command.kill_on_drop(true);
    let child = command.spawn()?;
    let proof = GroupProof {
        pid: child.id().map(|pid| pid as i32),
    };
    Ok((child, proof))
}
struct Cancel {
    reply: oneshot::Sender<ProcessOutcome>,
}
#[derive(Clone)]
pub struct ProcessHandle {
    commands: mpsc::Sender<Cancel>,
    outcome: watch::Receiver<Option<ProcessOutcome>>,
    output: Arc<Mutex<Output>>,
}
impl std::fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHandle")
            .field(
                "outcome",
                &self.outcome.borrow().as_ref().map(|outcome| outcome.end),
            )
            .finish_non_exhaustive()
    }
}
impl ProcessHandle {
    /// The caller must spawn into a dedicated process_group(0). External pipe
    /// owners supply tickets; no ticket may be completed on spawn acknowledgement.
    pub fn adopt(
        child: Child,
        config: ProcessConfig,
        external_drains: Vec<DrainTicket>,
    ) -> io::Result<Self> {
        Self::adopt_inner(child, None, config, external_drains)
    }
    pub fn adopt_attested(
        child: Child,
        proof: GroupProof,
        config: ProcessConfig,
        external_drains: Vec<DrainTicket>,
    ) -> io::Result<Self> {
        Self::adopt_inner(child, Some(proof), config, external_drains)
    }
    fn adopt_inner(
        mut child: Child,
        proof: Option<GroupProof>,
        mut config: ProcessConfig,
        mut external_drains: Vec<DrainTicket>,
    ) -> io::Result<Self> {
        let pid = child.id().map(|pid| pid as i32);
        #[cfg(unix)]
        let verified_group = cfg!(target_os = "macos")
            && pid.is_some_and(|pid| {
                let bound = proof.as_ref().is_none_or(|proof| proof.pid == Some(pid));
                if !bound {
                    return false;
                }
                let group = unsafe { libc::getpgid(pid) };
                if group == pid {
                    return true;
                }
                // XNU's live-process lookup may no longer find a zombie. Only the
                // launch proof plus non-reaping wait proves this exact child PID is
                // still reserved (including an exit transition not yet reported ready);
                // bare ESRCH is never authority to signal a group.
                proof.is_some()
                    && group == -1
                    && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                    && observe_exit_now(pid).is_ok()
            });
        #[cfg(not(unix))]
        let verified_group = false;
        {
            config.output_limit = config.output_limit.clamp(1024, 1024 * 1024);
            config.cleanup_timeout = config
                .cleanup_timeout
                .clamp(Duration::from_millis(50), Duration::from_secs(10));
            let output = Arc::new(Mutex::new(Output::default()));
            let mut readers = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                let (reporter, ticket) = DrainTicket::pair();
                external_drains.push(ticket);
                readers.push(drain(
                    stdout,
                    output.clone(),
                    false,
                    config.output_limit,
                    reporter,
                ));
            }
            if let Some(stderr) = child.stderr.take() {
                let (reporter, ticket) = DrainTicket::pair();
                external_drains.push(ticket);
                readers.push(drain(
                    stderr,
                    output.clone(),
                    true,
                    config.output_limit,
                    reporter,
                ));
            }
            let (commands, rx) = mpsc::channel(16);
            let (tx, outcome) = watch::channel(None);
            if !verified_group {
                tokio::spawn(retain_unverified(child, config, rx, tx, output.clone()));
            } else {
                #[cfg(unix)]
                tokio::spawn(own(
                    child,
                    pid.expect("verified leader"),
                    config,
                    external_drains,
                    readers,
                    rx,
                    tx,
                    output.clone(),
                ));
            }
            Ok(Self {
                commands,
                outcome,
                output,
            })
        }
    }
    pub fn outcome(&self) -> Option<ProcessOutcome> {
        self.outcome.borrow().clone()
    }
    pub fn same_process(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.output, &other.output)
    }
    pub fn output(&self) -> ProcessOutput {
        self.output
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot()
    }
    pub async fn wait_outcome(&self) -> ProcessOutcome {
        let mut receiver = self.outcome.clone();
        loop {
            if let Some(outcome) = receiver.borrow().clone() {
                return outcome;
            }
            if receiver.changed().await.is_err() {
                return self.owner_lost();
            }
        }
    }
    pub async fn cancel(&self) -> ProcessOutcome {
        if let Some(outcome) = self
            .outcome
            .borrow()
            .clone()
            .filter(|outcome| outcome.cleanup_complete)
        {
            return outcome;
        }
        let attempt = async {
            let (tx, rx) = oneshot::channel();
            if self.commands.send(Cancel { reply: tx }).await.is_err() {
                return self.wait_outcome().await;
            }
            match rx.await {
                Ok(outcome) => outcome,
                Err(_) => self.wait_outcome().await,
            }
        };
        match tokio::time::timeout(Duration::from_secs(30), attempt).await {
            Ok(outcome) => outcome,
            Err(_) => {
                let mut outcome = self.owner_lost();
                outcome.error = Some(
                    "Stop response exceeded its deadline; process owner remains retained".into(),
                );
                outcome
            }
        }
    }
    fn owner_lost(&self) -> ProcessOutcome {
        ProcessOutcome {
            end: ProcessEnd::CleanupFailure,
            exit_code: None,
            signal: None,
            cleanup_scope: CleanupScope::Unverified,
            cleanup_complete: false,
            pipes_complete: false,
            output: self.output(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            error: Some("process owner disappeared without verified cleanup".into()),
        }
    }
}
fn drain<R: AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    output: Arc<Mutex<Output>>,
    stderr: bool,
    limit: usize,
    reporter: DrainReporter,
) -> JoinHandle<io::Result<()>> {
    tokio::spawn(async move {
        let mut buffer = [0u8; 8192];
        loop {
            let count = match reader.read(&mut buffer).await {
                Ok(count) => count,
                Err(error) => {
                    reporter.complete(Err(error.to_string()));
                    return Err(error);
                }
            };
            if count == 0 {
                reporter.complete(Ok(()));
                return Ok(());
            }
            let mut output = output.lock().unwrap_or_else(|error| error.into_inner());
            if stderr {
                output.stderr.append(&buffer[..count], limit);
            } else {
                output.stdout.append(&buffer[..count], limit);
            }
        }
    })
}
async fn retain_unverified(
    mut child: Child,
    config: ProcessConfig,
    mut commands: mpsc::Receiver<Cancel>,
    tx: watch::Sender<Option<ProcessOutcome>>,
    output: Arc<Mutex<Output>>,
) {
    let started_at = Utc::now();
    loop {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(config.cleanup_timeout, child.wait()).await;
        let outcome=ProcessOutcome {end:ProcessEnd::CleanupFailure,exit_code:None,signal:None,cleanup_scope:CleanupScope::Unverified,cleanup_complete:false,pipes_complete:false,output:output.lock().unwrap_or_else(|error|error.into_inner()).snapshot(),started_at,finished_at:Utc::now(),error:Some("supplied child has no verified dedicated process group; ownership retained for explicit recovery".into())};
        tx.send_replace(Some(outcome.clone()));
        match commands.recv().await {
            Some(cancel) => {
                let _ = cancel.reply.send(outcome);
            }
            None => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}
#[cfg(unix)]
fn observe_exit_now(pid: i32) -> io::Result<Option<std::process::ExitStatus>> {
    use std::os::unix::process::ExitStatusExt;
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if unsafe { info.si_pid() } == 0 {
        return Ok(None);
    }
    let code = unsafe { info.si_status() };
    let raw = if info.si_code == libc::CLD_EXITED {
        code << 8
    } else {
        code
    };
    Ok(Some(std::process::ExitStatus::from_raw(raw)))
}
#[cfg(unix)]
async fn observe_exit(pid: i32) -> io::Result<std::process::ExitStatus> {
    loop {
        if let Some(status) = observe_exit_now(pid)? {
            return Ok(status);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[cfg(unix)]
async fn live_group_members(pgid: i32, exclude_leader: bool) -> io::Result<bool> {
    let mut command = tokio::process::Command::new("/bin/ps");
    command
        .args(["-o", "pid=,stat=", "-g", &pgid.to_string()])
        .kill_on_drop(true);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut inspector = command.spawn()?;
    let stdout = inspector
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing inspector stdout"))?;
    let stderr = inspector
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing inspector stderr"))?;
    let inspected = tokio::time::timeout(Duration::from_secs(1), async {
        let read = |reader: tokio::process::ChildStdout| async move {
            let mut data = Vec::new();
            reader.take(65537).read_to_end(&mut data).await?;
            if data.len() > 65536 {
                return Err(io::Error::other("owned group listing exceeded bound"));
            }
            Ok::<_, io::Error>(data)
        };
        let errors = async move {
            let mut data = Vec::new();
            stderr.take(8193).read_to_end(&mut data).await?;
            if data.len() > 8192 {
                return Err(io::Error::other(
                    "owned group inspector error exceeded bound",
                ));
            }
            Ok::<_, io::Error>(data)
        };
        let (stdout, stderr) = tokio::try_join!(read(stdout), errors)?;
        let status = inspector.wait().await?;
        Ok::<_, io::Error>(std::process::Output {
            status,
            stdout,
            stderr,
        })
    })
    .await;
    let output = match inspected {
        Ok(Ok(output)) => output,
        failure => {
            let _ = inspector.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(1), inspector.wait()).await;
            return Err(match failure {
                Ok(Err(error)) => error,
                _ => io::Error::new(io::ErrorKind::TimedOut, "owned group inspection timed out"),
            });
        }
    };
    parse_group_listing(&output, pgid, exclude_leader)
}
#[cfg(unix)]
fn parse_group_listing(
    output: &std::process::Output,
    pgid: i32,
    exclude_leader: bool,
) -> io::Result<bool> {
    if !output.stderr.is_empty() {
        return Err(io::Error::other(
            "owned group inspector reported an error; quiescence unconfirmed",
        ));
    }
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(io::Error::other("owned group inspection failed"));
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split_whitespace();
        let pid = fields.next().and_then(|field| field.parse::<i32>().ok());
        let state = fields.next().unwrap_or("");
        if pid.is_none() || state.is_empty() {
            return Err(io::Error::other("owned group inspection was malformed"));
        }
        if (!exclude_leader || pid != Some(pgid)) && !state.starts_with('Z') {
            return Ok(true);
        }
    }
    Ok(false)
}
#[cfg(unix)]
async fn cleanup(
    child: &mut Child,
    pgid: i32,
    budget: Duration,
) -> io::Result<std::process::ExitStatus> {
    // WNOWAIT retains the leader identity until all owned members are dead.
    // Never signal a captured group after reaping that identity.
    if child.id() != Some(pgid as u32) {
        return Err(io::Error::other(
            "leader identity already reaped; refusing stale process-group signal",
        ));
    }
    tokio::time::timeout(budget, async {
        while live_group_members(pgid, false).await? {
            // A fork can race the first group signal and inherit the group
            // without inheriting its parent's pending SIGKILL. Re-signal live
            // members within this same deadline. WNOWAIT still pins the leader;
            // it is reaped only after the entire dedicated group is quiescent.
            if unsafe { libc::kill(-pgid, libc::SIGKILL) } != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        child.wait().await
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "owned process group did not become quiescent",
        )
    })?
}
async fn external_failure(tickets: &[DrainTicket]) -> String {
    loop {
        for ticket in tickets {
            if let Some(error) = ticket
                .1
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
            {
                return error;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
async fn own(
    mut child: Child,
    pgid: i32,
    config: ProcessConfig,
    mut external: Vec<DrainTicket>,
    readers: Vec<JoinHandle<io::Result<()>>>,
    mut commands: mpsc::Receiver<Cancel>,
    tx: watch::Sender<Option<ProcessOutcome>>,
    output: Arc<Mutex<Output>>,
) {
    use std::os::unix::process::ExitStatusExt;
    let mut readers: Vec<_> = readers.into_iter().map(Some).collect();
    let started_at = Utc::now();
    let deadline = tokio::time::sleep(
        config
            .timeout
            .unwrap_or(Duration::from_secs(365 * 24 * 3600)),
    );
    tokio::pin!(deadline);
    let mut replies = Vec::new();
    let (mut end, mut status, mut error) = tokio::select! {
        result=observe_exit(pgid)=>match result { Ok(status)=>(if status.success(){ProcessEnd::Success}else if status.signal().is_some(){ProcessEnd::Signal}else{ProcessEnd::Nonzero},Some(status),None),Err(error)=>(ProcessEnd::IoFailure,None,Some(error.to_string())) },
        cancel=commands.recv()=>{ if let Some(cancel)=cancel { replies.push(cancel.reply); } (ProcessEnd::Cancelled,None,None) },
        _=&mut deadline, if config.timeout.is_some()=>(ProcessEnd::TimedOut,None,Some("process deadline exceeded".into())),
        failure=external_failure(&external), if !external.is_empty()=>(ProcessEnd::IoFailure,None,Some(failure)),
    };
    let mut group_quiescent = false;
    if end == ProcessEnd::Success {
        match live_group_members(pgid, true).await {
            Ok(true) => {
                end = ProcessEnd::Interrupted;
                error=Some("leader exited before its owned descendants; remaining work was interrupted for cleanup".into());
            }
            Err(failure) => {
                end = ProcessEnd::IoFailure;
                error = Some(failure.to_string());
            }
            _ => {}
        }
    }
    loop {
        let mut pipes_complete = false;
        let mut outcome_error = error.clone();
        if !group_quiescent {
            match cleanup(&mut child, pgid, config.cleanup_timeout).await {
                Ok(observed) => {
                    group_quiescent = true;
                    if status.is_none() {
                        status = Some(observed);
                    }
                }
                Err(failure) => outcome_error = Some(failure.to_string()),
            }
        }
        let mut cleanup_complete = group_quiescent;
        if cleanup_complete {
            let settled = tokio::time::timeout(config.cleanup_timeout, async {
                for slot in &mut readers {
                    if let Some(reader) = slot.as_mut() {
                        let result = reader.await;
                        *slot = None;
                        match result {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => return Err(error.to_string()),
                            Err(error) => return Err(error.to_string()),
                        }
                    }
                }
                for ticket in &mut external {
                    ticket.wait().await?;
                }
                Ok::<_, String>(())
            })
            .await;
            match settled {
                Ok(Ok(())) => pipes_complete = true,
                Ok(Err(failure)) => {
                    end = ProcessEnd::IoFailure;
                    outcome_error = Some(failure);
                    cleanup_complete = false;
                }
                Err(_) => {
                    cleanup_complete = false;
                    outcome_error =
                        Some("process pipes did not settle within cleanup deadline".into());
                }
            }
        }
        if let Some(failure) = external.iter().find_map(|ticket| {
            ticket
                .1
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }) {
            if cleanup_complete {
                end = ProcessEnd::IoFailure;
                outcome_error = Some(failure);
            }
        }
        if let Some(original) = &error {
            if outcome_error
                .as_ref()
                .is_some_and(|current| current != original)
            {
                outcome_error = Some(format!("{original}; {}", outcome_error.unwrap_or_default()));
            }
        }
        let outcome = ProcessOutcome {
            end: if cleanup_complete {
                end
            } else {
                ProcessEnd::CleanupFailure
            },
            exit_code: status.and_then(|status| status.code()),
            signal: status.and_then(|status| status.signal()),
            cleanup_scope: CleanupScope::DedicatedProcessGroup,
            cleanup_complete,
            pipes_complete,
            output: output
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .snapshot(),
            started_at,
            finished_at: Utc::now(),
            error: outcome_error,
        };
        tx.send_replace(Some(outcome.clone()));
        for reply in replies.drain(..) {
            let _ = reply.send(outcome.clone());
        }
        if cleanup_complete {
            return;
        }
        // Failed cleanup retains this owner and the captured PGID. Only a new
        // cleanup request retries; callers must retain their workspace lease.
        match commands.recv().await {
            Some(cancel) => replies.push(cancel.reply),
            None => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        // This request retries cleanup only; retain the original observed end
        // and cause instead of changing a timeout/failure into cancellation.
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    fn adopted(
        command: &mut tokio::process::Command,
        config: ProcessConfig,
        tickets: Vec<DrainTicket>,
    ) -> ProcessHandle {
        let (child, proof) = spawn_group(command).unwrap();
        ProcessHandle::adopt_attested(child, proof, config, tickets).unwrap()
    }
    fn worker(script: &str, config: ProcessConfig) -> ProcessHandle {
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        adopted(&mut command, config, vec![])
    }
    #[test]
    fn inspector_error_cannot_certify_empty_group() {
        use std::os::unix::process::ExitStatusExt;
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(1 << 8),
            stdout: vec![],
            stderr: b"inspection denied".to_vec(),
        };
        assert!(parse_group_listing(&output, 123, false).is_err());
    }
    #[tokio::test]
    async fn internal_reader_failure_is_sticky_and_retry_never_invents_eof() {
        struct FailingReader;
        impl tokio::io::AsyncRead for FailingReader {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<io::Result<()>> {
                std::task::Poll::Ready(Err(io::Error::other("injected pipe failure")))
            }
        }
        let (reporter, ticket) = DrainTicket::pair();
        let output = Arc::new(Mutex::new(Output::default()));
        assert!(drain(FailingReader, output, false, 1024, reporter)
            .await
            .unwrap()
            .is_err());
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "sleep 120 & wait"])
            .process_group(0)
            .kill_on_drop(true);
        let process = adopted(
            &mut command,
            ProcessConfig {
                timeout: None,
                ..Default::default()
            },
            vec![ticket],
        );
        let outcome = process.wait_outcome().await;
        assert!(!outcome.cleanup_complete && !outcome.pipes_complete);
        let retried = process.cancel().await;
        assert!(!retried.cleanup_complete && !retried.pipes_complete);
        assert!(retried.error.unwrap().contains("pipe"));
    }

    #[tokio::test]
    async fn actual_exit_and_both_pipes_are_required_for_success() {
        let process = worker(
            "sleep 0.05; printf 'héllo'; printf final-error >&2",
            ProcessConfig::default(),
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(5), process.wait_outcome())
                .await
                .is_err()
        );
        let outcome = process.wait_outcome().await;
        assert_eq!(outcome.end, ProcessEnd::Success, "{outcome:?}");
        assert!(outcome.cleanup_complete && outcome.pipes_complete);
        assert_eq!(outcome.output.stdout, "héllo");
        assert_eq!(outcome.output.stderr, "final-error");
    }
    #[tokio::test]
    async fn concurrent_pipe_pressure_is_bounded_and_unicode_safe() {
        let process = worker(
            "i=0; while [ $i -lt 12000 ]; do printf 'é🐢'; printf 'λ🧊' >&2; i=$((i+1)); done",
            ProcessConfig {
                output_limit: 1024,
                ..Default::default()
            },
        );
        let outcome = process.wait_outcome().await;
        assert_eq!(outcome.end, ProcessEnd::Success, "{outcome:?}");
        assert!(outcome.output.truncated);
        assert!(outcome.output.stdout.len() <= 1024 && outcome.output.stderr.len() <= 1024);
        assert!(!outcome.output.text().contains('�'));
    }
    #[tokio::test]
    async fn nonzero_signal_timeout_and_stop_have_distinct_outcomes() {
        assert_eq!(
            worker("exit 7", ProcessConfig::default())
                .wait_outcome()
                .await
                .end,
            ProcessEnd::Nonzero
        );
        assert_eq!(
            worker("kill -TERM $$", ProcessConfig::default())
                .wait_outcome()
                .await
                .end,
            ProcessEnd::Signal
        );
        let timeout = worker(
            "sleep 120 & wait",
            ProcessConfig {
                timeout: Some(Duration::from_millis(40)),
                ..Default::default()
            },
        )
        .wait_outcome()
        .await;
        assert_eq!(timeout.end, ProcessEnd::TimedOut);
        assert!(timeout.cleanup_complete);
        let process = worker("sleep 120 & wait", ProcessConfig::default());
        let stopped = process.cancel().await;
        assert_eq!(stopped.end, ProcessEnd::Cancelled);
        assert!(stopped.cleanup_complete);
    }
    #[tokio::test]
    async fn leader_exit_does_not_hide_surviving_pipe_owner() {
        let process = worker("sleep 120 & exit 0", ProcessConfig::default());
        let outcome = tokio::time::timeout(Duration::from_secs(2), process.wait_outcome())
            .await
            .unwrap();
        assert_eq!(outcome.end, ProcessEnd::Interrupted);
        assert!(outcome.cleanup_complete && outcome.pipes_complete);
    }
    #[tokio::test]
    async fn external_failure_is_sticky_and_stops_an_indefinite_worker() {
        // Exercise immediate failure while the shell is still forking its child.
        for _ in 0..32 {
            let (reporter, ticket) = DrainTicket::pair();
            let mut command = tokio::process::Command::new("/bin/sh");
            command
                .args(["-c", "sleep 120 & wait"])
                .process_group(0)
                .kill_on_drop(true);
            let config = ProcessConfig {
                timeout: None,
                cleanup_timeout: Duration::from_secs(1),
                ..Default::default()
            };
            let budget = config.cleanup_timeout * 2 + Duration::from_secs(2);
            let (child, proof) = spawn_group(&mut command).unwrap();
            let pgid = child.id().unwrap() as i32;
            let process =
                ProcessHandle::adopt_attested(child, proof, config, vec![ticket]).unwrap();
            reporter.fail("oversized external frame");
            reporter.complete(Ok(()));
            let outcome = tokio::time::timeout(budget, process.wait_outcome())
                .await
                .unwrap();
            if !outcome.cleanup_complete {
                let retried = process.cancel().await;
                eprintln!("initial retained outcome for group {pgid}: {outcome:#?}; explicit cleanup retry: {retried:#?}");
                assert!(
                    retried.cleanup_complete,
                    "generated worker remains retained: {retried:#?}"
                );
            }
            assert!(
                !live_group_members(pgid, false).await.unwrap(),
                "generated group {pgid} still has live members"
            );
            assert_eq!(outcome.end, ProcessEnd::IoFailure, "{outcome:#?}");
            assert!(
                outcome.cleanup_complete && outcome.pipes_complete,
                "{outcome:#?}"
            );
            assert!(outcome.error.unwrap().contains("oversized"));
        }
    }
    #[tokio::test]
    async fn supplied_unverified_group_is_retained_instead_of_abandoned() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "sleep 0.02"]).kill_on_drop(true);
        let process =
            ProcessHandle::adopt(command.spawn().unwrap(), ProcessConfig::default(), vec![])
                .unwrap();
        let outcome = process.wait_outcome().await;
        assert_eq!(outcome.cleanup_scope, CleanupScope::Unverified);
        assert!(!outcome.cleanup_complete);
        assert!(!process.cancel().await.cleanup_complete);
    }
    #[tokio::test]
    async fn group_scope_does_not_claim_sets_id_escape_containment() {
        let fixture = tempfile::tempdir().unwrap();
        let escaped = fixture.path().join("escaped-pid");
        let script="import os,time,sys\npid=os.fork()\nif pid==0:\n os.setsid();os.close(1);os.close(2);open(sys.argv[1],'w').write(str(os.getpid()));time.sleep(30)\nelse:\n time.sleep(.08)";
        let mut command = tokio::process::Command::new("/usr/bin/python3");
        command
            .args(["-c", script])
            .arg(&escaped)
            .process_group(0)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let process = adopted(&mut command, ProcessConfig::default(), vec![]);
        let outcome = process.wait_outcome().await;
        let pid: i32 = std::fs::read_to_string(&escaped).unwrap().parse().unwrap();
        // The harness owns this generated escaped PID and independently cleans
        // it; the supervisor cannot infer escape containment from group exit.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        let stopped = unsafe { libc::kill(pid, libc::SIGKILL) };
        assert_eq!(stopped, 0);
        assert!(alive);
        assert_eq!(outcome.cleanup_scope, CleanupScope::DedicatedProcessGroup);
        assert!(outcome.cleanup_complete);
    }

    #[tokio::test]
    async fn external_eof_is_observed_and_failed_cleanup_can_retry() {
        let (reporter, ticket) = DrainTicket::pair();
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "exit 0"])
            .process_group(0)
            .kill_on_drop(true);
        let process = adopted(
            &mut command,
            ProcessConfig {
                cleanup_timeout: Duration::from_millis(50),
                ..Default::default()
            },
            vec![ticket],
        );
        let failed = process.wait_outcome().await;
        assert_eq!(failed.end, ProcessEnd::CleanupFailure);
        assert!(!failed.cleanup_complete);
        reporter.complete(Ok(()));
        assert!(process.cancel().await.cleanup_complete);
    }
    #[tokio::test]
    async fn cleanup_retry_preserves_original_timeout_and_reason() {
        let (reporter, ticket) = DrainTicket::pair();
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "sleep 120 & wait"])
            .process_group(0)
            .kill_on_drop(true);
        let process = adopted(
            &mut command,
            ProcessConfig {
                timeout: Some(Duration::from_millis(30)),
                cleanup_timeout: Duration::from_millis(50),
                ..Default::default()
            },
            vec![ticket],
        );
        let failed = process.wait_outcome().await;
        assert_eq!(failed.end, ProcessEnd::CleanupFailure);
        assert!(failed
            .error
            .as_deref()
            .unwrap()
            .contains("process deadline exceeded"));
        reporter.complete(Ok(()));
        let recovered = process.cancel().await;
        assert!(recovered.cleanup_complete && recovered.pipes_complete);
        assert_eq!(recovered.end, ProcessEnd::TimedOut);
        assert_eq!(
            recovered.error.as_deref(),
            Some("process deadline exceeded")
        );
    }
    #[tokio::test]
    async fn repeated_fast_exit_uses_launch_proof_and_reserved_child_identity() {
        let mut zombie_fallbacks = 0;
        for _ in 0..64 {
            let mut command = tokio::process::Command::new("/bin/echo");
            command
                .arg("fast final λ")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let (child, proof) = spawn_group(&mut command).unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
            let pid = child.id().unwrap() as i32;
            let group = unsafe { libc::getpgid(pid) };
            let lookup_error = io::Error::last_os_error();
            if group == -1 {
                zombie_fallbacks += 1;
            }
            let process =
                ProcessHandle::adopt_attested(child, proof, ProcessConfig::default(), vec![])
                    .unwrap();
            let outcome = process.wait_outcome().await;
            assert_eq!(
                outcome.end,
                ProcessEnd::Success,
                "{outcome:?}; group={group} lookup={lookup_error:?}"
            );
            assert!(outcome.cleanup_complete && outcome.pipes_complete);
            assert_eq!(outcome.output.stdout, "fast final λ\n");
        }
        assert!(
            zombie_fallbacks > 0,
            "stress fixture must exercise post-exit group lookup failure"
        );
        let mut command = tokio::process::Command::new("/usr/bin/true");
        let child = command.spawn().unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let supplied = ProcessHandle::adopt(child, ProcessConfig::default(), vec![]).unwrap();
        assert!(!supplied.wait_outcome().await.cleanup_complete);
    }
    #[tokio::test]
    async fn attestation_cannot_be_transferred_to_another_or_reaped_child() {
        let mut first = tokio::process::Command::new("/bin/sleep");
        first.arg("120");
        let (child_a, proof_a) = spawn_group(&mut first).unwrap();
        let pid_a = child_a.id().unwrap() as i32;
        let mut second = tokio::process::Command::new("/bin/sleep");
        second.arg("120");
        let (child_b, _proof_b) = spawn_group(&mut second).unwrap();
        let wrong =
            ProcessHandle::adopt_attested(child_b, proof_a, ProcessConfig::default(), vec![])
                .unwrap();
        assert_eq!(
            wrong.wait_outcome().await.cleanup_scope,
            CleanupScope::Unverified
        );
        assert_eq!(
            unsafe { libc::kill(pid_a, 0) },
            0,
            "wrong child proof must not signal original leader"
        );
        let original = ProcessHandle::adopt(child_a, ProcessConfig::default(), vec![]).unwrap();
        assert!(original.cancel().await.cleanup_complete);
        let mut ended = tokio::process::Command::new("/usr/bin/true");
        let (mut child, proof) = spawn_group(&mut ended).unwrap();
        child.wait().await.unwrap();
        assert!(child.id().is_none());
        let stale =
            ProcessHandle::adopt_attested(child, proof, ProcessConfig::default(), vec![]).unwrap();
        assert!(!stale.wait_outcome().await.cleanup_complete);
    }
}
