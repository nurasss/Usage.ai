use chrono::{DateTime, Utc};
use std::time::Duration;

/// Refresh/retry policy shared by the runtime coordinator.
/// `scheduler.rs` re-exports these symbols for compatibility.
pub use crate::scheduler::{manual_refresh_allowed, retry_decision, RetryDecision};

/// Target cadences (§14.2). Active providers (fresh local evidence)
/// refresh fast; idle ones follow the configured interval with a
/// 5-minute floor. Exact numbers live HERE, never scattered across
/// UI or provider code.
pub const ACTIVE_CADENCE: Duration = Duration::from_secs(120);
pub const NORMAL_CADENCE_FLOOR: Duration = Duration::from_secs(300);
/// A scope counts as active while its source emits observations
/// (local history writes, usage events) — no inference involved.
pub const ACTIVITY_WINDOW: Duration = Duration::from_secs(900);
/// Offline multiplier: local sources keep working without internet,
/// just less often.
pub const OFFLINE_FACTOR: u64 = 4;
/// A tick gap beyond this multiple of the interval means the machine
/// slept: refresh once on wake, never replay missed ticks.
pub const WAKE_GAP_MULTIPLE: u64 = 3;

/// Is a scope due for refresh this tick?
/// - manual refresh is always due (immediate + coalesced; persisted
///   cooldowns are still enforced inside execution);
/// - a cooling scope is never due (server cooldowns are never bypassed);
/// - a never-refreshed scope is due (bootstrap);
/// - an active scope follows ACTIVE_CADENCE;
/// - otherwise the configured interval with a NORMAL floor.
pub fn refresh_due(
    last_success: Option<DateTime<Utc>>,
    last_observed: Option<DateTime<Utc>>,
    cooling: bool,
    manual: bool,
    configured_interval: Duration,
    now: DateTime<Utc>,
) -> bool {
    if manual || cooling {
        return manual && !cooling;
    }
    let Some(last_success) = last_success else {
        return true;
    };
    let elapsed = now
        .signed_duration_since(last_success)
        .to_std()
        .unwrap_or(Duration::ZERO);
    let active = last_observed.is_some_and(|observed| {
        now.signed_duration_since(observed)
            .to_std()
            .is_ok_and(|age| age <= ACTIVITY_WINDOW)
    });
    if active {
        elapsed >= ACTIVE_CADENCE
    } else {
        elapsed >= configured_interval.max(NORMAL_CADENCE_FLOOR)
    }
}

/// Did the scheduler likely sleep through ticks? Pure so the wake
/// path stays unit-testable.
pub fn wake_gap_exceeded(last_tick: DateTime<Utc>, interval: Duration, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(last_tick)
        .to_std()
        .is_ok_and(|gap| gap > interval * WAKE_GAP_MULTIPLE as u32)
}

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
    use chrono::Duration as ChronoDuration;
    fn ago(secs: i64) -> DateTime<Utc> {
        Utc::now() - ChronoDuration::seconds(secs)
    }
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
    #[test]
    fn due_matrix_covers_bootstrap_manual_cooldown_activity() {
        let now = Utc::now();
        let interval = Duration::from_secs(300);
        // Bootstrap: never refreshed.
        assert!(refresh_due(None, None, false, false, interval, now));
        // Manual bypasses cadence but never cooldowns.
        assert!(refresh_due(Some(ago(10)), None, false, true, interval, now));
        assert!(!refresh_due(Some(ago(10)), None, true, true, interval, now));
        assert!(!refresh_due(Some(ago(10)), None, true, false, interval, now));
        // Active scope (fresh observations): 2-minute cadence.
        assert!(refresh_due(Some(ago(180)), Some(ago(60)), false, false, interval, now));
        assert!(!refresh_due(Some(ago(60)), Some(ago(30)), false, false, interval, now));
        // Idle scope: configured interval with a 5-minute floor.
        assert!(refresh_due(Some(ago(400)), Some(ago(3600)), false, false, interval, now));
        assert!(!refresh_due(Some(ago(100)), Some(ago(3600)), false, false, interval, now));
        // Stale observations are not activity.
        assert!(!refresh_due(Some(ago(100)), Some(ago(2000)), false, false, interval, now));
    }
    #[test]
    fn wake_gap_needs_triple_interval() {
        let now = Utc::now();
        let interval = Duration::from_secs(120);
        assert!(!wake_gap_exceeded(now, interval, now));
        assert!(!wake_gap_exceeded(
            now - ChronoDuration::seconds(300),
            interval,
            now
        ));
        assert!(wake_gap_exceeded(
            now - ChronoDuration::seconds(400),
            interval,
            now
        ));
    }
}
