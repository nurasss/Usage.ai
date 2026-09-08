use super::{CancellationToken, HostError};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Bounded PTY session request. Like `ProcessHost`, arguments are
/// passed directly — never through a shell.
pub struct PtyRequest<'a> {
    pub executable: &'a Path,
    pub args: &'a [&'a str],
    pub timeout: Duration,
    pub output_cap: usize,
    pub cancel: CancellationToken,
}

/// Controlled PTY input: line-oriented writes only.
pub struct PtyInput {
    pub lines: Vec<String>,
}

/// Framework abstraction for future CLI strategies (e.g. App Server
/// consoles). The reference implementation below enforces the full
/// security policy while the native PTY backend is still pending:
/// allow-listed executables, cancellation and owned-only termination.
#[async_trait]
pub trait PTYHost: Send + Sync {
    async fn run(&self, req: PtyRequest<'_>, input: PtyInput) -> Result<Vec<u8>, HostError>;
}

/// Policy-enforcing placeholder. Validates the executable allow-list
/// and cancellation before reporting the backend as unavailable, so
/// policy tests stay meaningful without a native PTY dependency.
#[derive(Debug, Default)]
pub struct AllowlistedPty {
    allowed: std::sync::Mutex<std::collections::HashSet<PathBuf>>,
}

impl AllowlistedPty {
    pub fn with_allowed(executables: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            allowed: std::sync::Mutex::new(executables.into_iter().collect()),
        }
    }
}

#[async_trait]
impl PTYHost for AllowlistedPty {
    async fn run(&self, req: PtyRequest<'_>, _input: PtyInput) -> Result<Vec<u8>, HostError> {
        if req.cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        let allowed = req.executable.canonicalize().ok().is_some_and(|canonical| {
            self.allowed
                .lock()
                .map(|set| set.contains(&canonical))
                .unwrap_or(false)
        });
        if !allowed {
            return Err(HostError::Policy);
        }
        // No native PTY backend is wired in this cycle: no CLI strategy
        // may depend on PTY execution yet.
        Err(HostError::Unavailable)
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
                },
                PtyInput { lines: vec![] },
            )
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }
    #[tokio::test]
    async fn cancellation_checked_before_backend() {
        let host = AllowlistedPty::with_allowed([PathBuf::from("/bin/echo")]);
        let result = host
            .run(
                PtyRequest {
                    executable: Path::new("/bin/echo"),
                    args: &[],
                    timeout: Duration::from_secs(5),
                    output_cap: 64,
                    cancel: cancelled_token(),
                },
                PtyInput { lines: vec![] },
            )
            .await;
        assert!(matches!(result, Err(HostError::Cancelled)));
    }
}
