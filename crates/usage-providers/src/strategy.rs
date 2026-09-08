use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;
use usage_core::{MoneyBalance, Snapshot, UsageRecord};
use usage_host::Hosts;

/// Stable source identity inside a product pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(pub &'static str);

/// Source classification with explicit default trust order.
/// Order never authorizes an automatic fallback through an unsafe
/// source — user policy and permissions always win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceClassification {
    OfficialApi,
    SupportedClientApi,
    LocalStructuredData,
    OfficialExport,
    PrivateClientApi,
    BrowserSession,
    HtmlFallback,
}

pub fn trust_order(class: SourceClassification) -> u8 {
    match class {
        SourceClassification::OfficialApi => 0,
        SourceClassification::SupportedClientApi => 1,
        SourceClassification::LocalStructuredData => 2,
        SourceClassification::OfficialExport => 3,
        SourceClassification::PrivateClientApi => 4,
        SourceClassification::BrowserSession => 5,
        SourceClassification::HtmlFallback => 6,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Ready,
    NotConfigured,
    Disabled,
    Unsupported,
}

/// Terminal errors stop the pipeline; anything else may fall through
/// to the next strategy within policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    AuthenticationRequired,
    InsufficientScope,
    PermissionDenied,
    IdentityMismatch,
    UnsafeSourceDisabled,
    UserDisabled,
    RateLimited { retry_after_secs: Option<u64> },
    Cooldown,
    Network,
    Parse { schema: &'static str },
    Unavailable,
    Cancelled,
    Policy,
}

impl SourceError {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            SourceError::PermissionDenied
                | SourceError::IdentityMismatch
                | SourceError::UnsafeSourceDisabled
                | SourceError::UserDisabled
                | SourceError::Cooldown
                | SourceError::AuthenticationRequired
                | SourceError::InsufficientScope
        )
    }

    pub fn safe_code(&self) -> &'static str {
        match self {
            SourceError::AuthenticationRequired => "authentication_required",
            SourceError::InsufficientScope => "insufficient_scope",
            SourceError::PermissionDenied => "permission_denied",
            SourceError::IdentityMismatch => "identity_mismatch",
            SourceError::UnsafeSourceDisabled => "unsafe_source_disabled",
            SourceError::UserDisabled => "source_disabled",
            SourceError::RateLimited { .. } => "rate_limited",
            SourceError::Cooldown => "source_cooldown",
            SourceError::Network => "network_error",
            SourceError::Parse { .. } => "parse_error",
            SourceError::Unavailable => "unavailable",
            SourceError::Cancelled => "cancelled",
            SourceError::Policy => "policy_violation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackDecision {
    Next,
    Stop,
}

///Validated domain payload produced by one strategy execution.
#[derive(Debug)]
pub struct FetchPayload {
    pub snapshot: Option<Snapshot>,
    pub usage: Vec<UsageRecord>,
    pub costs: Vec<usage_core::CostRecord>,
    pub balances: Vec<MoneyBalance>,
    pub warnings: Vec<String>,
    pub schema_fingerprint: &'static str,
    pub coverage: usage_core::Coverage,
    pub observed_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct FetchContext<'a> {
    pub account: &'a usage_core::Account,
    pub hosts: &'a Hosts,
    pub timeout: Duration,
    /// Product-scoped local root override (user-selected custom path).
    pub custom_root: Option<PathBuf>,
    /// Secret supplied by the runtime from Keychain for this call only.
    pub secret: Option<Vec<u8>>,
}

#[async_trait]
pub trait FetchStrategy: Send + Sync {
    fn source_id(&self) -> SourceId;
    fn classification(&self) -> SourceClassification;
    /// Kill-switch key for private strategies; empty means always on.
    fn kill_switch(&self) -> &'static str {
        ""
    }
    async fn availability(&self, ctx: &FetchContext<'_>) -> Availability;
    async fn fetch(&self, ctx: &FetchContext<'_>) -> Result<FetchPayload, SourceError>;
    fn fallback_on(&self, error: &SourceError) -> FallbackDecision {
        if error.is_terminal() {
            FallbackDecision::Stop
        } else {
            FallbackDecision::Next
        }
    }
}

pub fn map_host_error(error: &usage_host::HostError) -> SourceError {
    match error {
        usage_host::HostError::Denied => SourceError::AuthenticationRequired,
        usage_host::HostError::Forbidden => SourceError::InsufficientScope,
        usage_host::HostError::RateLimited(retry_after_secs) => SourceError::RateLimited {
            retry_after_secs: *retry_after_secs,
        },
        usage_host::HostError::Timeout | usage_host::HostError::Unavailable => SourceError::Network,
        usage_host::HostError::NotFound => SourceError::Unavailable,
        usage_host::HostError::Policy => SourceError::Policy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_errors_stop_pipeline() {
        assert!(SourceError::PermissionDenied.is_terminal());
        assert!(SourceError::IdentityMismatch.is_terminal());
        assert!(SourceError::UserDisabled.is_terminal());
        assert!(!SourceError::Network.is_terminal());
        assert!(!SourceError::Unavailable.is_terminal());
        assert!(!SourceError::Parse { schema: "x" }.is_terminal());
    }
    #[test]
    fn trust_order_prefers_official() {
        assert!(
            trust_order(SourceClassification::OfficialApi)
                < trust_order(SourceClassification::LocalStructuredData)
        );
        assert!(
            trust_order(SourceClassification::PrivateClientApi)
                < trust_order(SourceClassification::BrowserSession)
        );
    }
    #[test]
    fn safe_codes_contain_no_payload() {
        for error in [
            SourceError::AuthenticationRequired,
            SourceError::RateLimited {
                retry_after_secs: Some(5),
            },
            SourceError::Parse { schema: "v1" },
        ] {
            assert!(!error.safe_code().contains('5'));
            assert!(!error.safe_code().contains("v1"));
        }
    }
}
