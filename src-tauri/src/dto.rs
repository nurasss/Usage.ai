use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use usage_core::{Capability, ConnectionState, Coverage, Freshness, MoneyBalance, Snapshot};
use usage_storage::ManagedAccount;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub launch_at_login: bool,
    pub refresh_interval_minutes: Option<u64>,
    pub menu_bar_mode: String,
    pub global_shortcut: String,
    pub theme: String,
    pub retention_days: u64,
    pub quota_warning_percent: u8,
    pub notifications_enabled: bool,
    pub quiet_hours_start: Option<String>,
    pub quiet_hours_end: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            refresh_interval_minutes: Some(5),
            menu_bar_mode: "icon".into(),
            global_shortcut: "Ctrl+Alt+U".into(),
            theme: "system".into(),
            retention_days: 90,
            quota_warning_percent: 20,
            notifications_enabled: false,
            quiet_hours_start: None,
            quiet_hours_end: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostSummaryDto {
    pub reported: Vec<MoneyDto>,
    pub estimated: Vec<MoneyDto>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub mode: String,
    pub offline: bool,
    pub providers: Vec<ProviderDto>,
    pub overview: HashMap<String, Vec<OverviewSegment>>,
    #[serde(default)]
    pub model_breakdown: HashMap<String, Vec<OverviewSegment>>,
    #[serde(default)]
    pub account_breakdown: HashMap<String, Vec<OverviewSegment>>,
    #[serde(default)]
    pub project_breakdown: HashMap<String, Vec<OverviewSegment>>,
    #[serde(default)]
    pub costs_by_period: HashMap<String, Vec<ProviderCostDto>>,
    #[serde(default)]
    pub unverified_overview: HashMap<String, Vec<OverviewSegment>>,
    #[serde(default)]
    pub excluded_unverified_count: HashMap<String, u64>,
    pub next_refresh_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCostDto {
    pub account_id: String,
    pub product_id: String,
    pub label: String,
    pub reported: Vec<MoneyDto>,
    pub estimated: Vec<MoneyDto>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub account_id: String,
    pub provider_id: String,
    pub product_id: String,
    pub provider_name: String,
    pub product_name: String,
    pub glyph: String,
    pub color: String,
    pub alias: String,
    pub plan_label: Option<String>,
    pub connection_state: ConnectionState,
    pub freshness: Freshness,
    pub coverage: Coverage,
    pub fetched_at: String,
    pub observed_at: Option<String>,
    pub capabilities: Vec<String>,
    pub quotas: Vec<QuotaDto>,
    pub tokens_today: Option<u64>,
    pub reported_cost_today: Option<MoneyDto>,
    pub estimated_cost_today: Option<MoneyDto>,
    #[serde(default)]
    pub balances: Vec<MoneyDto>,
    /// Refresh metadata is independent from the snapshot payload. This
    /// lets SWR expose a current failure alongside last-known-good data.
    #[serde(default)]
    pub current_error: Option<String>,
    #[serde(default)]
    pub last_successful_refresh: Option<String>,
    #[serde(default)]
    pub last_refresh_attempt: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaDto {
    pub pool_id: String,
    pub window_id: Option<String>,
    pub name: String,
    pub remaining_percent: Option<f64>,
    pub used: Option<u64>,
    pub limit: Option<u64>,
    pub unit: String,
    pub resets_at: Option<String>,
    pub window_start: Option<String>,
    pub window_kind: String,
    pub source: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MoneyDto {
    pub amount: String,
    pub currency: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OverviewSegment {
    pub label: String,
    pub value: u64,
    pub color: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsDto {
    pub provider: String,
    pub product: String,
    pub account_alias: String,
    pub selected_source: Option<String>,
    pub connection_state: ConnectionState,
    pub last_refresh_attempt: Option<String>,
    pub last_successful_refresh: Option<String>,
    pub last_data_observed_at: Option<String>,
    pub freshness: Freshness,
    pub coverage: Coverage,
    pub status_class: Option<String>,
    pub connector_version: String,
    pub parser_version: String,
    pub schema_fingerprint: Option<String>,
    pub capabilities_detected: Vec<String>,
    pub cooldown_until: Option<String>,
    pub last_safe_error_code: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub recent_attempts: Vec<AttemptDto>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptDto {
    pub source: String,
    pub status: String,
    pub safe_code: Option<String>,
    pub finished_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    pub id: String,
    pub provider_id: String,
    pub product_id: Option<String>,
    pub label: String,
    pub lifecycle: String,
    pub connection_ref: Option<String>,
    pub enabled: bool,
    pub custom_path: Option<String>,
    pub identity_confidence: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetDto {
    pub account_id: String,
    pub product_id: String,
    pub currency: String,
    pub amount: String,
    pub period: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub update_channel: Option<String>,
    pub updates_enabled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStatsDto {
    pub product_id: String,
    pub files_discovered: usize,
    pub files_imported: usize,
    pub records_accepted: usize,
    pub malformed: usize,
    pub checkpoint_resets: usize,
    pub warnings: Vec<String>,
}

pub fn managed_account_to_dto(a: &ManagedAccount) -> AccountDto {
    AccountDto {
        id: a.id.to_string(),
        provider_id: a.provider_id.clone(),
        product_id: a.product_id.clone(),
        label: a.label.clone(),
        lifecycle: format!("{:?}", a.lifecycle),
        connection_ref: a.connection_ref.clone(),
        enabled: a.enabled,
        custom_path: a.custom_path.clone(),
        identity_confidence: format!("{:?}", a.identity_confidence),
    }
}

pub fn from_snapshot(
    s: Snapshot,
    provider_name: &str,
    product_name: &str,
    alias: &str,
) -> ProviderDto {
    let descriptor = usage_providers::descriptor::find_descriptor(&s.provider_id, &s.product_id);
    let glyph = descriptor.map(|descriptor| descriptor.glyph).unwrap_or("•");
    let color = descriptor
        .map(|descriptor| descriptor.color)
        .unwrap_or("#8ea2ff");
    ProviderDto {
        account_id: s.account_id.to_string(),
        provider_id: s.provider_id,
        product_id: s.product_id,
        provider_name: provider_name.into(),
        product_name: product_name.into(),
        glyph: glyph.into(),
        color: color.into(),
        alias: alias.into(),
        plan_label: s.plan_label,
        connection_state: s.connection_state,
        freshness: s.freshness,
        coverage: s.coverage,
        fetched_at: s.fetched_at.to_rfc3339(),
        observed_at: s.observed_at.map(|d| d.to_rfc3339()),
        capabilities: s.capabilities.into_iter().map(capability_name).collect(),
        quotas: s
            .quotas
            .into_iter()
            .map(|q| QuotaDto {
                pool_id: q.pool_id,
                window_id: q.window_id,
                name: q.name,
                remaining_percent: q.remaining_percent.and_then(|v| v.to_string().parse().ok()),
                used: q.used,
                limit: q.limit,
                unit: format!("{:?}", q.unit).to_lowercase(),
                resets_at: q.resets_at.map(|d| d.to_rfc3339()),
                window_start: q.window_start.map(|d| d.to_rfc3339()),
                window_kind: format!("{:?}", q.window_kind).to_lowercase(),
                source: q.source,
            })
            .collect(),
        tokens_today: None,
        reported_cost_today: None,
        estimated_cost_today: None,
        balances: s
            .balances
            .into_iter()
            .map(|b: MoneyBalance| MoneyDto {
                amount: b.amount_decimal.to_string(),
                currency: b.currency,
            })
            .collect(),
        current_error: None,
        last_successful_refresh: None,
        last_refresh_attempt: None,
    }
}

pub fn unavailable(
    provider_id: &str,
    product_id: &str,
    provider_name: &str,
    product_name: &str,
    state: ConnectionState,
) -> ProviderDto {
    ProviderDto {
        account_id: format!("{provider_id}-{product_id}"),
        provider_id: provider_id.into(),
        product_id: product_id.into(),
        provider_name: provider_name.into(),
        product_name: product_name.into(),
        glyph: usage_providers::descriptor::find_descriptor(provider_id, product_id)
            .map(|descriptor| descriptor.glyph)
            .unwrap_or("•")
            .into(),
        color: usage_providers::descriptor::find_descriptor(provider_id, product_id)
            .map(|descriptor| descriptor.color)
            .unwrap_or("#8ea2ff")
            .into(),
        alias: "Не подключено".into(),
        plan_label: None,
        connection_state: state,
        freshness: Freshness::Unknown,
        coverage: Coverage::Unknown,
        fetched_at: chrono::Utc::now().to_rfc3339(),
        observed_at: None,
        capabilities: vec![],
        quotas: vec![],
        tokens_today: None,
        reported_cost_today: None,
        estimated_cost_today: None,
        balances: vec![],
        current_error: None,
        last_successful_refresh: None,
        last_refresh_attempt: None,
    }
}

pub fn capability_name(value: Capability) -> String {
    match value {
        Capability::SubscriptionQuota => "subscriptionQuota",
        Capability::QuotaResetTime => "quotaResetTime",
        Capability::ApiTokens => "apiTokens",
        Capability::ApiRequests => "apiRequests",
        Capability::ApiCostReported => "apiCostReported",
        Capability::ApiCostEstimated => "apiCostEstimated",
        Capability::Balance => "balance",
        Capability::Credits => "credits",
        Capability::ModelBreakdown => "modelBreakdown",
        Capability::ProjectBreakdown => "projectBreakdown",
        Capability::HistoryRemote => "historyRemote",
        Capability::HistoryLocal => "historyLocal",
        Capability::LocalSessions => "localSessions",
        Capability::MultiAccount => "multiAccount",
    }
    .into()
}

pub fn status_class(state: &ConnectionState) -> Option<String> {
    match state {
        ConnectionState::Connected | ConnectionState::Refreshing => Some("ok".into()),
        ConnectionState::RateLimited | ConnectionState::Cooldown => Some("rate_limited".into()),
        ConnectionState::AuthenticationRequired
        | ConnectionState::SessionExpired
        | ConnectionState::PermissionDenied
        | ConnectionState::InsufficientScope => Some("auth".into()),
        ConnectionState::Offline | ConnectionState::NetworkError => Some("network".into()),
        ConnectionState::NotConfigured
        | ConnectionState::Unavailable
        | ConnectionState::UnsupportedSource => Some("unavailable".into()),
        ConnectionState::ParseError => Some("schema_change".into()),
        ConnectionState::ApplicationNotRunning | ConnectionState::UnknownError => {
            Some("unknown".into())
        }
    }
}

pub fn map_error(e: &usage_core::ProviderError) -> ConnectionState {
    match e {
        usage_core::ProviderError::AuthenticationRequired => {
            ConnectionState::AuthenticationRequired
        }
        usage_core::ProviderError::PermissionDenied => ConnectionState::PermissionDenied,
        usage_core::ProviderError::InsufficientScope => ConnectionState::InsufficientScope,
        usage_core::ProviderError::RateLimited(_) => ConnectionState::RateLimited,
        usage_core::ProviderError::Network(_) => ConnectionState::NetworkError,
        usage_core::ProviderError::Parse(_) => ConnectionState::ParseError,
        usage_core::ProviderError::Unavailable(_) => ConnectionState::Unavailable,
    }
}

pub fn safe_error_code(error: &usage_core::ProviderError) -> String {
    match error {
        usage_core::ProviderError::AuthenticationRequired => "authentication_required".into(),
        usage_core::ProviderError::PermissionDenied => "permission_denied".into(),
        usage_core::ProviderError::InsufficientScope => "insufficient_scope".into(),
        usage_core::ProviderError::RateLimited(_) => "rate_limited".into(),
        usage_core::ProviderError::Network(_) => "network_error".into(),
        usage_core::ProviderError::Unavailable(_) => "unavailable".into(),
        usage_core::ProviderError::Parse(_) => "parse_error".into(),
    }
}

pub fn notify_state_label(state: &ConnectionState) -> &'static str {
    match state {
        ConnectionState::AuthenticationRequired => "Нужна авторизация",
        ConnectionState::SessionExpired => "Сессия истекла",
        ConnectionState::PermissionDenied => "Нет разрешения",
        ConnectionState::InsufficientScope => "Недостаточно прав",
        _ => "Подключение требует внимания",
    }
}

pub fn source_state(state: &usage_providers::strategy::SourceError) -> ConnectionState {
    match state {
        usage_providers::strategy::SourceError::AuthenticationRequired => {
            ConnectionState::AuthenticationRequired
        }
        usage_providers::strategy::SourceError::InsufficientScope => {
            ConnectionState::InsufficientScope
        }
        usage_providers::strategy::SourceError::PermissionDenied => {
            ConnectionState::PermissionDenied
        }
        usage_providers::strategy::SourceError::IdentityMismatch => {
            ConnectionState::AuthenticationRequired
        }
        usage_providers::strategy::SourceError::UnsafeSourceDisabled
        | usage_providers::strategy::SourceError::UserDisabled => {
            ConnectionState::UnsupportedSource
        }
        usage_providers::strategy::SourceError::RateLimited { .. }
        | usage_providers::strategy::SourceError::Cooldown => ConnectionState::RateLimited,
        usage_providers::strategy::SourceError::Network
        | usage_providers::strategy::SourceError::Cancelled => ConnectionState::NetworkError,
        usage_providers::strategy::SourceError::Parse { .. } => ConnectionState::ParseError,
        usage_providers::strategy::SourceError::Unavailable
        | usage_providers::strategy::SourceError::Policy => ConnectionState::Unavailable,
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateDto {
    pub root_hash: String,
    pub provider_id: String,
    pub product_id: String,
    pub root_hint: String,
    pub kind: String,
    pub status: String,
    pub bound_account_id: Option<String>,
}
