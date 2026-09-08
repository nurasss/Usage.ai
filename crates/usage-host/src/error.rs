/// Safe host error codes. Display strings never contain secrets,
/// paths, tokens, headers or response bodies.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("host unavailable")]
    Unavailable,
    #[error("access denied")]
    Denied,
    #[error("insufficient scope")]
    Forbidden,
    #[error("rate limited")]
    RateLimited(Option<u64>),
    #[error("not found")]
    NotFound,
    #[error("policy violation")]
    Policy,
    #[error("operation timed out")]
    Timeout,
}

pub fn safe_code(error: &HostError) -> &'static str {
    match error {
        HostError::Unavailable => "host_unavailable",
        HostError::Denied => "access_denied",
        HostError::Forbidden => "insufficient_scope",
        HostError::RateLimited(_) => "rate_limited",
        HostError::NotFound => "not_found",
        HostError::Policy => "policy_violation",
        HostError::Timeout => "operation_timeout",
    }
}
