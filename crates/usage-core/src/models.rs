use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: Uuid,
    pub provider_id: String,
    pub external_identity: Option<String>,
    pub label: String,
    pub connection_ref: Option<String>,
    pub lifecycle: AccountLifecycle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AccountLifecycle {
    Active,
    Disconnected,
    Archived,
}

/// How strongly the stored account identity is proven.
/// `Unknown` local data is never auto-merged with another account.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum IdentityConfidence {
    Verified,
    Weak,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub id: String,
    pub provider_id: String,
    pub kind: String,
    pub billing_scope: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    SubscriptionQuota,
    QuotaResetTime,
    ApiTokens,
    ApiRequests,
    ApiCostReported,
    ApiCostEstimated,
    Balance,
    Credits,
    ModelBreakdown,
    ProjectBreakdown,
    HistoryRemote,
    HistoryLocal,
    LocalSessions,
    MultiAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub account_id: Uuid,
    pub provider_id: String,
    pub product_id: String,
    pub plan_label: Option<String>,
    pub capabilities: BTreeSet<Capability>,
    pub quotas: Vec<Quota>,
    #[serde(default)]
    pub balances: Vec<MoneyBalance>,
    pub observed_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
    pub connection_state: ConnectionState,
    pub freshness: Freshness,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    pub pool_id: String,
    pub window_id: Option<String>,
    pub name: String,
    pub used: Option<u64>,
    pub limit: Option<u64>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub used_percent: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub remaining_percent: Option<Decimal>,
    pub unit: MetricUnit,
    pub window_start: Option<DateTime<Utc>>,
    pub resets_at: Option<DateTime<Utc>>,
    pub window_kind: WindowKind,
    pub source: String,
}

impl Quota {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for value in [self.used_percent, self.remaining_percent]
            .into_iter()
            .flatten()
        {
            if value < Decimal::ZERO || value > Decimal::ONE_HUNDRED {
                return Err(ValidationError::InvalidPercentage(value));
            }
        }
        if let (Some(used), Some(limit)) = (self.used, self.limit) {
            if used > limit && self.unit != MetricUnit::Requests {
                return Err(ValidationError::UsageAboveLimit { used, limit });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MetricUnit {
    Percent,
    Tokens,
    Requests,
    Credits,
    Currency,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowKind {
    Fixed,
    Rolling,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Refreshing,
    NotConfigured,
    AuthenticationRequired,
    SessionExpired,
    PermissionDenied,
    InsufficientScope,
    ApplicationNotRunning,
    RateLimited,
    Cooldown,
    Unavailable,
    UnsupportedSource,
    ParseError,
    NetworkError,
    Offline,
    UnknownError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "ageSeconds")]
pub enum Freshness {
    Fresh,
    Stale(u64),
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Coverage {
    Complete,
    Partial,
    LocalClientOnly,
    /// Source shape is parsed, but real-client semantic evidence is still
    /// required before totals can be called authoritative.
    UnverifiedSemantics,
    FromConnectionTime,
    ProviderDelayed,
    Unknown,
}

impl Coverage {
    /// Returns true if this coverage represents trustworthy, authoritative data
    /// eligible for authoritative totals, primary tray percentage, and native alerts.
    /// UnverifiedSemantics, LocalClientOnly, and Unknown are non-authoritative.
    pub fn is_authoritative(&self) -> bool {
        matches!(self, Coverage::Complete | Coverage::Partial)
    }

    /// Primary menu-bar / tray metric selection allows only authoritative sources.
    pub fn allows_tray_metric(&self) -> bool {
        self.is_authoritative()
    }

    /// Authoritative quota notifications (threshold, pace, reset) allow only authoritative sources.
    pub fn allows_quota_notifications(&self) -> bool {
        self.is_authoritative()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub account_id: Uuid,
    pub product_id: String,
    pub billing_scope_id: Option<String>,
    pub source_record_id: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Aggregate cache count retained for cross-provider totals and
    /// backwards-compatible consumers. Claude records additionally retain
    /// the source's read/create buckets below.
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub requests: Option<u64>,
    pub source: String,
    pub coverage: Coverage,
    /// Source-file provenance (§P0-01). Legacy rows carry `None` and are
    /// treated as generation 1 of an unattributed import.
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub file_generation: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CostRecord {
    pub account_id: Uuid,
    pub product_id: String,
    pub billing_scope_id: Option<String>,
    pub source_record_id: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount_decimal: Decimal,
    pub currency: String,
    pub kind: CostKind,
    pub source: String,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CostKind {
    Reported,
    Estimated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MoneyBalance {
    pub account_id: Uuid,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount_decimal: Decimal,
    pub currency: String,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValidationError {
    #[error("percentage outside 0..=100: {0}")]
    InvalidPercentage(Decimal),
    #[error("usage {used} is above limit {limit}")]
    UsageAboveLimit { used: u64, limit: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    #[test]
    fn decimal_percents_roundtrip_exactly_through_json() {
        // LKG snapshots serialize quotas to SQLite JSON: 32.5 and
        // 19.99 must survive the round trip bit-exact, never via f64.
        let quota = Quota {
            pool_id: "x".into(),
            window_id: None,
            name: "x".into(),
            used: None,
            limit: None,
            used_percent: Some(Decimal::from_str("32.5").unwrap()),
            remaining_percent: Some(Decimal::from_str("19.99").unwrap()),
            unit: MetricUnit::Percent,
            window_start: None,
            resets_at: None,
            window_kind: WindowKind::Unknown,
            source: "fixture".into(),
        };
        let json = serde_json::to_string(&quota).unwrap();
        assert!(json.contains("32.5") && json.contains("19.99"));
        let back: Quota = serde_json::from_str(&json).unwrap();
        assert_eq!(back, quota);
    }
    #[test]
    fn rejects_invalid_percentage() {
        let quota = Quota {
            pool_id: "x".into(),
            window_id: None,
            name: "x".into(),
            used: None,
            limit: None,
            used_percent: None,
            remaining_percent: Some(Decimal::new(101, 0)),
            unit: MetricUnit::Percent,
            window_start: None,
            resets_at: None,
            window_kind: WindowKind::Unknown,
            source: "fixture".into(),
        };
        assert!(matches!(
            quota.validate(),
            Err(ValidationError::InvalidPercentage(_))
        ));
    }
    #[test]
    fn preserves_absence_in_json() {
        let quota = Quota {
            pool_id: "x".into(),
            window_id: None,
            name: "x".into(),
            used: None,
            limit: None,
            used_percent: None,
            remaining_percent: None,
            unit: MetricUnit::Percent,
            window_start: None,
            resets_at: None,
            window_kind: WindowKind::Unknown,
            source: "fixture".into(),
        };
        let json = serde_json::to_value(quota).unwrap();
        assert!(json["remainingPercent"].is_null());
    }
}
