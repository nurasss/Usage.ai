use std::time::Duration;

/// Refresh/retry policy shared by the runtime coordinator.
/// `scheduler.rs` re-exports these symbols for compatibility.
pub use crate::scheduler::{manual_refresh_allowed, retry_decision, RetryDecision};

/// Compute the effective cooldown delay for a failed attempt:
/// explicit `Retry-After` wins, auth errors never retry aggressively,
/// everything else uses exponential backoff with jitter.
pub fn cooldown_for_attempt(
    status: Option<u16>,
    attempt: u32,
    retry_after: Option<Duration>,
    offline: bool,
) -> Option<Duration> {
    retry_decision(status, attempt, retry_after, offline).delay
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_after_wins_over_backoff() {
        assert_eq!(
            cooldown_for_attempt(Some(429), 3, Some(Duration::from_secs(77)), false),
            Some(Duration::from_secs(77))
        );
    }
    #[test]
    fn auth_errors_stay_non_retryable() {
        assert_eq!(cooldown_for_attempt(Some(401), 0, None, false), None);
        assert_eq!(cooldown_for_attempt(Some(403), 0, None, false), None);
    }
}
