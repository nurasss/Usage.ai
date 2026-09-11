use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, MetricUnit, ProviderError, Quota,
    Snapshot, WindowKind,
};

pub const SCHEMA_FINGERPRINT: &str = "codex-appserver-rpc-v1";
pub const KILL_SWITCH: &str = "codex-app-server";
pub const SOURCE_ID: &str = "codex-app-server";
const RPC_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RPC_BYTES: usize = 512 * 1024;
/// Health-handshake budget per candidate executable. A cold node
/// wrapper can take seconds on first launch; a dead install EOFs
/// fast. Hangs must never stall availability: the probe is bounded
/// and a negative result is never cached (see HEALTHY_EXECUTABLES).
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// Process-global POSITIVE health cache: executables that answered an
/// `initialize` handshake are trusted without re-probing for the rest
/// of the process. Negative results are deliberately NOT cached — a
/// cold or flaky first launch must recover on the next refresh, not
/// stay dark until restart. Keys are canonicalized paths so symlinked
/// shims (e.g. npm-global) hit one entry.
static HEALTHY_EXECUTABLES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<PathBuf>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

/// Static Codex CLI locations, tried in order. No PATH lookup, no
/// shell. Existence alone does NOT prove a working install — a stale
/// system shim can exist as a file yet never answer RPC (observed on
/// a root-owned /usr/local/bin/codex with a broken vendor payload).
/// The platform layer pre-filters these by existence into the process
/// allow-list; the strategy then serves the first candidate that
/// passes the `initialize` health handshake (`select_executable`).
/// A Policy denial is try-next; any other failure is try-next too —
/// the failure only becomes source-unavailable when NO candidate is
/// healthy, at which point the JSONL fallback (with banner) serves.
pub fn codex_executable_candidates(home: &std::path::Path) -> Vec<PathBuf> {
    [
        PathBuf::from("/usr/local/bin/codex"),
        PathBuf::from("/opt/homebrew/bin/codex"),
        home.join(".npm-global/bin/codex"),
        home.join(".local/bin/codex"),
    ]
    .into_iter()
    .collect()
}

/// Credential metadata ONLY: auth mode, API-key presence, OAuth token
/// presence flag and the stable provider account id. Secret VALUES are
/// never deserialized into this struct, never logged, never stored —
/// only a fingerprint of `account_id` leaves the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthMetadata {
    pub auth_mode: Option<String>,
    pub has_api_key: bool,
    pub has_oauth_tokens: bool,
    pub account_id_fingerprint: Option<String>,
}

pub fn parse_auth_metadata(bytes: &[u8]) -> Result<AuthMetadata, ProviderError> {
    #[derive(serde::Deserialize)]
    struct Wire {
        auth_mode: Option<String>,
        #[serde(rename = "OPENAI_API_KEY")]
        openai_api_key: Option<String>,
        tokens: Option<WireTokens>,
    }
    #[derive(serde::Deserialize)]
    struct WireTokens {
        account_id: Option<String>,
    }
    let wire: Wire = serde_json::from_slice(bytes)
        .map_err(|_| ProviderError::Parse("codex_auth_schema".into()))?;
    // Only presence bits and non-secret identifiers survive: token
    // values are dropped here and never leave this function.
    let has_api_key = wire.openai_api_key.is_some_and(|k| !k.is_empty());
    let account_id_fingerprint = wire
        .tokens
        .as_ref()
        .and_then(|t| t.account_id.clone())
        .filter(|id| !id.is_empty())
        .map(|id| usage_host::fingerprint("codex-account", id.as_bytes()));
    Ok(AuthMetadata {
        auth_mode: wire.auth_mode,
        has_api_key,
        has_oauth_tokens: wire.tokens.is_some(),
        account_id_fingerprint,
    })
}

/// Observed login identity for routing: account type + plan only.
/// Email is present in the RPC response but is NEVER used as identity
/// (B-02 ban) and never stored.
pub fn identity_fingerprint(account_type: &str, plan_type: Option<&str>) -> String {
    usage_host::fingerprint(
        "codex-app-server",
        format!("{account_type}:{}", plan_type.unwrap_or("unknown")).as_bytes(),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpcQuotaWindow {
    pub used_percent: Decimal,
    pub resets_at: Option<DateTime<Utc>>,
    pub window_minutes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RpcRateBucket {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub plan_type: Option<String>,
    pub primary: Option<RpcQuotaWindow>,
    pub secondary: Option<RpcQuotaWindow>,
    pub has_unverified_credits: bool,
    pub reached: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RpcRateLimits {
    pub buckets: Vec<RpcRateBucket>,
    pub warnings: Vec<String>,
}

fn decimal_from_json(value: &serde_json::Value) -> Option<Decimal> {
    match value {
        serde_json::Value::Number(n) => Decimal::from_str_exact(&n.to_string()).ok(),
        serde_json::Value::String(s) => Decimal::from_str_exact(s.trim()).ok(),
        _ => None,
    }
}

fn parse_window(value: &serde_json::Value) -> Option<RpcQuotaWindow> {
    let used_percent = value.get("usedPercent").and_then(decimal_from_json)?;
    if used_percent < Decimal::ZERO || used_percent > Decimal::ONE_HUNDRED {
        return None;
    }
    Some(RpcQuotaWindow {
        used_percent,
        resets_at: value
            .get("resetsAt")
            .and_then(|v| v.as_i64())
            .and_then(|t| Utc.timestamp_opt(t, 0).single()),
        window_minutes: value.get("windowDurationMins").and_then(|v| v.as_u64()),
    })
}

fn opt_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn parse_bucket(value: &serde_json::Value) -> Option<RpcRateBucket> {
    if !value.is_object() {
        return None;
    }
    let has_unverified_credits = value
        .get("credits")
        .and_then(|c| c.get("hasCredits"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut reached = vec![];
    if let Some(marker) = value.get("rateLimitReachedType").and_then(|v| v.as_str()) {
        reached.push(marker.to_string());
    }
    Some(RpcRateBucket {
        limit_id: opt_string(value, "limitId"),
        limit_name: opt_string(value, "limitName"),
        plan_type: opt_string(value, "planType"),
        primary: value.get("primary").and_then(parse_window),
        secondary: value.get("secondary").and_then(parse_window),
        has_unverified_credits,
        reached,
    })
}

/// Pure parser for `account/rateLimits/read` results. Unknown or
/// malformed buckets are skipped with warnings; a response without
/// any usable bucket is a schema error, never an empty bill.
pub fn parse_ratelimits_response(
    value: &serde_json::Value,
) -> Result<RpcRateLimits, ProviderError> {
    let mut out = RpcRateLimits::default();
    let mut buckets: Vec<&serde_json::Value> = vec![];
    if let Some(primary) = value.get("rateLimits") {
        buckets.push(primary);
    }
    if let Some(by_id) = value.get("rateLimitsByLimitId").and_then(|v| v.as_object()) {
        let mut ids: Vec<_> = by_id.keys().collect();
        ids.sort();
        for id in ids {
            buckets.push(&by_id[id]);
        }
    }
    if buckets.is_empty() {
        return Err(ProviderError::Parse("codex_appserver_schema".into()));
    }
    for (idx, bucket) in buckets.iter().enumerate() {
        match parse_bucket(bucket) {
            Some(mut parsed) => {
                // A bucket without any window carries no quota signal.
                if parsed.primary.is_none() && parsed.secondary.is_none() {
                    out.warnings.push(format!("bucket_{idx}_no_windows"));
                    continue;
                }
                if parsed.has_unverified_credits {
                    out.warnings
                        .push("credits_present_unverified_units".to_string());
                }
                for marker in parsed.reached.drain(..) {
                    out.warnings.push(format!("rate_limit_reached:{marker}"));
                }
                out.buckets.push(parsed);
            }
            None => out.warnings.push(format!("bucket_{idx}_skipped")),
        }
    }
    if out.buckets.is_empty() {
        return Err(ProviderError::Parse("codex_appserver_schema".into()));
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEvidence {
    pub account_type: String,
    pub plan_type: Option<String>,
    pub requires_auth: bool,
}

/// Pure parser for `account/read` results. `refreshToken` must always
/// be false upstream; this parser only classifies what came back.
/// A missing account with `requiresOpenaiAuth` is an expired/missing
/// credential, never an empty identity.
pub fn parse_account_response(value: &serde_json::Value) -> Result<AccountEvidence, ProviderError> {
    let requires_auth = value
        .get("requiresOpenaiAuth")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    match value.get("account") {
        None | Some(serde_json::Value::Null) => {
            if requires_auth {
                Err(ProviderError::AuthenticationRequired)
            } else {
                Err(ProviderError::Parse("codex_appserver_schema".into()))
            }
        }
        Some(account) => {
            let account_type = account
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            Ok(AccountEvidence {
                account_type,
                plan_type: account
                    .get("planType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                requires_auth,
            })
        }
    }
}

fn window_to_quota(
    pool: &str,
    name: &str,
    window: &RpcQuotaWindow,
    limit_id: &Option<String>,
    observed: DateTime<Utc>,
) -> Quota {
    Quota {
        pool_id: format!("{}-{pool}", limit_id.as_deref().unwrap_or("codex")),
        window_id: window.resets_at.map(|r| r.timestamp().to_string()),
        name: name.into(),
        used: None,
        limit: None,
        used_percent: Some(window.used_percent),
        remaining_percent: Some(Decimal::ONE_HUNDRED - window.used_percent),
        unit: MetricUnit::Percent,
        window_start: window
            .resets_at
            .zip(window.window_minutes)
            .map(|(r, m)| r - chrono::Duration::minutes(m as i64)),
        resets_at: window.resets_at,
        // Provider-owned RPC with explicit duration+reset: a fixed
        // window by construction (unlike derived JSONL snapshots,
        // which stay Unknown).
        window_kind: WindowKind::Fixed,
        source: format!("codex-app-server@{}", observed.timestamp()),
    }
}

/// JSON-RPC transport over an owned stdio child. Injected so tests
/// run against a scripted stub without spawning anything.
#[async_trait::async_trait]
pub trait AppServerTransport: Send + Sync {
    async fn transact(
        &self,
        requests: Vec<serde_json::Value>,
        cancel: &usage_host::CancellationToken,
    ) -> Result<Vec<serde_json::Value>, ProviderError>;
}

pub fn rpc_request(id: u64, method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"id": id, "method": method, "params": params})
}

fn match_responses(
    requests: &[serde_json::Value],
    lines: &[String],
) -> Result<Vec<serde_json::Value>, ProviderError> {
    use std::collections::HashMap;
    let mut by_id: HashMap<u64, serde_json::Value> = HashMap::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(trimmed)
            .map_err(|_| ProviderError::Parse("codex_appserver_rpc".into()))?;
        if value.get("method").is_some() {
            continue; // server notification, not our response
        }
        let Some(id) = value.get("id").and_then(|v| v.as_u64()) else {
            continue;
        };
        if let Some(error) = value.get("error") {
            return Err(map_rpc_error(error));
        }
        by_id.insert(
            id,
            value
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
    }
    requests
        .iter()
        .map(|request| {
            let id = request.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            by_id
                .remove(&id)
                .ok_or_else(|| ProviderError::Parse("codex_appserver_rpc".into()))
        })
        .collect()
}

fn map_rpc_error(error: &serde_json::Value) -> ProviderError {
    let message = error
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if message.contains("unauthorized")
        || message.contains("unauthenticated")
        || message.contains("token expired")
        || message.contains("auth required")
        || message.contains("login")
    {
        ProviderError::AuthenticationRequired
    } else if message.contains("forbidden") || message.contains("insufficient") {
        ProviderError::InsufficientScope
    } else if message.contains("rate") && message.contains("limit") {
        ProviderError::RateLimited(None)
    } else {
        ProviderError::Unavailable("codex_appserver_rpc_error".into())
    }
}

/// Quota session over one owned stdio child: initialize, account/read
/// with `refreshToken: false` (never refresh), account/rateLimits/read.
/// Exactly one physical process per logical fetch.
pub async fn run_quota_session<T: AppServerTransport + ?Sized>(
    transport: &T,
    cancel: &usage_host::CancellationToken,
) -> Result<(AccountEvidence, RpcRateLimits), ProviderError> {
    if cancel.is_cancelled() {
        return Err(ProviderError::Network("codex_appserver_cancelled".into()));
    }
    let requests = vec![
        rpc_request(
            1,
            "initialize",
            serde_json::json!({"clientInfo": {"name": "Usage.ai", "version": "1.3.0"}}),
        ),
        rpc_request(
            2,
            "account/read",
            serde_json::json!({"refreshToken": false}),
        ),
        rpc_request(3, "account/rateLimits/read", serde_json::json!({})),
    ];
    let results = transport.transact(requests, cancel).await?;
    if cancel.is_cancelled() {
        return Err(ProviderError::Network("codex_appserver_cancelled".into()));
    }
    let account = parse_account_response(&results[1])?;
    // An account object without identity (proven live: a login-less
    // CODEX_HOME reports `{}` with requiresOpenaiAuth=true) is not
    // an empty success — it is a missing login. Surfacing auth state
    // keeps per-profile binding honest: the snapshot never claims a
    // quota the profile does not have.
    if account.account_type == "unknown" {
        return Err(ProviderError::AuthenticationRequired);
    }
    let limits = parse_ratelimits_response(&results[2])?;
    Ok((account, limits))
}

fn quotas_from_buckets(
    account: &Account,
    evidence: &AccountEvidence,
    limits: &RpcRateLimits,
    observed_at: DateTime<Utc>,
    capabilities: BTreeSet<Capability>,
) -> Snapshot {
    let mut quotas = vec![];
    for (idx, bucket) in limits.buckets.iter().enumerate() {
        let label = bucket
            .limit_name
            .clone()
            .unwrap_or_else(|| format!("Quota {}", idx + 1));
        if let Some(window) = &bucket.primary {
            quotas.push(window_to_quota(
                "primary",
                &label,
                window,
                &bucket.limit_id,
                observed_at,
            ));
        }
        if let Some(window) = &bucket.secondary {
            quotas.push(window_to_quota(
                "secondary",
                &format!("{label} (secondary)"),
                window,
                &bucket.limit_id,
                observed_at,
            ));
        }
    }
    let fetched = Utc::now();
    let age = (fetched - observed_at).num_seconds().max(0) as u64;
    Snapshot {
        account_id: account.id,
        provider_id: "openai".into(),
        product_id: "codex".into(),
        plan_label: evidence.plan_type.clone(),
        capabilities,
        quotas,
        balances: vec![],
        observed_at: Some(observed_at),
        fetched_at: fetched,
        connection_state: ConnectionState::Connected,
        freshness: if age < 600 {
            Freshness::Fresh
        } else {
            Freshness::Stale(age)
        },
        coverage: Coverage::Complete,
    }
}

/// Codex App Server quota strategy over scoped hosts. Never refreshes
/// tokens, never runs inference, never writes foreign state. Disabled
/// via the `codex-app-server` kill switch without touching the product.
///
/// Anti-contamination: the server reflects its own home login, so this
/// strategy only serves default (non-custom) connections. Managed
/// custom profiles keep their JSONL observation source.
pub struct CodexAppServerStrategy {
    pub executables: Vec<PathBuf>,
    pub stub: Option<Arc<dyn AppServerTransport>>,
    /// Profile root this strategy serves. None = default login.
    /// Some(root) spawns the server with CODEX_HOME=root, so the
    /// server reads THAT profile's auth — never the default login.
    /// Proven live: an empty root reports an empty account (no type,
    /// no email), never the default login, so cross-profile fallback
    /// is structurally impossible (§11.5).
    pub custom_root: Option<PathBuf>,
}

impl CodexAppServerStrategy {
    pub fn new(executables: Vec<PathBuf>) -> Self {
        Self {
            executables,
            stub: None,
            custom_root: None,
        }
    }

    #[cfg(test)]
    fn with_stub(stub: Arc<dyn AppServerTransport>) -> Self {
        Self {
            executables: vec![],
            stub: Some(stub),
            custom_root: None,
        }
    }

    /// Scoped environment for the child: exactly one variable,
    /// default login untouched when None.
    fn profile_env(&self) -> Vec<(String, String)> {
        self.custom_root
            .as_ref()
            .map(|root| {
                vec![(
                    "CODEX_HOME".to_string(),
                    root.to_string_lossy().into_owned(),
                )]
            })
            .unwrap_or_default()
    }

    /// Bounded health handshake against one candidate: spawn, send a
    /// single `initialize`, require a matched response id. No account
    /// read, no rate-limits read, no state touched. False on ANY
    /// failure (spawn denial, EOF, timeout, malformed RPC) — callers
    /// treat false as try-next, never as an error to surface.
    pub async fn probe_candidate(
        process: &dyn usage_host::ProcessHost,
        executable: &Path,
        env: &[(String, String)],
        cancel: &usage_host::CancellationToken,
    ) -> bool {
        let transport = HostProcessTransport {
            process,
            executable: executable.to_path_buf(),
            timeout: PROBE_TIMEOUT,
            env: env.to_vec(),
        };
        let init = rpc_request(
            1,
            "initialize",
            serde_json::json!({"clientInfo": {"name": "Usage.ai", "version": "1.3.0"}}),
        );
        transport.transact(vec![init], cancel).await.is_ok()
    }

    fn healthy_cached(executable: &Path) -> bool {
        let key = executable
            .canonicalize()
            .unwrap_or_else(|_| executable.to_path_buf());
        HEALTHY_EXECUTABLES
            .lock()
            .map(|set| set.contains(&key))
            .unwrap_or(false)
    }

    fn mark_healthy(executable: &Path) {
        let key = executable
            .canonicalize()
            .unwrap_or_else(|_| executable.to_path_buf());
        if let Ok(mut set) = HEALTHY_EXECUTABLES.lock() {
            set.insert(key);
        }
    }

    /// First healthy executable in static order: cached positives win
    /// without spawning; otherwise each candidate gets one bounded
    /// handshake. Returns None only when no candidate answers — the
    /// coordinator then skips to the JSONL fallback (with banner).
    async fn select_executable(
        &self,
        process: &dyn usage_host::ProcessHost,
        cancel: &usage_host::CancellationToken,
    ) -> Option<PathBuf> {
        if let Some(hit) = self.executables.iter().find(|e| Self::healthy_cached(e)) {
            return Some(hit.clone());
        }
        for candidate in &self.executables {
            if cancel.is_cancelled() {
                return None;
            }
            if Self::probe_candidate(process, candidate, &self.profile_env(), cancel).await {
                Self::mark_healthy(candidate);
                return Some(candidate.clone());
            }
        }
        None
    }

    fn capabilities() -> BTreeSet<Capability> {
        [
            Capability::SubscriptionQuota,
            Capability::QuotaResetTime,
            Capability::ApiTokens,
            Capability::LocalSessions,
            Capability::HistoryLocal,
        ]
        .into_iter()
        .collect()
    }
}

#[async_trait::async_trait]
impl crate::strategy::FetchStrategy for CodexAppServerStrategy {
    fn source_id(&self) -> crate::strategy::SourceId {
        crate::strategy::SourceId(SOURCE_ID)
    }
    fn classification(&self) -> crate::strategy::SourceClassification {
        crate::strategy::SourceClassification::SupportedClientApi
    }
    fn kill_switch(&self) -> &'static str {
        KILL_SWITCH
    }
    async fn availability(
        &self,
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> crate::strategy::Availability {
        if ctx.cancel.is_cancelled() {
            return crate::strategy::Availability::Disabled;
        }
        if ctx.facade.process.is_none() {
            return crate::strategy::Availability::Unsupported;
        }
        if self.stub.is_some() {
            return crate::strategy::Availability::Ready;
        }
        if self.executables.is_empty() {
            return crate::strategy::Availability::NotConfigured;
        }
        let Some(process) = ctx.facade.process else {
            return crate::strategy::Availability::Unsupported;
        };
        // Health-gated: a present-but-dead install (stale system shim)
        // is NotConfigured, never Ready. The coordinator then skips to
        // the JSONL fallback instead of failing the refresh.
        match self.select_executable(process, &ctx.cancel).await {
            Some(_) => crate::strategy::Availability::Ready,
            None => crate::strategy::Availability::NotConfigured,
        }
    }
    async fn fetch(
        &self,
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> Result<crate::strategy::FetchPayload, crate::strategy::SourceError> {
        use crate::strategy::SourceError;
        if ctx.cancel.is_cancelled() {
            return Err(SourceError::Cancelled);
        }
        let Some(process) = ctx.facade.process else {
            return Err(SourceError::Policy);
        };
        let outcome = if let Some(stub) = &self.stub {
            run_quota_session(stub.as_ref(), &ctx.cancel).await
        } else {
            // Re-resolve here (not just in availability): a strategy
            // built before a binary healed must pick it up, and direct
            // fetch callers may skip the availability gate.
            let Some(executable) = self.select_executable(process, &ctx.cancel).await else {
                return Err(SourceError::Unavailable);
            };
            let transport = HostProcessTransport {
                process,
                executable,
                timeout: RPC_TIMEOUT,
                env: self.profile_env(),
            };
            run_quota_session(&transport, &ctx.cancel).await
        };
        let (evidence, limits) = outcome.map_err(|e| match e {
            ProviderError::AuthenticationRequired => SourceError::AuthenticationRequired,
            ProviderError::RateLimited(retry) => SourceError::RateLimited {
                retry_after_secs: retry,
            },
            ProviderError::Parse(_) => SourceError::Parse {
                schema: SCHEMA_FINGERPRINT,
            },
            ProviderError::Network(_) => SourceError::Network,
            ProviderError::Unavailable(_) => SourceError::Unavailable,
            _ => SourceError::Unavailable,
        })?;
        if ctx.cancel.is_cancelled() {
            return Err(SourceError::Cancelled);
        }
        let observed_at = ctx.facade.clock.now();
        let snapshot = quotas_from_buckets(
            ctx.account,
            &evidence,
            &limits,
            observed_at,
            Self::capabilities(),
        );
        let mut warnings = limits.warnings.clone();
        if evidence.requires_auth {
            warnings.push("requires_openai_auth".to_string());
        }
        Ok(crate::strategy::FetchPayload {
            snapshot: Some(snapshot),
            usage: vec![],
            costs: vec![],
            balances: vec![],
            warnings,
            schema_fingerprint: SCHEMA_FINGERPRINT,
            coverage: Coverage::Complete,
            observed_at: Some(observed_at),
            source_error: None,
            observed_identity: Some(identity_fingerprint(
                &evidence.account_type,
                evidence.plan_type.as_deref(),
            )),
        })
    }
}

/// Host-backed JSON-RPC transport: one owned stdio child per logical
/// fetch, killed on timeout/cancel/drop.
pub struct HostProcessTransport<'a> {
    pub process: &'a dyn usage_host::ProcessHost,
    pub executable: PathBuf,
    pub timeout: Duration,
    pub env: Vec<(String, String)>,
}

#[async_trait::async_trait]
impl AppServerTransport for HostProcessTransport<'_> {
    async fn transact(
        &self,
        requests: Vec<serde_json::Value>,
        cancel: &usage_host::CancellationToken,
    ) -> Result<Vec<serde_json::Value>, ProviderError> {
        use usage_host::InteractiveRequest;
        let mut child = self
            .process
            .spawn_interactive(InteractiveRequest {
                executable: &self.executable,
                args: &["app-server"],
                cancel: cancel.clone(),
                env: self.env.clone(),
            })
            .await
            .map_err(|_| ProviderError::Unavailable("codex_appserver_spawn".into()))?;
        let outcome = transact_session(&mut child, requests, self.timeout, cancel).await;
        let _ = child.kill().await;
        outcome
    }
}

async fn transact_session(
    child: &mut usage_host::InteractiveChild,
    requests: Vec<serde_json::Value>,
    timeout: Duration,
    cancel: &usage_host::CancellationToken,
) -> Result<Vec<serde_json::Value>, ProviderError> {
    for request in &requests {
        if cancel.is_cancelled() {
            return Err(ProviderError::Network("codex_appserver_cancelled".into()));
        }
        // The only values ever written are our own request envelopes:
        // no secret, no credential, no user data leaves the process.
        if !request_is_safe(request) {
            return Err(ProviderError::Unavailable("codex_appserver_policy".into()));
        }
        let line = serde_json::to_string(request)
            .map_err(|_| ProviderError::Parse("codex_appserver_rpc".into()))?;
        child
            .write_line(&line)
            .await
            .map_err(|_| ProviderError::Unavailable("codex_appserver_io".into()))?;
    }
    let wanted: Vec<u64> = requests
        .iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_u64()))
        .collect();
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut lines = vec![];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cancel.is_cancelled() {
            return Err(ProviderError::Network("codex_appserver_cancelled".into()));
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(ProviderError::Network("codex_appserver_timeout".into()));
        }
        let line = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ProviderError::Network("codex_appserver_cancelled".into())),
            result = tokio::time::timeout(remaining, child.read_line(MAX_RPC_BYTES)) => match result {
                Err(_) => return Err(ProviderError::Network("codex_appserver_timeout".into())),
                Ok(Err(_)) => return Err(ProviderError::Unavailable("codex_appserver_io".into())),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(line))) => line,
            },
        };
        // Track matched response ids from parsed JSON, never from
        // substring search (ids may legally appear inside payloads).
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if value.get("method").is_none() {
                if let Some(id) = value.get("id").and_then(|v| v.as_u64()) {
                    if wanted.contains(&id) {
                        seen.insert(id);
                    }
                }
            }
        }
        lines.push(line);
        if wanted.iter().all(|id| seen.contains(id)) {
            break;
        }
    }
    match_responses(&requests, &lines)
}

/// Outbound request allowlist: only our own fixed-shape envelopes.
fn request_is_safe(request: &serde_json::Value) -> bool {
    let method_ok = request
        .get("method")
        .and_then(|v| v.as_str())
        .is_some_and(|m| matches!(m, "initialize" | "account/read" | "account/rateLimits/read"));
    let params_ok = match request.get("method").and_then(|v| v.as_str()) {
        Some("initialize") => true,
        Some("account/read") => {
            request.get("params").and_then(|p| p.get("refreshToken"))
                == Some(&serde_json::Value::Bool(false))
        }
        Some("account/rateLimits/read") => request.get("params") == Some(&serde_json::json!({})),
        _ => false,
    };
    method_ok && params_ok && request.get("id").and_then(|v| v.as_u64()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use usage_host::ProcessHost;

    struct ScriptServer {
        responses: Mutex<HashMap<u64, serde_json::Value>>,
        errors: Mutex<HashMap<u64, serde_json::Value>>,
        calls: Mutex<usize>,
        seen_requests: Mutex<Vec<serde_json::Value>>,
        delay_ms: u64,
    }

    #[async_trait::async_trait]
    impl AppServerTransport for ScriptServer {
        async fn transact(
            &self,
            requests: Vec<serde_json::Value>,
            cancel: &usage_host::CancellationToken,
        ) -> Result<Vec<serde_json::Value>, ProviderError> {
            *self.calls.lock().unwrap() += 1;
            self.seen_requests.lock().unwrap().extend(requests.clone());
            if self.delay_ms > 0 {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(ProviderError::Network("codex_appserver_cancelled".into())),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)) => {}
                }
            }
            if cancel.is_cancelled() {
                return Err(ProviderError::Network("codex_appserver_cancelled".into()));
            }
            let responses = self.responses.lock().unwrap();
            let errors = self.errors.lock().unwrap();
            requests
                .iter()
                .map(|request| {
                    let id = request.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                    if let Some(error) = errors.get(&id) {
                        return Err(map_rpc_error(error));
                    }
                    responses
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| ProviderError::Parse("codex_appserver_rpc".into()))
                })
                .collect()
        }
    }

    fn script(responses: &[(u64, serde_json::Value)]) -> ScriptServer {
        ScriptServer {
            responses: Mutex::new(responses.iter().cloned().collect()),
            errors: Mutex::new(HashMap::new()),
            calls: Mutex::new(0),
            seen_requests: Mutex::new(vec![]),
            delay_ms: 0,
        }
    }

    fn account_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../fixtures/codex/appserver-account-plus.json"
        ))
        .unwrap()
    }

    fn limits_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../fixtures/codex/appserver-ratelimits.json")).unwrap()
    }

    fn ok_server() -> ScriptServer {
        script(&[
            (1, serde_json::json!({})),
            (2, account_fixture()),
            (3, limits_fixture()),
        ])
    }

    #[tokio::test]
    async fn full_session_maps_windows_and_identity() {
        let server = ok_server();
        let (evidence, limits) = run_quota_session(&server, &usage_host::CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(*server.calls.lock().unwrap(), 1);
        assert_eq!(evidence.account_type, "chatgpt");
        assert_eq!(evidence.plan_type.as_deref(), Some("plus"));
        // Primary bucket + codex bucket + extra bucket.
        assert_eq!(limits.buckets.len(), 3);
        let primary = limits.buckets[0].primary.as_ref().unwrap();
        assert_eq!(primary.used_percent, Decimal::new(27, 0));
        assert_eq!(primary.window_minutes, Some(300));
        assert!(primary.resets_at.is_some());
        let extra = limits
            .buckets
            .iter()
            .find(|b| b.limit_id.as_deref() == Some("codex-extra"))
            .unwrap();
        assert_eq!(
            extra.primary.as_ref().unwrap().used_percent,
            Decimal::new(5, 0)
        );
        // Identity never contains the email.
        let fp = identity_fingerprint(&evidence.account_type, evidence.plan_type.as_deref());
        let debug = format!("{evidence:?}:{fp}");
        assert!(!debug.contains("redacted@example.invalid"));
        assert!(!debug.contains('@'));
    }

    #[tokio::test]
    async fn only_safe_envelopes_leave_the_process() {
        let server = ok_server();
        let _ = run_quota_session(&server, &usage_host::CancellationToken::new()).await;
        let seen = server.seen_requests.lock().unwrap();
        assert_eq!(seen.len(), 3);
        let methods: Vec<_> = seen
            .iter()
            .filter_map(|r| r.get("method").and_then(|v| v.as_str()))
            .collect();
        // No inference-carrying method is ever sent.
        assert_eq!(
            methods,
            vec!["initialize", "account/read", "account/rateLimits/read"]
        );
        for request in seen.iter() {
            assert!(request_is_safe(request), "{request}");
        }
        let account_read = seen
            .iter()
            .find(|r| r.get("method") == Some(&serde_json::json!("account/read")))
            .unwrap();
        assert_eq!(
            account_read
                .get("params")
                .and_then(|p| p.get("refreshToken")),
            Some(&serde_json::Value::Bool(false))
        );
        let raw = serde_json::to_string(&*seen).unwrap().to_ascii_lowercase();
        // refreshToken:false is the only token-adjacent field allowed
        // out; secret values, credentials and prompts are banned.
        for forbidden in ["sk-", "bearer", "cookie", "password", "prompt", "secret"] {
            assert!(!raw.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn expired_credential_maps_to_auth_not_empty_identity() {
        let value = serde_json::json!({"account": null, "requiresOpenaiAuth": true});
        assert!(matches!(
            parse_account_response(&value),
            Err(ProviderError::AuthenticationRequired)
        ));
        let broken = serde_json::json!({"account": null, "requiresOpenaiAuth": false});
        assert!(matches!(
            parse_account_response(&broken),
            Err(ProviderError::Parse(_))
        ));
    }

    #[test]
    fn rpc_errors_classify_safely() {
        let auth = serde_json::json!({"message": "Unauthorized: login required"});
        assert!(matches!(
            map_rpc_error(&auth),
            ProviderError::AuthenticationRequired
        ));
        let rate = serde_json::json!({"message": "rate limit exceeded, retry later"});
        assert!(matches!(
            map_rpc_error(&rate),
            ProviderError::RateLimited(_)
        ));
        let other = serde_json::json!({"message": "boom", "code": -32603});
        assert!(matches!(
            map_rpc_error(&other),
            ProviderError::Unavailable(_)
        ));
    }

    #[test]
    fn malformed_and_empty_buckets_rejected() {
        assert!(parse_ratelimits_response(&serde_json::json!({})).is_err());
        assert!(parse_ratelimits_response(&serde_json::json!({
            "rateLimits": {"limitId": "x", "primary": null, "secondary": null}
        }))
        .is_err());
        assert!(parse_ratelimits_response(&serde_json::json!({
            "rateLimits": {"limitId": "x", "primary": {"usedPercent": 101, "resetsAt": null, "windowDurationMins": null}}
        }))
        .is_err());
    }

    #[test]
    fn partial_response_without_buckets_is_complete_single_source() {
        let value: serde_json::Value = serde_json::from_str(include_str!(
            "../fixtures/codex/appserver-ratelimits-partial.json"
        ))
        .unwrap();
        let parsed = parse_ratelimits_response(&value).unwrap();
        assert_eq!(parsed.buckets.len(), 1);
        assert!(parsed.warnings.is_empty());
        assert!(parsed.buckets[0].plan_type.is_none());
    }

    #[test]
    fn identity_is_stable_and_email_free() {
        let a = identity_fingerprint("chatgpt", Some("plus"));
        assert_eq!(a, identity_fingerprint("chatgpt", Some("plus")));
        assert_ne!(a, identity_fingerprint("chatgpt", Some("pro")));
        assert_ne!(a, identity_fingerprint("apiKey", Some("plus")));
        assert!(parse_auth_metadata(br#"{"auth_mode":"oauth","OPENAI_API_KEY":null,"tokens":{"id_token":"x","access_token":"y","refresh_token":"z","account_id":"acc-1"}}"#).unwrap().account_id_fingerprint.is_some());
    }

    #[test]
    fn auth_metadata_never_carries_secrets() {
        let meta = parse_auth_metadata(br#"{"auth_mode":"oauth","OPENAI_API_KEY":null,"tokens":{"id_token":"id","access_token":"at","refresh_token":"rt","account_id":"acc-9"}}"#).unwrap();
        assert_eq!(meta.auth_mode.as_deref(), Some("oauth"));
        assert!(!meta.has_api_key);
        assert!(meta.has_oauth_tokens);
        let serialized = format!("{meta:?}");
        for forbidden in ["id_token", "\"at\"", "\"rt\"", "acc-9"] {
            assert!(!serialized.contains(forbidden), "{forbidden}");
        }
        assert!(parse_auth_metadata(b"not json").is_err());
    }

    #[tokio::test]
    async fn cancelled_session_aborts_without_results() {
        let server = ok_server();
        let cancel = usage_host::CancellationToken::new();
        cancel.cancel();
        let result = run_quota_session(&server, &cancel).await;
        assert!(result.is_err());
        assert_eq!(*server.calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn silent_server_times_out_and_child_is_reaped() {
        // A server that never answers must surface a timeout, never a
        // hang. A tiny sleeping script stands in for the mute child:
        // it ignores argv, holds stdio open and says nothing.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mute-server");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let host = usage_host::AllowlistedProcess::with_allowed([script.clone()]);
        let transport = HostProcessTransport {
            process: &host,
            executable: script,
            timeout: std::time::Duration::from_millis(150),
            env: vec![],
        };
        let started = std::time::Instant::now();
        let result = transport
            .transact(
                vec![rpc_request(1, "initialize", serde_json::json!({}))],
                &usage_host::CancellationToken::new(),
            )
            .await;
        assert!(
            matches!(result, Err(ProviderError::Network(_))),
            "{result:?}"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(host.owned_count(), 0);
    }

    #[tokio::test]
    async fn oversized_response_body_is_rejected() {
        // The read bound truncates a single line beyond MAX_RPC_BYTES;
        // truncated JSON fails closed at match time and can never
        // become quota. Simulate exactly what read_line yields.
        let huge = "x".repeat(600 * 1024);
        let full = format!("{{\"id\":3,\"result\":{{\"note\":\"{huge}\"}}}}");
        let truncated = full[..256 * 1024].to_string();
        assert!(match_responses(
            &[rpc_request(
                3,
                "account/rateLimits/read",
                serde_json::json!({})
            )],
            &[truncated],
        )
        .is_err());
    }

    #[test]
    fn match_responses_ignores_notifications_and_requires_ids() {
        let requests = vec![
            rpc_request(2, "account/read", serde_json::json!({})),
            rpc_request(3, "account/rateLimits/read", serde_json::json!({})),
        ];
        let lines = vec![
            r#"{"method":"account/updated","params":{}}"#.to_string(),
            r#"{"id":2,"result":{"requiresOpenaiAuth":false,"account":null}}"#.to_string(),
        ];
        // Missing id 3 fails the whole match: no partial attribution.
        assert!(match_responses(&requests, &lines).is_err());
    }
}

#[cfg(test)]
mod credit_and_order_tests {
    use super::*;
    use crate::strategy::{FetchContext, FetchStrategy};
    use uuid::Uuid;

    #[test]
    fn absent_credits_stay_silent_present_credits_warn() {
        let no_credits = serde_json::json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": {"usedPercent": 10, "windowDurationMins": 300, "resetsAt": 1789032901},
                "credits": null
            }
        });
        let parsed = parse_ratelimits_response(&no_credits).unwrap();
        assert_eq!(parsed.buckets.len(), 1);
        assert!(parsed.warnings.is_empty());

        let with_credits = serde_json::json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": {"usedPercent": 10, "windowDurationMins": 300, "resetsAt": 1789032901},
                "credits": {"hasCredits": true, "unlimited": false, "balance": "42"}
            }
        });
        let parsed = parse_ratelimits_response(&with_credits).unwrap();
        // Units unverified: warning only, no numeric balance anywhere.
        assert!(parsed
            .warnings
            .contains(&"credits_present_unverified_units".to_string()));
        let rendered = format!("{:?}", parsed.buckets[0]);
        assert!(!rendered.contains("42"));
    }

    #[test]
    fn absent_plan_and_names_degrade_gracefully() {
        let value = serde_json::json!({
            "rateLimits": {
                "limitId": null,
                "limitName": null,
                "primary": {"usedPercent": 0, "windowDurationMins": null, "resetsAt": null},
                "planType": null
            }
        });
        let parsed = parse_ratelimits_response(&value).unwrap();
        assert_eq!(parsed.buckets.len(), 1);
        assert!(parsed.buckets[0].plan_type.is_none());
        assert!(parsed.buckets[0]
            .primary
            .as_ref()
            .unwrap()
            .resets_at
            .is_none());
    }

    #[test]
    fn executable_candidates_are_static_and_shell_free() {
        let home = std::path::Path::new("/Users/example");
        let candidates = codex_executable_candidates(home);
        assert!(!candidates.is_empty());
        for candidate in &candidates {
            assert!(candidate.is_absolute(), "{candidate:?}");
            let text = candidate.to_string_lossy();
            assert!(!text.contains("sh"), "{candidate:?}");
            assert!(candidate.file_name().is_some_and(|n| n == "codex"));
        }
        // Home-scoped entries stay under the given home.
        assert!(candidates.iter().any(|p| p.starts_with(home)));
    }

    #[test]
    fn kill_switch_identity_is_stable() {
        assert_eq!(KILL_SWITCH, "codex-app-server");
        assert_eq!(SOURCE_ID, "codex-app-server");
        assert_eq!(SCHEMA_FINGERPRINT, "codex-appserver-rpc-v1");
    }

    /// A dead install must never shadow a working one: probe rejects
    /// instant-exit binaries, accepts a responder that answers the
    /// `initialize` handshake, and the strategy serves the first
    /// HEALTHY candidate in static order. The responder is a temp-dir
    /// script (test-only scaffolding; production spawns allow-listed
    /// Codex CLIs, never scripts) because an echoing pipe like
    /// /bin/cat reflects our own request envelope, which the protocol
    /// correctly classifies as a notification, not a response.
    #[tokio::test]
    async fn selection_skips_dead_install() {
        use std::os::unix::fs::PermissionsExt;
        let dead = PathBuf::from("/usr/bin/false");
        assert!(dead.is_file(), "/usr/bin/false must exist");
        let script = std::env::temp_dir().join(format!(
            "usage-ai-probe-responder-{}.sh",
            std::process::id()
        ));
        std::fs::write(
            &script,
            "#!/bin/sh\nread _line\nprintf '%s\\n' '{\"id\":1,\"result\":{\"ok\":true}}'\n",
        )
        .expect("write responder script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("chmod responder script");
        let process = usage_host::AllowlistedProcess::with_allowed([dead.clone(), script.clone()]);
        let cancel = usage_host::CancellationToken::new();
        assert!(
            !CodexAppServerStrategy::probe_candidate(&process, &dead, &[], &cancel).await,
            "instant-exit binary is not a server"
        );
        assert!(
            CodexAppServerStrategy::probe_candidate(&process, &script, &[], &cancel).await,
            "handshake responder is healthy"
        );
        // Order matters: dead-first must still resolve to the responder.
        let strategy = CodexAppServerStrategy::new(vec![dead, script.clone()]);
        let picked = strategy.select_executable(&process, &cancel).await;
        assert_eq!(picked, Some(script.clone()));
        let _ = std::fs::remove_file(&script);
    }

    /// Per-profile isolation (§11.5/§14): two CODEX_HOMEs served by
    /// one shim binary observe only their own login. The responder
    /// varies its identity by $CODEX_HOME and records every input
    /// line into that same home; each fetch must bind its own
    /// identity and leave the sibling home untouched.
    #[tokio::test]
    async fn profile_a_fetch_cannot_observe_profile_b() {
        use crate::strategy::FetchStrategy;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        let base = tempfile::tempdir().unwrap();
        let root_a = base.path().join("root-a");
        let root_b = base.path().join("root-b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let limits: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/codex/appserver-ratelimits.json"))
                .unwrap();
        // JSON-RPC framing is one object per line: the fixture is
        // pretty-printed on disk, so compact it like a real server.
        // (Unparsable fragment lines are skipped by the matcher, so a
        // multi-line responder would starve id matching until timeout
        // instead of failing fast — framing is load-bearing here.)
        let limits_line = serde_json::to_string(&limits).unwrap();
        std::fs::write(root_a.join("limits.json"), &limits_line).unwrap();
        std::fs::write(root_b.join("limits.json"), &limits_line).unwrap();
        let script = base.path().join("fake-codex.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nrecord=\"$CODEX_HOME/record.txt\"\nplan=team\ncase \"$CODEX_HOME\" in *root-a*) plan=plus;; esac\nn=0\nwhile IFS= read -r line; do\n  n=$((n+1))\n  printf '%s\\n' \"$line\" >> \"$record\"\n  if [ \"$n\" -eq 1 ]; then printf '%s\\n' '{\"id\":1,\"result\":{}}'; fi\n  if [ \"$n\" -eq 2 ]; then printf '{\"id\":2,\"result\":{\"account\":{\"type\":\"chatgpt\",\"planType\":\"'; printf '%s' \"$plan\"; printf '%s\\n' '\"},\"requiresOpenaiAuth\":true}}'; fi\n  if [ \"$n\" -eq 3 ]; then printf '{\"id\":3,\"result\":%s}\\n' \"$(cat \"$CODEX_HOME/limits.json\")\"; exit 0; fi\ndone\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let hosts = usage_host::Hosts {
            clock: Arc::new(usage_host::SystemClock),
            logger: Arc::new(usage_host::NullLogger),
            keychain: Arc::new(usage_host::MemoryKeychain::default()),
            http: Arc::new(usage_host::ReqwestHttpHost::default()),
            files: Arc::new(usage_host::ScopedFiles),
            file_scope: Arc::new(usage_host::ScopedFiles),
            network: Arc::new(usage_host::ObservedNetwork::default()),
            process: Arc::new(usage_host::AllowlistedProcess::with_allowed([
                script.clone()
            ])),
            pty: Arc::new(usage_host::AllowlistedPty::default()),
        };
        let descriptor =
            crate::descriptor::find_descriptor("openai", "codex").expect("codex descriptor");
        // Drive both fetches through real spawns with per-root env.
        let mut identities = std::collections::HashMap::new();
        for (name, root) in [("a", &root_a), ("b", &root_b)] {
            let account = Account {
                id: uuid::Uuid::new_v4(),
                provider_id: "openai".into(),
                external_identity: None,
                label: "T".into(),
                connection_ref: None,
                lifecycle: usage_core::AccountLifecycle::Active,
            };
            let strategy = CodexAppServerStrategy {
                executables: vec![script.clone()],
                stub: None,
                custom_root: Some(root.clone()),
            };
            let ctx = FetchContext {
                account: &account,
                facade: hosts.facade(&usage_host::facade::FacadePolicy {
                    files: true,
                    http: false,
                    process: true,
                    pty: false,
                    allowed_hosts: &[],
                    keychain_service: None,
                }),
                descriptor,
                timeout: std::time::Duration::from_secs(10),
                local_root: None,
                secret: None,
                cancel: usage_host::CancellationToken::new(),
            };
            assert_eq!(
                strategy.availability(&ctx).await,
                crate::strategy::Availability::Ready
            );
            let payload = strategy.fetch(&ctx).await.expect("root fetch");
            identities.insert(name, payload.observed_identity.expect("identity binds"));
            // Each home recorded exactly its own traffic: the 3-line
            // session (plus one gate probe line on first contact —
            // the health cache skips re-probing). Separate files per
            // home mean no observation can cross (the identity asserts
            // below prove the binding too).
            let record = std::fs::read_to_string(root.join("record.txt")).expect("own record");
            for method in ["initialize", "account/read", "account/rateLimits/read"] {
                assert!(record.contains(method), "{name} record misses {method}");
            }
        }
        assert_ne!(
            identities["a"], identities["b"],
            "A and B bind different logins"
        );
        assert_eq!(
            identities["a"],
            identity_fingerprint("chatgpt", Some("plus"))
        );
        assert_eq!(
            identities["b"],
            identity_fingerprint("chatgpt", Some("team"))
        );
    }

    /// A login-less root reports an empty account, which is auth
    /// state — never an empty success (§13).
    #[tokio::test]
    async fn unknown_account_type_is_authentication_required() {
        struct Stub;
        #[async_trait::async_trait]
        impl AppServerTransport for Stub {
            async fn transact(
                &self,
                _requests: Vec<serde_json::Value>,
                _cancel: &usage_host::CancellationToken,
            ) -> Result<Vec<serde_json::Value>, ProviderError> {
                Ok(vec![
                    serde_json::json!({}),
                    serde_json::json!({"account": {}, "requiresOpenaiAuth": true}),
                    serde_json::json!({}),
                ])
            }
        }
        let outcome = run_quota_session(&Stub, &usage_host::CancellationToken::new()).await;
        assert!(matches!(
            outcome,
            Err(ProviderError::AuthenticationRequired)
        ));
    }
}

#[cfg(test)]
mod strategy_wiring_tests {
    use super::*;
    use crate::strategy::{Availability, FetchContext, FetchStrategy};
    use std::sync::Arc;
    use uuid::Uuid;

    fn hosts() -> usage_host::Hosts {
        usage_host::Hosts {
            clock: Arc::new(usage_host::SystemClock),
            logger: Arc::new(usage_host::NullLogger),
            keychain: Arc::new(usage_host::MemoryKeychain::default()),
            http: Arc::new(usage_host::ReqwestHttpHost::default()),
            files: Arc::new(usage_host::ScopedFiles),
            file_scope: Arc::new(usage_host::ScopedFiles),
            network: Arc::new(usage_host::ObservedNetwork::default()),
            process: Arc::new(usage_host::AllowlistedProcess::default()),
            pty: Arc::new(usage_host::AllowlistedPty::default()),
        }
    }

    fn ctx<'a>(
        hosts: &'a usage_host::Hosts,
        account: &'a Account,
        descriptor: &'static crate::descriptor::ProductDescriptor,
    ) -> FetchContext<'a> {
        FetchContext {
            account,
            facade: hosts.facade(&usage_host::facade::FacadePolicy {
                files: true,
                http: false,
                process: true,
                pty: false,
                allowed_hosts: &[],
                keychain_service: None,
            }),
            descriptor,
            timeout: Duration::from_secs(5),
            local_root: None,
            secret: None,
            cancel: usage_host::CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn stub_strategy_end_to_end_with_real_wiring() {
        struct Stub;
        #[async_trait::async_trait]
        impl AppServerTransport for Stub {
            async fn transact(
                &self,
                _requests: Vec<serde_json::Value>,
                _cancel: &usage_host::CancellationToken,
            ) -> Result<Vec<serde_json::Value>, ProviderError> {
                let account: serde_json::Value = serde_json::from_str(include_str!(
                    "../fixtures/codex/appserver-account-plus.json"
                ))
                .unwrap();
                let limits: serde_json::Value = serde_json::from_str(include_str!(
                    "../fixtures/codex/appserver-ratelimits.json"
                ))
                .unwrap();
                Ok(vec![serde_json::json!({}), account, limits])
            }
        }
        let hosts = hosts();
        let account = Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        let descriptor =
            crate::descriptor::find_descriptor("openai", "codex").expect("codex descriptor");
        let strategy = CodexAppServerStrategy::with_stub(Arc::new(Stub));
        let ctx = ctx(&hosts, &account, descriptor);
        assert_eq!(strategy.availability(&ctx).await, Availability::Ready);
        let payload = strategy.fetch(&ctx).await.unwrap();
        let snapshot = payload.snapshot.unwrap();
        // 2 buckets x primary/secondary + extra primary = 5 quotas.
        assert_eq!(snapshot.quotas.len(), 5);
        assert_eq!(snapshot.plan_label.as_deref(), Some("plus"));
        assert!(payload.observed_identity.is_some());
    }

    #[tokio::test]
    async fn no_process_grant_means_unsupported() {
        let hosts = hosts();
        let account = Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        let descriptor =
            crate::descriptor::find_descriptor("openai", "codex").expect("codex descriptor");
        let ctx = FetchContext {
            account: &account,
            facade: hosts.facade(&usage_host::facade::FacadePolicy {
                files: true,
                http: false,
                process: false,
                pty: false,
                allowed_hosts: &[],
                keychain_service: None,
            }),
            descriptor,
            timeout: Duration::from_secs(5),
            local_root: None,
            secret: None,
            cancel: usage_host::CancellationToken::new(),
        };
        let strategy = CodexAppServerStrategy::new(vec![]);
        assert_eq!(strategy.availability(&ctx).await, Availability::Unsupported);
        assert!(matches!(
            strategy.fetch(&ctx).await,
            Err(crate::strategy::SourceError::Policy)
        ));
    }
}

#[cfg(test)]
mod live_acceptance {
    use super::*;
    use crate::strategy::{Availability, FetchContext, FetchStrategy};
    use std::sync::Arc;

    /// Opt-in live probe against the real local Codex CLI. Never runs in
    /// CI: requires USAGE_AI_LIVE_CODEX=1 and a working user login. It
    /// performs read-only RPCs only (initialize, account/read with
    /// refreshToken=false, account/rateLimits/read) and asserts the
    /// verified shapes the redacted fixtures mirror.
    #[tokio::test]
    #[ignore = "requires a local Codex login; run manually with USAGE_AI_LIVE_CODEX=1"]
    async fn live_app_server_reports_verified_quota() {
        if std::env::var_os("USAGE_AI_LIVE_CODEX").is_none() {
            return;
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
        // All existing candidates, in static order: proves selection
        // skips a present-but-dead install and serves the healthy one.
        let executables: Vec<PathBuf> = codex_executable_candidates(&home)
            .into_iter()
            .filter(|p| p.is_file())
            .collect();
        assert!(
            !executables.is_empty(),
            "codex binary must exist for the live probe"
        );
        let hosts = usage_host::Hosts {
            clock: Arc::new(usage_host::SystemClock),
            logger: Arc::new(usage_host::NullLogger),
            keychain: Arc::new(usage_host::MemoryKeychain::default()),
            http: Arc::new(usage_host::ReqwestHttpHost::default()),
            files: Arc::new(usage_host::ScopedFiles),
            file_scope: Arc::new(usage_host::ScopedFiles),
            network: Arc::new(usage_host::ObservedNetwork::default()),
            process: Arc::new(usage_host::AllowlistedProcess::with_allowed(
                executables.clone(),
            )),
            pty: Arc::new(usage_host::AllowlistedPty::default()),
        };
        let account = Account {
            id: uuid::Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        let descriptor =
            crate::descriptor::find_descriptor("openai", "codex").expect("codex descriptor");
        let ctx = FetchContext {
            account: &account,
            facade: hosts.facade(&usage_host::facade::FacadePolicy {
                files: true,
                http: false,
                process: true,
                pty: false,
                allowed_hosts: &[],
                keychain_service: None,
            }),
            descriptor,
            timeout: Duration::from_secs(25),
            local_root: None,
            secret: None,
            cancel: usage_host::CancellationToken::new(),
        };
        let strategy = CodexAppServerStrategy::new(executables);
        assert_eq!(strategy.availability(&ctx).await, Availability::Ready);
        let payload = strategy.fetch(&ctx).await.expect("live quota fetch");
        let snapshot = payload.snapshot.expect("snapshot");
        assert!(!snapshot.quotas.is_empty(), "live quotas must be present");
        assert!(snapshot.plan_label.is_some());
        assert!(payload.observed_identity.is_some());
        // No email or secret material anywhere in the safe surface.
        let rendered = format!("{snapshot:?}");
        // No email or secret material anywhere in the safe surface.
        // The only legal '@' is our own quota source marker
        // `codex-app-server@<unix-ts>` (Quota.source, constructed
        // locally — never server text).
        let mut rest = rendered.as_str();
        while let Some(pos) = rest.find('@') {
            let before = &rest[..pos];
            let after = &rest[pos + 1..];
            assert!(
                before.ends_with("codex-app-server")
                    && after.chars().next().is_some_and(|c| c.is_ascii_digit()),
                "unexpected '@' in snapshot surface near: ...{}...",
                &rendered[pos.saturating_sub(24)..(pos + 12).min(rendered.len())]
            );
            rest = after;
        }
    }
}
