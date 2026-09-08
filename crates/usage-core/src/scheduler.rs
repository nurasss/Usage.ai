use rand::Rng;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryDecision {
    pub delay: Option<Duration>,
    pub reason: &'static str,
}

pub fn retry_decision(
    status: Option<u16>,
    attempt: u32,
    retry_after: Option<Duration>,
    offline: bool,
) -> RetryDecision {
    if offline {
        return RetryDecision {
            delay: None,
            reason: "offline_until_network_change",
        };
    }
    match status {
        Some(401 | 403) => RetryDecision {
            delay: None,
            reason: "authentication_or_scope",
        },
        Some(429) => RetryDecision {
            delay: Some(retry_after.unwrap_or(Duration::from_secs(300))),
            reason: "source_cooldown",
        },
        Some(500..=599) | None => {
            let cap = 15_u64
                .saturating_mul(2_u64.saturating_pow(attempt.min(6)))
                .min(900);
            let jitter = rand::rng().random_range(80_u64..=120_u64);
            RetryDecision {
                delay: Some(Duration::from_secs(cap * jitter / 100)),
                reason: "transient_backoff",
            }
        }
        _ => RetryDecision {
            delay: None,
            reason: "non_retryable",
        },
    }
}

pub fn manual_refresh_allowed(cooldown_remaining: Option<Duration>) -> bool {
    cooldown_remaining.is_none_or(|value| value.is_zero())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_is_not_retried() {
        assert_eq!(retry_decision(Some(401), 1, None, false).delay, None);
    }
    #[test]
    fn retry_after_wins() {
        assert_eq!(
            retry_decision(Some(429), 1, Some(Duration::from_secs(77)), false).delay,
            Some(Duration::from_secs(77))
        );
    }
    #[test]
    fn offline_waits_for_signal() {
        assert_eq!(
            retry_decision(None, 4, None, true).reason,
            "offline_until_network_change"
        );
    }
    #[test]
    fn manual_refresh_respects_cooldown() {
        assert!(!manual_refresh_allowed(Some(Duration::from_secs(5))));
    }
}
