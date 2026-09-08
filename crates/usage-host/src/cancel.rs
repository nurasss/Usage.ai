use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Notify;

/// Cooperative cancellation primitive shared by the runtime,
/// strategies and every host boundary. Clones refer to the same
/// cancellation state. Dropping never cancels; only `cancel()` does.
///
/// Implemented over an atomic flag plus a waker (a bare watch channel
/// would silently drop `send` when no receiver exists yet).
#[derive(Debug, Clone)]
pub struct CancellationToken {
    inner: Arc<TokenInner>,
}

#[derive(Debug)]
struct TokenInner {
    flag: AtomicBool,
    notify: Notify,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(TokenInner {
                flag: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub fn cancel(&self) {
        self.inner.flag.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::SeqCst)
    }

    /// Resolves when `cancel()` is called. Dropping the future does
    /// not cancel anything.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            self.inner.notify.notified().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_is_observed_by_clones() {
        let token = CancellationToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled());
    }
    #[tokio::test]
    async fn cancelled_future_resolves() {
        let token = CancellationToken::new();
        let waiter = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            waiter.cancel();
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), token.cancelled())
            .await
            .expect("cancellation must resolve");
    }
}
