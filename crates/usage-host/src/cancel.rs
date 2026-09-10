use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
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
    children: Mutex<Vec<Weak<TokenInner>>>,
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
                children: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Create a token cancelled by any parent. This keeps account disable,
    /// account deletion and application shutdown on one host-compatible
    /// token without spawning a watcher task for every refresh.
    pub fn linked(parents: impl IntoIterator<Item = CancellationToken>) -> Self {
        let child = Self::new();
        for parent in parents {
            parent.add_child(&child);
        }
        child
    }

    fn add_child(&self, child: &CancellationToken) {
        if self.is_cancelled() {
            child.cancel();
            return;
        }
        if let Ok(mut children) = self.inner.children.lock() {
            children.push(Arc::downgrade(&child.inner));
        }
        // Close the small check/register race: a parent cancelled between
        // the first check and registration still cancels this child.
        if self.is_cancelled() {
            child.cancel();
        }
    }

    pub fn cancel(&self) {
        if self.inner.flag.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.notify.notify_waiters();
        let children = self
            .inner
            .children
            .lock()
            .map(|mut children| {
                children.retain(|child| child.strong_count() > 0);
                children
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for child in children {
            CancellationToken { inner: child }.cancel();
        }
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
