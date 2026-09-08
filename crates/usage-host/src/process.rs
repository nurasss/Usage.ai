use super::{CancellationToken, HostError};
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Bounded process spawn request. Arguments are passed directly to
/// the executable — provider input can never become `sh -c`.
pub struct SpawnRequest<'a> {
    pub executable: &'a Path,
    pub args: &'a [&'a str],
    pub timeout: Duration,
    pub stdout_cap: usize,
    pub stderr_cap: usize,
    pub cancel: CancellationToken,
}

pub struct SpawnOutput {
    pub status_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

/// Approved-executable process host. Only canonical paths present in
/// the allow-list may run; only Usage.ai-owned children are ever
/// terminated (there is no `kill(pid)` API at all).
#[async_trait]
pub trait ProcessHost: Send + Sync {
    async fn spawn(&self, req: SpawnRequest<'_>) -> Result<SpawnOutput, HostError>;
    fn owned_count(&self) -> usize;
}

#[derive(Debug, Default)]
pub struct AllowlistedProcess {
    allowed: Mutex<HashSet<PathBuf>>,
    owned: Mutex<HashSet<u32>>,
}

impl AllowlistedProcess {
    pub fn with_allowed(executables: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            allowed: Mutex::new(executables.into_iter().collect()),
            owned: Mutex::new(HashSet::new()),
        }
    }

    fn is_allowed(&self, executable: &Path) -> bool {
        let Ok(canonical) = executable.canonicalize() else {
            return false;
        };
        self.allowed
            .lock()
            .map(|allowed| allowed.contains(&canonical))
            .unwrap_or(false)
    }

    fn track(&self, pid: u32) {
        if let Ok(mut owned) = self.owned.lock() {
            owned.insert(pid);
        }
    }

    fn untrack(&self, pid: u32) {
        if let Ok(mut owned) = self.owned.lock() {
            owned.remove(&pid);
        }
    }
}

#[async_trait]
impl ProcessHost for AllowlistedProcess {
    async fn spawn(&self, req: SpawnRequest<'_>) -> Result<SpawnOutput, HostError> {
        if req.cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        if !self.is_allowed(req.executable) {
            return Err(HostError::Policy);
        }
        let mut child = tokio::process::Command::new(req.executable)
            .args(req.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| HostError::Unavailable)?;
        if let Some(pid) = child.id() {
            self.track(pid);
        }
        let pid = child.id();
        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();
        let stdout_cap = req.stdout_cap.max(1);
        let stderr_cap = req.stderr_cap.max(1);
        // `wait` borrows the child, so timeout/cancel paths below can
        // still terminate the owned process. There is no kill(pid) API:
        // only this owned handle is ever signalled.
        let wait_result = tokio::select! {
            biased;
            _ = req.cancel.cancelled() => {
                let _ = child.kill().await;
                Err(HostError::Cancelled)
            }
            outcome = tokio::time::timeout(req.timeout, child.wait()) => match outcome {
                Err(_) => {
                    let _ = child.kill().await;
                    Ok(None)
                }
                Ok(Err(_)) => Err(HostError::Unavailable),
                Ok(Ok(status)) => Ok(Some(status.code())),
            },
        };
        if let Some(pid) = pid {
            self.untrack(pid);
        }
        let status_code = wait_result?;
        let mut stdout = vec![];
        if let Some(pipe) = stdout_pipe.as_mut() {
            use tokio::io::AsyncReadExt;
            let _ = pipe.read_to_end(&mut stdout).await;
        }
        let mut stderr = vec![];
        if let Some(pipe) = stderr_pipe.as_mut() {
            use tokio::io::AsyncReadExt;
            let _ = pipe.read_to_end(&mut stderr).await;
        }
        stdout.truncate(stdout_cap);
        stderr.truncate(stderr_cap);
        let timed_out = status_code.is_none();
        Ok(SpawnOutput {
            status_code: status_code.flatten(),
            stdout,
            stderr,
            timed_out,
        })
    }

    fn owned_count(&self) -> usize {
        self.owned.lock().map(|o| o.len()).unwrap_or(0)
    }
}

#[cfg(all(test, unix))]
mod tests {
    #[allow(dead_code)]
    fn cancelled_token() -> super::CancellationToken {
        let token = super::CancellationToken::new();
        token.cancel();
        token
    }

    use super::*;
    fn echo_host() -> AllowlistedProcess {
        AllowlistedProcess::with_allowed([PathBuf::from("/bin/echo")])
    }
    #[tokio::test]
    async fn allowed_executable_runs() {
        let host = echo_host();
        let out = host
            .spawn(SpawnRequest {
                executable: Path::new("/bin/echo"),
                args: &["hello"],
                timeout: Duration::from_secs(5),
                stdout_cap: 1024,
                stderr_cap: 1024,
                cancel: CancellationToken::new(),
            })
            .await
            .unwrap();
        assert_eq!(out.status_code, Some(0));
        assert_eq!(out.stdout, b"hello\n");
        assert_eq!(host.owned_count(), 0);
    }
    #[tokio::test]
    async fn arbitrary_executable_denied_including_shell() {
        let host = echo_host();
        for exe in ["/bin/sh", "/bin/ls", "/usr/bin/echo", "/nonexistent"] {
            let result = host
                .spawn(SpawnRequest {
                    executable: Path::new(exe),
                    args: &["-c", "echo pwned"],
                    timeout: Duration::from_secs(5),
                    stdout_cap: 1024,
                    stderr_cap: 1024,
                    cancel: CancellationToken::new(),
                })
                .await;
            assert!(result.is_err(), "{exe} must be denied");
        }
    }
    #[tokio::test]
    async fn timeout_kills_owned_child() {
        let host = AllowlistedProcess::with_allowed([PathBuf::from("/bin/sleep")]);
        let out = host
            .spawn(SpawnRequest {
                executable: Path::new("/bin/sleep"),
                args: &["30"],
                timeout: Duration::from_millis(100),
                stdout_cap: 64,
                stderr_cap: 64,
                cancel: CancellationToken::new(),
            })
            .await
            .unwrap();
        assert!(out.timed_out);
        assert_eq!(host.owned_count(), 0);
    }
    #[tokio::test]
    async fn pre_cancelled_token_spawns_nothing() {
        let host = echo_host();
        let result = host
            .spawn(SpawnRequest {
                executable: Path::new("/bin/echo"),
                args: &[],
                timeout: Duration::from_secs(5),
                stdout_cap: 64,
                stderr_cap: 64,
                cancel: cancelled_token(),
            })
            .await;
        assert!(matches!(result, Err(HostError::Cancelled)));
    }
}
