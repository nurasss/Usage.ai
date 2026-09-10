use async_trait::async_trait;
use std::time::Duration;
use usage_core::{MoneyBalance, Snapshot, UsageRecord};
use usage_host::{ProviderHostFacade, ScopedRoot};

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
    CredentialExpired,
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
                | SourceError::CredentialExpired
                | SourceError::InsufficientScope
                | SourceError::Cancelled
        )
    }

    pub fn safe_code(&self) -> &'static str {
        match self {
            SourceError::AuthenticationRequired => "authentication_required",
            SourceError::CredentialExpired => "credential_expired",
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
    /// Source-observed account identity fingerprint, when the source
    /// proves one (never an email, token, or display name). The runtime
    /// routes on it: mismatch discards the payload without commit.
    pub observed_identity: Option<String>,
    /// A source may commit a valid partial result while reporting the
    /// failure that stopped pagination. This is intentionally separate from
    /// `Result::Err`: page 1 data must not be discarded when page 2 fails.
    pub source_error: Option<SourceError>,
}

pub struct FetchContext<'a> {
    pub account: &'a usage_core::Account,
    /// ProviderHostFacade is constructed by runtime policy. Strategies do
    /// not receive the unrestricted host bundle.
    pub facade: ProviderHostFacade<'a>,
    pub descriptor: &'static crate::descriptor::ProductDescriptor,
    pub timeout: Duration,
    /// Product root was resolved and scoped by runtime before strategy
    /// invocation. Providers can only use the opaque root and scoped file
    /// methods exposed by the facade.
    pub local_root: Option<ScopedRoot>,
    /// Secret supplied by the runtime from Keychain for this call only.
    pub secret: Option<Vec<u8>>,
    /// Cooperative cancellation: strategies and hosts must observe it
    /// at every await/yield boundary.
    pub cancel: usage_host::CancellationToken,
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

/// Byte-level log parser for file-backed strategies. Parsing is pure
/// over caller-supplied bytes: no filesystem, no environment, no
/// network. The runtime supplies scoped reads and commits results
/// with file provenance.
pub trait FileLogParser: Send + Sync {
    fn source_id(&self) -> SourceId;
    fn schema_fingerprint(&self) -> &'static str;
    fn parse_chunk(
        &self,
        account: &usage_core::Account,
        path_hash: &str,
        base_offset: u64,
        bytes: &[u8],
    ) -> ParsedChunk;
}

/// One parsed byte range: records plus resume metadata. `consumed`
/// is the next offset to read from; a trailing partial line is never
/// consumed. `malformed` counts unparseable JSON lines separately
/// from silently skipped non-event lines.
#[derive(Debug, Default)]
pub struct ParsedChunk {
    pub records: Vec<UsageRecord>,
    pub consumed: u64,
    pub partial: bool,
    pub warnings: Vec<String>,
    pub malformed: usize,
}

pub fn map_host_error(error: &usage_host::HostError) -> SourceError {
    match error {
        usage_host::HostError::Denied => SourceError::AuthenticationRequired,
        usage_host::HostError::Forbidden => SourceError::InsufficientScope,
        usage_host::HostError::RateLimited(retry_after_secs) => SourceError::RateLimited {
            retry_after_secs: *retry_after_secs,
        },
        usage_host::HostError::Timeout | usage_host::HostError::Unavailable => SourceError::Network,
        usage_host::HostError::Cancelled => SourceError::Cancelled,
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
