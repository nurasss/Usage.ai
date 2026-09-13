use super::{CancellationToken, HostError};
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Bounded PTY session request. Like `ProcessHost`, arguments are
/// passed directly — never through a shell.
pub struct PtyRequest<'a> {
    pub executable: &'a Path,
    pub args: &'a [&'a str],
    pub timeout: Duration,
    pub output_cap: usize,
    pub cancel: CancellationToken,
    /// Extra environment, restricted to process::SCOPED_ENV_VARS.
    pub env: Vec<(String, String)>,
    /// Inherited variables to remove, restricted to
    /// process::SCRUBBED_ENV_VARS.
    pub env_remove: Vec<String>,
}

/// Bounded interactive PTY spawn request. No shell, no inherited
/// dangerous env beyond an explicit allowlist (none by default).
pub struct PtySpawnRequest<'a> {
    pub executable: &'a Path,
    pub args: &'a [&'a str],
    pub cancel: CancellationToken,
    /// Extra environment, restricted to process::SCOPED_ENV_VARS.
    pub env: Vec<(String, String)>,
    /// Inherited variables to remove, restricted to
    /// process::SCRUBBED_ENV_VARS.
    pub env_remove: Vec<String>,
}

/// Controlled PTY input: line-oriented writes only.
pub struct PtyInput {
    pub lines: Vec<String>,
}

/// Owned interactive PTY child. Only this handle can signal its
/// process; dropping best-effort kills an unreaped child (the run
/// paths below always reap before returning, so Drop is exceptional).
pub struct PtyChild {
    #[cfg(unix)]
    master: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
    #[cfg(unix)]
    pid: i32,
    #[cfg(unix)]
    reaped: bool,
    #[cfg(not(unix))]
    _private: (),
}

#[cfg(unix)]
impl PtyChild {
    pub async fn write_all(&mut self, bytes: &[u8]) -> Result<(), HostError> {
        use std::os::fd::AsRawFd;
        let mut written = 0usize;
        // Bounded retries: a wedged slave must surface Unavailable,
        // never spin the refresh loop.
        for _ in 0..1024 {
            if written >= bytes.len() {
                return Ok(());
            }
            let mut guard = self
                .master
                .writable()
                .await
                .map_err(|_| HostError::Unavailable)?;
            let chunk = &bytes[written..];
            match guard.try_io(|inner| {
                // SAFETY: fd is our owned pty master; the pointer/len
                // pair borrows the caller's live slice.
                let n = unsafe {
                    libc::write(
                        inner.get_ref().as_raw_fd(),
                        chunk.as_ptr() as *const libc::c_void,
                        chunk.len(),
                    )
                };
                if n < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(Ok(0)) => return Err(HostError::Unavailable),
                Ok(Ok(n)) => written += n,
                Ok(Err(_)) => return Err(HostError::Unavailable),
                Err(_) => continue,
            }
        }
        Err(HostError::Unavailable)
    }

    /// One non-blocking drain: `Ok(None)` is EOF (including the
    /// standard pty EIO-after-exit), `Ok(Some(_))` may be empty only
    /// on spurious wakeups (callers re-poll).
    pub async fn read_chunk(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, HostError> {
        use std::os::fd::AsRawFd;
        let mut guard = self
            .master
            .readable()
            .await
            .map_err(|_| HostError::Unavailable)?;
        let mut buf = vec![0u8; max_bytes.clamp(1, 256 * 1024)];
        let len = buf.len();
        match guard.try_io(|inner| {
            // SAFETY: fd is our owned pty master; the buffer is live
            // for the call.
            let n = unsafe {
                libc::read(
                    inner.get_ref().as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    len,
                )
            };
            if n < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(Ok(0)) => Ok(None),
            Ok(Ok(n)) => {
                buf.truncate(n);
                Ok(Some(buf))
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Some(vec![])),
            // Reading a master whose slave side has no live process
            // raises EIO: that IS end-of-session, not an error.
            Ok(Err(e)) if e.raw_os_error() == Some(libc::EIO) => Ok(None),
            Ok(Err(_)) => Err(HostError::Unavailable),
            Err(_) => Ok(Some(vec![])),
        }
    }

    fn reap_once(&mut self) {
        if self.reaped {
            return;
        }
        let mut status = 0;
        // SAFETY: pid is our forkpty child; WNOHANG never blocks.
        let done = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) == self.pid };
        if done {
            self.reaped = true;
        }
    }

    /// Polite-then-firm termination of the OWNED child only, then
    /// reap: SIGTERM, short grace, SIGKILL, bounded reap. There is no
    /// kill(pid) API — only this owned handle is ever signalled.
    pub async fn kill(&mut self) -> Result<(), HostError> {
        if !self.reaped {
            // SAFETY: pid is our forkpty child; ESRCH (already gone)
            // is fine — the reap below still collects a zombie.
            unsafe {
                libc::kill(self.pid, libc::SIGTERM);
            }
            let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
            while tokio::time::Instant::now() < deadline {
                self.reap_once();
                if self.reaped {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            // SAFETY: same owned pid; SIGKILL cannot be caught.
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
            }
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while tokio::time::Instant::now() < deadline {
                self.reap_once();
                if self.reaped {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for PtyChild {
    fn drop(&mut self) {
        if !self.reaped {
            // Best-effort only: never block in Drop. All run paths
            // reap before returning, so reaching here is exceptional.
            // SAFETY: owned forkpty child pid.
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(not(unix))]
impl PtyChild {
    pub async fn write_all(&mut self, _bytes: &[u8]) -> Result<(), HostError> {
        Err(HostError::Unavailable)
    }
    pub async fn read_chunk(&mut self, _max: usize) -> Result<Option<Vec<u8>>, HostError> {
        Err(HostError::Unavailable)
    }
    pub async fn kill(&mut self) -> Result<(), HostError> {
        Err(HostError::Unavailable)
    }
}

/// Serializes fork(2): only async-signal-safe work happens post-fork,
/// and our own concurrent spawns cannot interleave.
#[cfg(unix)]
static SPAWN_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

/// Forkpty spawn: child execs the allowlisted executable DIRECTLY
/// (execv with an absolute path performs no search, no shell), with
/// a pinned deterministic locale (LANG/LC_ALL=C) and fixed 100x30
/// window so TUI layout is stable across hosts. Returns the master
/// fd and child pid to the caller, which owns both.
#[cfg(unix)]
fn spawn_pty_child(
    executable: &Path,
    args: &[&str],
    env: &[(String, String)],
    env_remove: &[String],
) -> Result<PtyChild, HostError> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    super::process::check_scoped_env(env)?;
    super::process::check_scrubbed_env(env_remove)?;
    let exe = CString::new(executable.as_os_str().as_bytes()).map_err(|_| HostError::Policy)?;
    let mut argv_owned = Vec::with_capacity(args.len() + 1);
    argv_owned.push(exe.clone());
    for arg in args {
        argv_owned.push(CString::new(*arg).map_err(|_| HostError::Policy)?);
    }
    // Scoped env pairs are validated above and converted pre-fork:
    // only async-signal-safe setenv/exec happen post-fork.
    let mut env_owned = Vec::with_capacity(env.len());
    for (name, value) in env {
        env_owned.push((
            CString::new(name.as_str()).map_err(|_| HostError::Policy)?,
            CString::new(value.as_str()).map_err(|_| HostError::Policy)?,
        ));
    }
    let mut remove_owned = Vec::with_capacity(env_remove.len());
    for name in env_remove {
        remove_owned.push(CString::new(name.as_str()).map_err(|_| HostError::Policy)?);
    }
    let mut argv: Vec<*const libc::c_char> = argv_owned.iter().map(|s| s.as_ptr()).collect();
    argv.push(std::ptr::null());
    let lock = SPAWN_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().map_err(|_| HostError::Unavailable)?;
    let mut master: libc::c_int = -1;
    let mut winsize = libc::winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: forkpty with valid out-param + winsize; parent and
    // child diverge on the return value immediately below.
    let pid = unsafe {
        libc::forkpty(
            &mut master,
            std::ptr::null_mut(),
            std::ptr::null_mut::<libc::termios>(),
            &mut winsize,
        )
    };
    if pid < 0 {
        return Err(HostError::Unavailable);
    }
    if pid == 0 {
        // Child: async-signal-safe surface only from here to exec.
        // setenv is taken under the spawn lock with no other
        // fork/exec activity of ours in flight.
        unsafe {
            std::ffi::CString::new("LANG")
                .ok()
                .map(|k| libc::setenv(k.as_ptr(), c"C".as_ptr(), 1));
            std::ffi::CString::new("LC_ALL")
                .ok()
                .map(|k| libc::setenv(k.as_ptr(), c"C".as_ptr(), 1));
            std::ffi::CString::new("TERM")
                .ok()
                .map(|k| libc::setenv(k.as_ptr(), c"xterm-256color".as_ptr(), 1));
            for (name, value) in &env_owned {
                libc::setenv(name.as_ptr(), value.as_ptr(), 1);
            }
            for name in &remove_owned {
                libc::unsetenv(name.as_ptr());
            }
            libc::execv(exe.as_ptr(), argv.as_ptr());
            libc::_exit(127);
        }
    }
    // SAFETY: master is a fresh owned fd from a successful forkpty.
    // Force non-blocking BEFORE any async use: AsyncFd readiness +
    // try_io must never execute a blocking syscall, otherwise a
    // spuriously-ready master would wedge a current_thread runtime
    // past every timeout (observed: 30s hang on an echo-primed pty).
    let flags = unsafe { libc::fcntl(master, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        // SAFETY: the pid/master belong to this just-created child/session.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, std::ptr::null_mut(), 0);
            libc::close(master);
        }
        return Err(HostError::Unavailable);
    }
    let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(master) };
    let async_fd = tokio::io::unix::AsyncFd::new(owned).map_err(|_| {
        // SAFETY: owned child, construction already failed — do not leak it.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }
        HostError::Unavailable
    })?;
    Ok(PtyChild {
        master: async_fd,
        pid,
        reaped: false,
    })
}

/// Framework abstraction for CLI strategies driven through an owned
/// pseudo-terminal (e.g. the installed Claude Code usage command).
/// Same allowlist, timeout and ownership rules as `ProcessHost`.
#[async_trait]
pub trait PTYHost: Send + Sync {
    async fn run(&self, req: PtyRequest<'_>, input: PtyInput) -> Result<Vec<u8>, HostError>;
    async fn spawn_pty(&self, req: PtySpawnRequest<'_>) -> Result<PtyChild, HostError>;
}

/// Approved-executable PTY host. Only canonical paths present in the
/// allow-list may run; only Usage.ai-owned children are ever
/// terminated (there is no `kill(pid)` API at all).
#[derive(Debug, Default)]
pub struct AllowlistedPty {
    allowed: std::sync::Mutex<HashSet<PathBuf>>,
}

impl AllowlistedPty {
    pub fn with_allowed(executables: impl IntoIterator<Item = PathBuf>) -> Self {
        // Canonicalize at construction so symlinked prefixes (e.g.
        // npm-global bin shims, macOS /tmp -> /private/tmp) cannot
        // cause false denials later.
        let allowed = executables
            .into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        Self {
            allowed: std::sync::Mutex::new(allowed),
        }
    }

    fn allowed_executable(&self, executable: &Path) -> Option<PathBuf> {
        let Ok(canonical) = executable.canonicalize() else {
            return None;
        };
        self.allowed
            .lock()
            .ok()
            .filter(|allowed| allowed.contains(&canonical))
            .map(|_| canonical)
    }
}

const MAX_PTY_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[async_trait]
impl PTYHost for AllowlistedPty {
    async fn run(&self, req: PtyRequest<'_>, input: PtyInput) -> Result<Vec<u8>, HostError> {
        if req.cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        let executable = self
            .allowed_executable(req.executable)
            .ok_or(HostError::Policy)?;
        let mut child = self
            .spawn_pty_inner(
                &executable,
                req.args,
                &req.env,
                &req.env_remove,
                &req.cancel,
            )
            .await?;
        for line in &input.lines {
            if req.cancel.is_cancelled() {
                let _ = child.kill().await;
                return Err(HostError::Cancelled);
            }
            if line.len() >= 64 * 1024 {
                let _ = child.kill().await;
                return Err(HostError::Policy);
            }
            let framed = format!("{line}\r");
            if child.write_all(framed.as_bytes()).await.is_err() {
                let _ = child.kill().await;
                return Err(HostError::Unavailable);
            }
        }
        let output_cap = req.output_cap.clamp(1, MAX_PTY_OUTPUT_BYTES);
        let deadline = tokio::time::Instant::now() + req.timeout;
        let mut out = Vec::new();
        loop {
            if req.cancel.is_cancelled() {
                let _ = child.kill().await;
                return Err(HostError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let _ = child.kill().await;
                return Err(HostError::Timeout);
            }
            if out.len() >= output_cap {
                let _ = child.kill().await;
                break;
            }
            let room = output_cap - out.len();
            let chunk = tokio::select! {
                biased;
                _ = req.cancel.cancelled() => {
                    let _ = child.kill().await;
                    return Err(HostError::Cancelled);
                }
                _ = tokio::time::sleep_until(deadline) => {
                    let _ = child.kill().await;
                    return Err(HostError::Timeout);
                }
                chunk = child.read_chunk(room.min(64 * 1024)) => chunk,
            };
            match chunk {
                Err(_) => {
                    let _ = child.kill().await;
                    return Err(HostError::Unavailable);
                }
                Ok(None) => break,
                Ok(Some(bytes)) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    out.extend_from_slice(&bytes);
                }
            }
        }
        let _ = child.kill().await;
        Ok(out)
    }

    async fn spawn_pty(&self, req: PtySpawnRequest<'_>) -> Result<PtyChild, HostError> {
        if req.cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        let executable = self
            .allowed_executable(req.executable)
            .ok_or(HostError::Policy)?;
        self.spawn_pty_inner(
            &executable,
            req.args,
            &req.env,
            &req.env_remove,
            &req.cancel,
        )
        .await
    }
}

impl AllowlistedPty {
    async fn spawn_pty_inner(
        &self,
        executable: &Path,
        args: &[&str],
        env: &[(String, String)],
        env_remove: &[String],
        cancel: &CancellationToken,
    ) -> Result<PtyChild, HostError> {
        if cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        #[cfg(unix)]
        {
            spawn_pty_child(executable, args, env, env_remove)
        }
        #[cfg(not(unix))]
        {
            let _ = (executable, args, env, env_remove);
            Err(HostError::Unavailable)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn arbitrary_executable_denied() {
        let host = AllowlistedPty::with_allowed([PathBuf::from("/bin/echo")]);
        let result = host
            .run(
                PtyRequest {
                    executable: Path::new("/bin/sh"),
                    args: &["-c", "echo pwned"],
                    timeout: Duration::from_secs(5),
                    output_cap: 64,
                    cancel: CancellationToken::new(),
                    env: vec![],
                    env_remove: vec![],
                },
                PtyInput { lines: vec![] },
            )
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }
    #[tokio::test]
    async fn cancellation_checked_before_backend() {
        let host = AllowlistedPty::with_allowed([PathBuf::from("/bin/echo")]);
        let token = CancellationToken::new();
        token.cancel();
        let result = host
            .run(
                PtyRequest {
                    executable: Path::new("/bin/echo"),
                    args: &[],
                    timeout: Duration::from_secs(5),
                    output_cap: 64,
                    cancel: token,
                    env: vec![],
                    env_remove: vec![],
                },
                PtyInput { lines: vec![] },
            )
            .await;
        assert!(matches!(result, Err(HostError::Cancelled)));
    }
    #[tokio::test]
    async fn interactive_echo_roundtrip_through_real_pty() {
        // /bin/cat under a real pty echoes typed lines back: proves
        // fork/exec/read/write/reap without any third-party CLI.
        let host = AllowlistedPty::with_allowed([PathBuf::from("/bin/cat")]);
        let mut child = host
            .spawn_pty(PtySpawnRequest {
                executable: Path::new("/bin/cat"),
                args: &[],
                cancel: CancellationToken::new(),
                env: vec![],
                env_remove: vec![],
            })
            .await
            .expect("spawn cat under pty");
        child.write_all(b"hello-pty\r").await.expect("write");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut collected = Vec::new();
        while tokio::time::Instant::now() < deadline {
            match child.read_chunk(4096).await.expect("read") {
                None => break,
                Some(bytes) => {
                    collected.extend_from_slice(&bytes);
                    if collected
                        .windows(b"hello-pty".len())
                        .any(|w| w == b"hello-pty")
                    {
                        break;
                    }
                }
            }
        }
        let text = String::from_utf8_lossy(&collected);
        assert!(text.contains("hello-pty"), "echo missing: {text:?}");
        child.kill().await.expect("kill");
    }
    #[tokio::test]
    async fn oneshot_run_caps_output_and_reaps() {
        // `yes` floods stdout: the output cap must bound the capture
        // and the owned child must not survive the call.
        let host = AllowlistedPty::with_allowed([PathBuf::from("/usr/bin/yes")]);
        let before = active_yes_count();
        let out = host
            .run(
                PtyRequest {
                    executable: Path::new("/usr/bin/yes"),
                    args: &[],
                    timeout: Duration::from_secs(10),
                    output_cap: 4096,
                    cancel: CancellationToken::new(),
                    env: vec![],
                    env_remove: vec![],
                },
                PtyInput { lines: vec![] },
            )
            .await
            .expect("capped run");
        assert!(out.len() <= 4096 + 64 * 1024, "cap exceeded: {}", out.len());
        assert!(!out.is_empty());
        // Poll for reaping: a real leak persists forever, but
        // scheduling delays under load must not flake the gate.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if active_yes_count() == before {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "yes child leaked");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    #[cfg(unix)]
    fn active_yes_count() -> usize {
        // Best-effort leak check via pgrep when available.
        std::process::Command::new("pgrep")
            .args(["-f", "yes"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .count()
            })
            .unwrap_or(0)
    }
    #[tokio::test]
    async fn scoped_env_rejected_before_fork() {
        let host = AllowlistedPty::with_allowed([PathBuf::from("/bin/echo")]);
        let result = host
            .spawn_pty(PtySpawnRequest {
                executable: Path::new("/bin/echo"),
                args: &[],
                cancel: CancellationToken::new(),
                env: vec![("LD_PRELOAD".into(), "/tmp/x".into())],
                env_remove: vec![],
            })
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }

    /// Credential scrubbing works through forkpty too: a planted
    /// ambient marker is unset before exec. Guard-restored runner env
    /// (see process tests for the blast-radius analysis).
    #[cfg(unix)]
    #[tokio::test]
    async fn pty_child_env_scrubbed_before_exec() {
        if !Path::new("/usr/bin/env").is_file() {
            return;
        }
        struct EnvGuard {
            prev: Option<std::ffi::OsString>,
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match &self.prev {
                    Some(value) => std::env::set_var("ANTHROPIC_AUTH_TOKEN", value),
                    None => std::env::remove_var("ANTHROPIC_AUTH_TOKEN"),
                }
            }
        }
        let prev = std::env::var_os("ANTHROPIC_AUTH_TOKEN");
        std::env::set_var("ANTHROPIC_AUTH_TOKEN", "marker-67890");
        let _guard = EnvGuard { prev };
        let host = AllowlistedPty::with_allowed([PathBuf::from("/usr/bin/env")]);
        let out = host
            .run(
                PtyRequest {
                    executable: Path::new("/usr/bin/env"),
                    args: &[],
                    timeout: Duration::from_secs(10),
                    output_cap: 65536,
                    cancel: CancellationToken::new(),
                    env: vec![],
                    env_remove: vec!["ANTHROPIC_AUTH_TOKEN".into()],
                },
                PtyInput { lines: vec![] },
            )
            .await
            .expect("env dump under pty");
        let text = String::from_utf8_lossy(&out);
        assert!(
            !text.contains("marker-67890"),
            "scrubbed variable leaked into pty child env"
        );
    }
}
