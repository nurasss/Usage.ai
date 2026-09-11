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
    /// Extra environment, restricted to [`SCOPED_ENV_VARS`] with
    /// absolute-path values (e.g. per-profile `CODEX_HOME`).
    pub env: Vec<(String, String)>,
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
    /// Interactive stdio session for line-oriented JSON-RPC style
    /// protocols. Same allowlist, timeout and ownership rules as
    /// `spawn`; the caller drives request/response turns.
    async fn spawn_interactive(
        &self,
        req: InteractiveRequest<'_>,
    ) -> Result<InteractiveChild, HostError>;
    fn owned_count(&self) -> usize;
}

/// Bounded interactive spawn request. No shell, no inherited
/// dangerous env beyond an explicit allowlist (none by default).
pub struct InteractiveRequest<'a> {
    pub executable: &'a Path,
    pub args: &'a [&'a str],
    pub cancel: CancellationToken,
    /// Extra environment, restricted to [`SCOPED_ENV_VARS`] with
    /// absolute-path values (e.g. per-profile `CODEX_HOME`).
    pub env: Vec<(String, String)>,
}

/// Environment variable names a strategy may override per profile.
/// Closed list: anything else is a Policy violation. Values must be
/// absolute paths (profile roots), never PATH/shell material.
pub const SCOPED_ENV_VARS: &[&str] = &["CODEX_HOME", "CLAUDE_CONFIG_DIR"];

/// Validate scoped env pairs before any spawn.
pub(crate) fn check_scoped_env(env: &[(String, String)]) -> Result<(), HostError> {
    for (name, value) in env {
        if !SCOPED_ENV_VARS.contains(&name.as_str()) {
            return Err(HostError::Policy);
        }
        if !std::path::Path::new(value).is_absolute() {
            return Err(HostError::Policy);
        }
    }
    Ok(())
}

/// Owned interactive child. Only this handle can signal its process;
/// dropping aborts via `kill_on_drop`.
pub struct InteractiveChild {
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    child: tokio::process::Child,
    pid: Option<u32>,
    owned: std::sync::Arc<Mutex<HashSet<u32>>>,
}

impl Drop for InteractiveChild {
    fn drop(&mut self) {
        if let Some(pid) = self.pid.take() {
            if let Ok(mut owned) = self.owned.lock() {
                owned.remove(&pid);
            }
        }
    }
}

impl InteractiveChild {
    pub async fn write_line(&mut self, line: &str) -> Result<(), HostError> {
        use tokio::io::AsyncWriteExt;
        let mut framed = format!("{line}\n");
        // Bound a single write; oversized payloads are a caller bug.
        framed.truncate(256 * 1024);
        self.stdin
            .write_all(framed.as_bytes())
            .await
            .map_err(|_| HostError::Unavailable)?;
        self.stdin
            .flush()
            .await
            .map_err(|_| HostError::Unavailable)?;
        Ok(())
    }

    pub async fn read_line(&mut self, max_bytes: usize) -> Result<Option<String>, HostError> {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt};
        let mut line = String::new();
        let mut limited = (&mut self.stdout).take(max_bytes.min(1024 * 1024) as u64);
        match limited.read_line(&mut line).await {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(line)),
            Err(_) => Err(HostError::Unavailable),
        }
    }

    pub async fn kill(&mut self) -> Result<(), HostError> {
        self.child
            .kill()
            .await
            .map_err(|_| HostError::Unavailable)?;
        Ok(())
    }

    pub async fn wait(&mut self) -> Result<Option<i32>, HostError> {
        self.child
            .wait()
            .await
            .map(|status| status.code())
            .map_err(|_| HostError::Unavailable)
    }
}

#[derive(Debug, Default, Clone)]
pub struct AllowlistedProcess {
    allowed: std::sync::Arc<Mutex<HashSet<PathBuf>>>,
    owned: std::sync::Arc<Mutex<HashSet<u32>>>,
}

impl AllowlistedProcess {
    pub fn with_allowed(executables: impl IntoIterator<Item = PathBuf>) -> Self {
        // Canonicalize at construction so symlinked prefixes (e.g.
        // macOS /tmp -> /private/tmp) cannot cause false denials later.
        let allowed = executables
            .into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        Self {
            allowed: std::sync::Arc::new(Mutex::new(allowed)),
            owned: std::sync::Arc::new(Mutex::new(HashSet::new())),
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
        check_scoped_env(&req.env)?;
        let mut child = tokio::process::Command::new(req.executable)
            .args(req.args)
            .envs(req.env.iter().map(|(k, v)| (k, v)))
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

    async fn spawn_interactive(
        &self,
        req: InteractiveRequest<'_>,
    ) -> Result<InteractiveChild, HostError> {
        if req.cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        if !self.is_allowed(req.executable) {
            return Err(HostError::Policy);
        }
        check_scoped_env(&req.env)?;
        let mut child = tokio::process::Command::new(req.executable)
            .args(req.args)
            .envs(req.env.iter().map(|(k, v)| (k, v)))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| HostError::Unavailable)?;
        let pid = child.id();
        if let Some(pid) = pid {
            self.track(pid);
        }
        let stdin = child.stdin.take().ok_or(HostError::Unavailable)?;
        let stdout = child.stdout.take().ok_or(HostError::Unavailable)?;
        Ok(InteractiveChild {
            stdin,
            stdout: tokio::io::BufReader::new(stdout),
            child,
            pid,
            owned: self.owned.clone(),
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

    /// Scoped env allowlist: only CODEX_HOME/CLAUDE_CONFIG_DIR with
    /// absolute paths pass; everything else is Policy — for both
    /// one-shot and interactive spawns.
    #[tokio::test]
    async fn scoped_env_names_and_paths_are_enforced() {
        let host = AllowlistedProcess::with_allowed([PathBuf::from("/bin/echo")]);
        for (name, value) in [
            ("EVIL", "/tmp"),
            ("PATH", "/tmp"),
            ("CODEX_HOME", "relative/path"),
            ("CLAUDE_CONFIG_DIR", ""),
        ] {
            let denied = host
                .spawn(SpawnRequest {
                    executable: Path::new("/bin/echo"),
                    args: &[],
                    timeout: Duration::from_secs(5),
                    stdout_cap: 64,
                    stderr_cap: 64,
                    cancel: CancellationToken::new(),
                    env: vec![(name.into(), value.into())],
                })
                .await;
            assert!(
                matches!(denied, Err(HostError::Policy)),
                "{name}={value} must be denied"
            );
            let denied = host
                .spawn_interactive(InteractiveRequest {
                    executable: Path::new("/bin/echo"),
                    args: &[],
                    cancel: CancellationToken::new(),
                    env: vec![(name.into(), value.into())],
                })
                .await;
            assert!(
                matches!(denied, Err(HostError::Policy)),
                "interactive {name}={value} must be denied"
            );
        }
        // Allowlisted pair spawns fine.
        let ok = host
            .spawn(SpawnRequest {
                executable: Path::new("/bin/echo"),
                args: &[],
                timeout: Duration::from_secs(5),
                stdout_cap: 64,
                stderr_cap: 64,
                cancel: CancellationToken::new(),
                env: vec![("CODEX_HOME".into(), "/tmp".into())],
            })
            .await;
        assert!(ok.is_ok());
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
                env: vec![],
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
                    env: vec![],
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
                env: vec![],
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
                env: vec![],
            })
            .await;
        assert!(matches!(result, Err(HostError::Cancelled)));
    }

    #[tokio::test]
    async fn in_flight_cancellation_kills_owned_child() {
        let host = AllowlistedProcess::with_allowed([PathBuf::from("/bin/sleep")]);
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });
        let result = host
            .spawn(SpawnRequest {
                executable: Path::new("/bin/sleep"),
                args: &["30"],
                timeout: Duration::from_secs(5),
                stdout_cap: 64,
                stderr_cap: 64,
                cancel,
                env: vec![],
            })
            .await;
        assert!(matches!(result, Err(HostError::Cancelled)));
        assert_eq!(host.owned_count(), 0);
    }
}

#[cfg(all(test, unix))]
mod interactive_tests {
    use super::*;

    fn cat_host() -> AllowlistedProcess {
        AllowlistedProcess::with_allowed([PathBuf::from("/bin/cat")])
    }

    #[tokio::test]
    async fn interactive_roundtrip_over_stdio() {
        let host = cat_host();
        let mut child = host
            .spawn_interactive(InteractiveRequest {
                executable: Path::new("/bin/cat"),
                args: &[],
                cancel: CancellationToken::new(),
                env: vec![],
            })
            .await
            .unwrap();
        child
            .write_line(r#"{"id":1,"method":"ping"}"#)
            .await
            .unwrap();
        let line = child.read_line(4096).await.unwrap().expect("echo line");
        assert!(line.contains("\"method\":\"ping\""));
        child.kill().await.unwrap();
        drop(child);
        assert_eq!(host.owned_count(), 0);
    }

    #[tokio::test]
    async fn interactive_denies_non_allowlisted_executable() {
        let host = cat_host();
        let result = host
            .spawn_interactive(InteractiveRequest {
                executable: Path::new("/bin/sh"),
                args: &["-c", "echo pwned"],
                cancel: CancellationToken::new(),
                env: vec![],
            })
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }

    #[tokio::test]
    async fn interactive_read_line_is_bounded() {
        let host = cat_host();
        let mut child = host
            .spawn_interactive(InteractiveRequest {
                executable: Path::new("/bin/cat"),
                args: &[],
                cancel: CancellationToken::new(),
                env: vec![],
            })
            .await
            .unwrap();
        child.write_line("abcdef").await.unwrap();
        let line = child.read_line(4).await.unwrap().expect("line");
        assert!(line.len() <= 4);
        child.kill().await.unwrap();
    }
}
