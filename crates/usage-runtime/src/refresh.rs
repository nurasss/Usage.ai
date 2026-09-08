use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, Semaphore};
use usage_core::{refresh_policy, Account, ConnectionState};
use usage_host::{policy, Hosts, OnlineState};
use usage_providers::strategy::{
    Availability, FallbackDecision, FetchContext, FetchStrategy, SourceError,
};
use usage_storage::Storage;
use uuid::Uuid;

use crate::registry::{Registry, StrategyKind};
use crate::snapshot::stale_view_from_lkg;

type InflightKey = (u64, ScopeKey);
type InflightMap = HashMap<InflightKey, watch::Sender<Option<RefreshOutcome>>>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopeKey {
    pub account_id: Uuid,
    pub provider_id: String,
    pub product_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Manual,
    Scheduled,
    Wake,
    Tray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptStatus {
    Ok,
    Error,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct AttemptRecord {
    pub source: String,
    pub status: AttemptStatus,
    pub safe_code: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RefreshOutcome {
    pub scope: ScopeKey,
    pub snapshot: Option<usage_core::Snapshot>,
    pub stale: bool,
    pub error: Option<SourceError>,
    pub attempts: Vec<AttemptRecord>,
    pub warnings: Vec<String>,
    pub costs_imported: usize,
    pub usage_imported: usize,
    pub selected_source: Option<String>,
    pub schema_fingerprint: Option<String>,
}

/// Per-scope runtime memory. Survives across refreshes inside the
/// process; rate-limit cooldowns additionally persist in storage.
#[derive(Debug, Clone, Default)]
pub struct RefreshState {
    pub last_attempt: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub last_observed: Option<DateTime<Utc>>,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub error_code: Option<String>,
    pub connector_version: String,
    pub parser_version: String,
    pub last_state: Option<ConnectionState>,
    pub quota_windows: HashMap<String, (Option<String>, Option<f64>)>,
    pub quota_observations: HashMap<String, Vec<(DateTime<Utc>, f64)>>,
    pub attempt_count: u32,
    pub identity_note: Option<String>,
}

pub struct RefreshRequest {
    pub scope: ScopeKey,
    pub account: Account,
    pub custom_root: Option<PathBuf>,
    pub secret: Option<Vec<u8>>,
    pub trigger: Trigger,
}

pub struct Coordinator {
    hosts: Arc<Hosts>,
    storage: Arc<Mutex<Storage>>,
    registry: Registry,
    states: Mutex<HashMap<ScopeKey, RefreshState>>,
    inflight: Mutex<InflightMap>,
    semaphore: Arc<Semaphore>,
    generation: Mutex<u64>,
}

impl Coordinator {
    pub fn new(hosts: Arc<Hosts>, storage: Arc<Mutex<Storage>>) -> Self {
        let coordinator = Self {
            hosts,
            storage,
            registry: Registry::production(),
            states: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            semaphore: Arc::new(Semaphore::new(policy::REFRESH_PARALLELISM)),
            generation: Mutex::new(0),
        };
        coordinator.restore_persistent_cooldowns();
        coordinator
    }

    fn restore_persistent_cooldowns(&self) {
        let rows = self
            .storage
            .lock()
            .ok()
            .and_then(|s| s.load_cooldowns().ok())
            .unwrap_or_default();
        if rows.is_empty() {
            return;
        }
        let mut states = self.states.lock().unwrap_or_else(|e| e.into_inner());
        for (account_id, source, until_at) in rows {
            let (Ok(account), Ok(until)) = (
                account_id.parse::<Uuid>(),
                until_at.parse::<DateTime<Utc>>(),
            ) else {
                continue;
            };
            // A persisted source cooldown applies to every matching scope
            // of the account until a success clears it.
            for (scope, state) in states.iter_mut() {
                if scope.account_id == account && scope_source_id(scope) == source {
                    state.cooldown_until = Some(until);
                    state.error_code = Some("rate_limited".into());
                }
            }
        }
    }

    pub fn bump_generation(&self) -> u64 {
        self.generation
            .lock()
            .map(|mut gen| {
                *gen += 1;
                *gen
            })
            .unwrap_or(0)
    }

    pub fn states_snapshot(&self) -> HashMap<ScopeKey, RefreshState> {
        self.states
            .lock()
            .map(|states| states.clone())
            .unwrap_or_default()
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Refresh many scopes with bounded parallelism. One slow provider
    /// never serializes the whole chain; a newer generation supersedes
    /// older in-flight work through cancellation checks.
    pub async fn refresh_many(
        self: &Arc<Self>,
        requests: Vec<RefreshRequest>,
    ) -> Vec<RefreshOutcome> {
        let generation = self.bump_generation();
        let mut join_set = tokio::task::JoinSet::new();
        for request in requests {
            let coordinator = Arc::clone(self);
            let semaphore = coordinator.semaphore.clone();
            join_set.spawn(async move {
                let permit = semaphore.acquire_owned().await;
                match permit {
                    Ok(permit) => coordinator.refresh_scope(request, generation, permit).await,
                    Err(_) => {
                        let scope = request.scope.clone();
                        coordinator.stale_outcome(&scope, Some(SourceError::Cancelled), vec![])
                    }
                }
            });
        }
        let mut outcomes = vec![];
        while let Some(joined) = join_set.join_next().await {
            match joined {
                Ok(outcome) => outcomes.push(outcome),
                Err(_) => continue,
            }
        }
        outcomes
    }

    pub async fn refresh_scope(
        &self,
        request: RefreshRequest,
        generation: u64,
        _permit: tokio::sync::OwnedSemaphorePermit,
    ) -> RefreshOutcome {
        let key = (generation, request.scope.clone());
        // Coalescing: concurrent triggers for one scope share a single
        // execution instead of stampeding the source.
        let waiter = self
            .inflight
            .lock()
            .map(|mut inflight| {
                if let Some(sender) = inflight.get(&key) {
                    Some(sender.subscribe())
                } else {
                    let (sender, _) = watch::channel(None);
                    inflight.insert(key.clone(), sender);
                    None
                }
            })
            .unwrap_or(None);
        if let Some(mut waiter) = waiter {
            if waiter.wait_for(|outcome| outcome.is_some()).await.is_ok() {
                let outcome = waiter.borrow().clone();
                if let Some(outcome) = outcome {
                    return outcome;
                }
            }
            // Sharing failed; fall through and run directly.
        }
        let outcome = self.execute(request, generation).await;
        if let Ok(mut inflight) = self.inflight.lock() {
            if let Some(sender) = inflight.remove(&key) {
                let _ = sender.send(Some(outcome.clone()));
            }
        }
        outcome
    }

    async fn execute(&self, request: RefreshRequest, generation: u64) -> RefreshOutcome {
        let scope = request.scope.clone();
        self.hosts.logger.info(
            "refresh_scope_start",
            &[
                ("trigger", trigger_label(request.trigger)),
                ("product", &scope.product_id),
            ],
        );
        // Identity routing before any fetch: a mismatched credential
        // never attributes foreign data to this account (no auto-merge).
        if let crate::account_router::Routing::Mismatch { observed, .. } =
            self.check_identity(&request)
        {
            let attempt = self.record_attempt(
                &scope,
                "identity-router",
                AttemptStatus::Error,
                Some("identity_mismatch"),
            );
            if let Ok(mut states) = self.states.lock() {
                let state = states.entry(scope.clone()).or_default();
                state.last_attempt = Some(self.hosts.clock.now());
                state.error_code = Some("identity_mismatch".into());
                state.identity_note = Some(format!("identity_mismatch:observed={observed}"));
            }
            return self.stale_outcome(&scope, Some(SourceError::IdentityMismatch), vec![attempt]);
        }
        if let Some(until) = self.cooldown_remaining(&scope) {
            let _ = until;
            // Server cooldowns are never bypassed, including by manual triggers.
            return self.stale_outcome(&scope, Some(SourceError::Cooldown), vec![]);
        }
        self.touch_attempt(&scope);
        let kinds = crate::registry::strategies_for(&scope.provider_id, &scope.product_id);
        if kinds.is_empty() {
            return self.stale_outcome(&scope, Some(SourceError::Unavailable), vec![]);
        }
        let mut attempts = vec![];
        let mut warnings = vec![];
        let mut last_error: Option<SourceError> = None;
        for kind in kinds {
            if self.current_generation() != generation {
                return self.stale_outcome(&scope, Some(SourceError::Cancelled), attempts);
            }
            let strategy = build_strategy(kind, request.custom_root.clone());
            let source_id = strategy.source_id().0.to_string();
            if self.registry.is_disabled(strategy.kill_switch())
                && !strategy.kill_switch().is_empty()
            {
                attempts.push(self.record_attempt(
                    &scope,
                    &source_id,
                    AttemptStatus::Skipped,
                    Some("source_disabled"),
                ));
                continue;
            }
            if self.hosts.network.state() == OnlineState::Offline
                && matches!(
                    strategy.classification(),
                    usage_providers::strategy::SourceClassification::OfficialApi
                )
            {
                attempts.push(self.record_attempt(
                    &scope,
                    &source_id,
                    AttemptStatus::Skipped,
                    Some("offline"),
                ));
                last_error = Some(SourceError::Network);
                continue;
            }
            let ctx = FetchContext {
                account: &request.account,
                hosts: &self.hosts,
                timeout: policy::SOURCE_TIMEOUT,
                custom_root: request.custom_root.clone(),
                secret: request.secret.clone(),
            };
            match strategy.availability(&ctx).await {
                Availability::Ready => {}
                Availability::NotConfigured => {
                    attempts.push(self.record_attempt(
                        &scope,
                        &source_id,
                        AttemptStatus::Skipped,
                        Some("not_configured"),
                    ));
                    last_error = Some(SourceError::Unavailable);
                    continue;
                }
                Availability::Disabled => {
                    attempts.push(self.record_attempt(
                        &scope,
                        &source_id,
                        AttemptStatus::Skipped,
                        Some("source_disabled"),
                    ));
                    last_error = Some(SourceError::UserDisabled);
                    break;
                }
                Availability::Unsupported => {
                    attempts.push(self.record_attempt(
                        &scope,
                        &source_id,
                        AttemptStatus::Skipped,
                        Some("unsupported_source"),
                    ));
                    last_error = Some(SourceError::Unavailable);
                    continue;
                }
            }
            let started = self.hosts.clock.now();
            let result = tokio::time::timeout(policy::SOURCE_TIMEOUT, strategy.fetch(&ctx)).await;
            let finished = self.hosts.clock.now();
            let (payload, error) = match result {
                Ok(Ok(payload)) => (Some(payload), None),
                Ok(Err(error)) => (None, Some(error)),
                Err(_) => (None, Some(SourceError::Network)),
            };
            match (payload, error) {
                (Some(payload), _) => {
                    match self.commit_payload(&scope, &request, &source_id, payload) {
                        Ok(outcome) => {
                            attempts.push(self.record_attempt(
                                &scope,
                                &source_id,
                                AttemptStatus::Ok,
                                None,
                            ));
                            let mut outcome = outcome;
                            outcome.attempts = attempts;
                            return outcome;
                        }
                        Err(error) => {
                            attempts.push(self.record_attempt(
                                &scope,
                                &source_id,
                                AttemptStatus::Error,
                                Some(error.safe_code()),
                            ));
                            self.note_failure(&scope, &source_id, &error, started, finished);
                            warnings.push(error.safe_code().to_string());
                            last_error = Some(error);
                            break;
                        }
                    }
                }
                (None, Some(error)) => {
                    attempts.push(self.record_attempt(
                        &scope,
                        &source_id,
                        AttemptStatus::Error,
                        Some(error.safe_code()),
                    ));
                    self.note_failure(&scope, &source_id, &error, started, finished);
                    last_error = Some(error.clone());
                    if strategy.fallback_on(&error) == FallbackDecision::Stop {
                        break;
                    }
                }
                (None, None) => unreachable!(),
            }
        }
        let _ = warnings;
        self.stale_outcome(&scope, last_error, attempts)
    }

    fn commit_payload(
        &self,
        scope: &ScopeKey,
        request: &RefreshRequest,
        source_id: &str,
        payload: usage_providers::strategy::FetchPayload,
    ) -> Result<RefreshOutcome, SourceError> {
        let snapshot = payload
            .snapshot
            .ok_or(SourceError::Parse { schema: "snapshot" })?;
        for quota in &snapshot.quotas {
            quota
                .validate()
                .map_err(|_| SourceError::Parse { schema: "quota" })?;
        }
        let payload_json = serde_json::to_string(&snapshot)
            .map_err(|_| SourceError::Parse { schema: "snapshot" })?;
        let fetched_at = snapshot.fetched_at;
        let observed_at = snapshot.observed_at;
        let (usage_imported, costs_imported) = self
            .storage
            .lock()
            .map_err(|_| SourceError::Unavailable)?
            .commit_refresh_bundle(&usage_storage::RefreshBundle {
                account_id: request.account.id,
                product_id: &scope.product_id,
                snapshot_payload_json: &payload_json,
                observed_at,
                fetched_at,
                usage: &payload.usage,
                costs: &payload.costs,
            })
            .map(|report| (report.usage_accepted, report.costs_accepted))
            .map_err(|_| SourceError::Unavailable)?;
        self.on_success(scope, source_id, &snapshot, payload.schema_fingerprint);
        Ok(RefreshOutcome {
            scope: scope.clone(),
            snapshot: Some(snapshot),
            stale: false,
            error: None,
            attempts: vec![],
            warnings: payload.warnings,
            costs_imported,
            usage_imported,
            selected_source: Some(source_id.to_string()),
            schema_fingerprint: Some(payload.schema_fingerprint.to_string()),
        })
    }

    fn stale_outcome(
        &self,
        scope: &ScopeKey,
        error: Option<SourceError>,
        attempts: Vec<AttemptRecord>,
    ) -> RefreshOutcome {
        let snapshot = self.storage.lock().ok().and_then(|storage| {
            storage
                .last_known_good(scope.account_id, &scope.product_id)
                .ok()
                .flatten()
                .and_then(|stored| stale_view_from_lkg(&stored, self.hosts.clock.now()))
                .map(|view| view.snapshot)
        });
        let error_code = error.as_ref().map(|e| e.safe_code().to_string());
        if let Ok(mut states) = self.states.lock() {
            let state = states.entry(scope.clone()).or_default();
            state.last_attempt = Some(self.hosts.clock.now());
            state.error_code = error_code;
        }
        RefreshOutcome {
            scope: scope.clone(),
            snapshot,
            stale: true,
            error,
            attempts,
            warnings: vec![],
            costs_imported: 0,
            usage_imported: 0,
            selected_source: None,
            schema_fingerprint: None,
        }
    }

    /// Identity routing for one scope. API credentials yield an
    /// observed fingerprint; local file sources yield none. First-seen
    /// fingerprints persist as `Weak` — never `Verified`, never merged.
    fn check_identity(&self, request: &RefreshRequest) -> crate::account_router::Routing {
        use crate::account_router::{route, Routing};
        use usage_core::IdentityConfidence;

        let scope = &request.scope;
        let observed: Option<String> = if scope.product_id == "openai-api" {
            request
                .secret
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| usage_host::fingerprint("openai-api", s))
        } else {
            None
        };
        let (stored, confidence) = self
            .storage
            .lock()
            .ok()
            .and_then(|s| s.get_account(scope.account_id).ok())
            .flatten()
            .map(|m| (m.external_identity, m.identity_confidence))
            .unwrap_or((
                request.account.external_identity.clone(),
                IdentityConfidence::Unknown,
            ));
        let routing = route(stored.as_deref(), confidence, observed.as_deref());
        if matches!(
            routing,
            Routing::Attributed {
                confidence: IdentityConfidence::Weak
            }
        ) && stored.is_none()
        {
            if let Some(fp) = observed {
                if let Ok(mut storage) = self.storage.lock() {
                    let _ = storage.set_account_identity(
                        scope.account_id,
                        &fp,
                        IdentityConfidence::Weak,
                    );
                }
            }
        }
        routing
    }

    fn cooldown_remaining(&self, scope: &ScopeKey) -> Option<Duration> {
        self.states.lock().ok().and_then(|states| {
            states.get(scope)?.cooldown_until.and_then(|until| {
                let ms = (until - self.hosts.clock.now()).num_milliseconds();
                (ms > 0).then(|| Duration::from_millis(ms as u64))
            })
        })
    }

    fn touch_attempt(&self, scope: &ScopeKey) {
        if let Ok(mut states) = self.states.lock() {
            let state = states.entry(scope.clone()).or_default();
            state.last_attempt = Some(self.hosts.clock.now());
        }
    }

    fn on_success(
        &self,
        scope: &ScopeKey,
        source_id: &str,
        snapshot: &usage_core::Snapshot,
        schema_fingerprint: &'static str,
    ) {
        if let Ok(mut states) = self.states.lock() {
            let state = states.entry(scope.clone()).or_default();
            state.last_success = Some(self.hosts.clock.now());
            state.last_observed = snapshot.observed_at;
            state.cooldown_until = None;
            state.error_code = None;
            state.attempt_count = 0;
            state.connector_version = source_id.to_string();
            state.parser_version = schema_fingerprint.to_string();
            state.last_state = Some(snapshot.connection_state.clone());
        }
        if let Ok(mut storage) = self.storage.lock() {
            let _ = storage.clear_cooldown(scope.account_id, source_id);
        }
    }

    fn note_failure(
        &self,
        scope: &ScopeKey,
        source_id: &str,
        error: &SourceError,
        _started: DateTime<Utc>,
        _finished: DateTime<Utc>,
    ) {
        let delay = match error {
            SourceError::RateLimited { retry_after_secs } => Some(
                retry_after_secs
                    .map(Duration::from_secs)
                    .unwrap_or(policy::FALLBACK_COOLDOWN),
            ),
            SourceError::Network | SourceError::Unavailable | SourceError::Parse { .. } => {
                let attempt = self
                    .states
                    .lock()
                    .ok()
                    .and_then(|states| states.get(scope).map(|s| s.attempt_count))
                    .unwrap_or(0);
                refresh_policy::cooldown_for_attempt(None, attempt, None, false)
            }
            _ => None,
        };
        if let Ok(mut states) = self.states.lock() {
            let state = states.entry(scope.clone()).or_default();
            state.attempt_count = state.attempt_count.saturating_add(1);
            state.error_code = Some(error.safe_code().to_string());
            if let Some(delay) = delay {
                if let Ok(until) =
                    chrono::Duration::from_std(delay).map(|d| self.hosts.clock.now() + d)
                {
                    // Only server-driven rate limits persist across restarts;
                    // transient backoff stays in memory.
                    if matches!(error, SourceError::RateLimited { .. }) {
                        if let Ok(mut storage) = self.storage.lock() {
                            let _ = storage.set_cooldown(scope.account_id, source_id, until);
                        }
                    }
                    state.cooldown_until = Some(until);
                }
            }
        }
    }

    fn record_attempt(
        &self,
        scope: &ScopeKey,
        source: &str,
        status: AttemptStatus,
        safe_code: Option<&str>,
    ) -> AttemptRecord {
        let now = self.hosts.clock.now();
        let status_str = match status {
            AttemptStatus::Ok => "ok",
            AttemptStatus::Error => "error",
            AttemptStatus::Skipped => "skipped",
        };
        if let Ok(mut storage) = self.storage.lock() {
            let _ = storage.record_attempt(&usage_storage::AttemptLog {
                account_id: scope.account_id,
                product_id: &scope.product_id,
                source,
                status: status_str,
                safe_code,
                started_at: now,
                finished_at: now,
            });
        }
        AttemptRecord {
            source: source.to_string(),
            status,
            safe_code: safe_code.map(|s| s.to_string()),
            started_at: now,
            finished_at: now,
        }
    }

    fn current_generation(&self) -> u64 {
        self.generation.lock().map(|g| *g).unwrap_or(0)
    }
}

fn trigger_label(trigger: Trigger) -> &'static str {
    match trigger {
        Trigger::Manual => "manual",
        Trigger::Scheduled => "scheduled",
        Trigger::Wake => "wake",
        Trigger::Tray => "tray",
    }
}

fn scope_source_id(scope: &ScopeKey) -> String {
    // Persistent cooldowns are keyed per account+source. The scope's
    // product maps to its primary strategy source id.
    crate::registry::strategies_for(&scope.provider_id, &scope.product_id)
        .first()
        .map(strategy_source_name)
        .unwrap_or_default()
}

fn strategy_source_name(kind: &StrategyKind) -> String {
    match kind {
        StrategyKind::CodexJsonl => "codex-local-jsonl".into(),
        StrategyKind::ClaudeJsonl => "claude-local-jsonl".into(),
        StrategyKind::OpenAiCosts => "openai-api-costs".into(),
    }
}

fn build_strategy(kind: StrategyKind, custom_root: Option<PathBuf>) -> Box<dyn FetchStrategy> {
    match kind {
        StrategyKind::CodexJsonl => {
            Box::new(usage_providers::codex::CodexJsonlStrategy::new(custom_root))
        }
        StrategyKind::ClaudeJsonl => Box::new(usage_providers::claude::ClaudeJsonlStrategy::new(
            custom_root,
        )),
        StrategyKind::OpenAiCosts => {
            Box::new(usage_providers::openai_api::OpenAiCostsStrategy { base_url: None })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use usage_core::AccountLifecycle;
    use usage_host::{
        HttpJsonRequest, HttpJsonResponse, MemoryKeychain, NullLogger, ObservedNetwork,
        ScopedFiles, SystemClock,
    };
    use uuid::Uuid;

    struct ScriptHttp {
        respond: Mutex<ScriptBehavior>,
        calls: Mutex<usize>,
        delay_ms: u64,
    }

    #[derive(Clone)]
    enum ScriptBehavior {
        CostPage(serde_json::Value),
        RateLimited,
    }

    #[async_trait]
    impl usage_host::HttpHost for ScriptHttp {
        async fn get_json(
            &self,
            _req: HttpJsonRequest<'_>,
        ) -> Result<HttpJsonResponse, usage_host::HostError> {
            *self.calls.lock().unwrap() += 1;
            if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }
            match self.respond.lock().unwrap().clone() {
                ScriptBehavior::CostPage(body) => Ok(HttpJsonResponse {
                    status: 200,
                    retry_after_secs: None,
                    body: Some(body),
                }),
                ScriptBehavior::RateLimited => Err(usage_host::HostError::RateLimited(Some(60))),
            }
        }
    }

    fn test_hosts(http: Arc<ScriptHttp>) -> Arc<Hosts> {
        Arc::new(Hosts {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http,
            files: Arc::new(ScopedFiles),
            network: Arc::new(ObservedNetwork::default()),
        })
    }

    fn script_http(behavior: ScriptBehavior) -> Arc<ScriptHttp> {
        Arc::new(ScriptHttp {
            respond: Mutex::new(behavior),
            calls: Mutex::new(0),
            delay_ms: 0,
        })
    }

    fn cost_page() -> serde_json::Value {
        serde_json::json!({
            "object": "page",
            "has_more": false,
            "data": [{
                "start_time": 1_725_000_000,
                "end_time": 1_725_086_400,
                "results": [
                    {"amount": {"value": 4.82, "currency": "usd"}, "line_item": "gpt-4o"}
                ]
            }]
        })
    }

    fn account(provider: &str, label: &str) -> Account {
        Account {
            id: Uuid::new_v4(),
            provider_id: provider.into(),
            external_identity: None,
            label: label.into(),
            connection_ref: None,
            lifecycle: AccountLifecycle::Active,
        }
    }

    fn codex_fixture_sessions() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions").join("s1");
        std::fs::create_dir_all(&sessions).unwrap();
        let line = include_str!("../../usage-providers/fixtures/codex-token-count.jsonl").trim();
        std::fs::write(sessions.join("rollout-1.jsonl"), format!("{line}\n")).unwrap();
        dir
    }

    #[tokio::test]
    async fn e2e_codex_refresh_then_failure_serves_last_known_good() {
        let dir = codex_fixture_sessions();
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = script_http(ScriptBehavior::CostPage(cost_page()));
        let coordinator = Arc::new(Coordinator::new(test_hosts(http), storage.clone()));
        let acc = account("openai", "T");
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "codex".into(),
        };
        // Persist the account row: snapshots carry a foreign key.
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let request = || RefreshRequest {
            scope: scope.clone(),
            account: acc.clone(),
            custom_root: Some(dir.path().join("sessions")),
            secret: None,
            trigger: Trigger::Manual,
        };
        let first = coordinator.refresh_many(vec![request()]).await;
        assert_eq!(first.len(), 1);
        assert!(!first[0].stale);
        assert!(first[0].snapshot.is_some());
        assert_eq!(first[0].snapshot.as_ref().unwrap().quotas.len(), 1);
        assert!(storage
            .lock()
            .unwrap()
            .last_known_good(acc.id, "codex")
            .unwrap()
            .is_some());
        // Source disappears: the good snapshot survives as stale + error.
        std::fs::remove_dir_all(dir.path().join("sessions")).unwrap();
        let second = coordinator
            .refresh_many(vec![RefreshRequest {
                trigger: Trigger::Wake,
                ..request()
            }])
            .await;
        assert!(second[0].stale);
        assert!(second[0].snapshot.is_some());
        assert!(second[0].error.is_some());
    }

    #[tokio::test]
    async fn concurrent_triggers_coalesce_into_one_execution() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 300,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };
        let generation = coordinator.bump_generation();
        let make = || RefreshRequest {
            scope: scope.clone(),
            account: acc.clone(),
            custom_root: None,
            secret: Some(b"sk-test".to_vec()),
            trigger: Trigger::Tray,
        };
        // First trigger starts and parks inside the fetch; the second
        // trigger arrives while it is still in flight and must share
        // the execution instead of starting its own.
        let coordinator_a = coordinator.clone();
        let permit_a = coordinator.semaphore.clone().acquire_owned().await.unwrap();
        let request_a = make();
        let first = tokio::spawn(async move {
            coordinator_a
                .refresh_scope(request_a, generation, permit_a)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let coordinator_b = coordinator.clone();
        let permit_b = coordinator.semaphore.clone().acquire_owned().await.unwrap();
        let request_b = make();
        let second = tokio::spawn(async move {
            coordinator_b
                .refresh_scope(request_b, generation, permit_b)
                .await
        });
        let (first, second) = tokio::join!(first, second);
        let (first, second) = (first.unwrap(), second.unwrap());
        assert!(first.snapshot.is_some());
        assert!(second.snapshot.is_some());
        // One shared execution: a single logical fetch set.
        assert_eq!(*http.calls.lock().unwrap(), 1);
        assert_eq!(first.costs_imported, 1);
    }

    #[tokio::test]
    async fn rate_limit_cooldown_blocks_even_manual_refresh() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = script_http(ScriptBehavior::RateLimited);
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };
        let request = |trigger| RefreshRequest {
            scope: scope.clone(),
            account: acc.clone(),
            custom_root: None,
            secret: Some(b"sk-test".to_vec()),
            trigger,
        };
        let first = coordinator
            .refresh_many(vec![request(Trigger::Scheduled)])
            .await;
        assert!(matches!(
            first[0].error,
            Some(SourceError::RateLimited { .. })
        ));
        assert!(!storage.lock().unwrap().load_cooldowns().unwrap().is_empty());
        // Manual refresh must not bypass the server cooldown: no new fetch.
        let second = coordinator
            .refresh_many(vec![request(Trigger::Manual)])
            .await;
        assert!(matches!(second[0].error, Some(SourceError::Cooldown)));
        assert_eq!(*http.calls.lock().unwrap(), 1);
    }

    #[test]
    fn trigger_labels_cover_all_variants() {
        assert_eq!(trigger_label(Trigger::Manual), "manual");
        assert_eq!(trigger_label(Trigger::Scheduled), "scheduled");
        assert_eq!(trigger_label(Trigger::Wake), "wake");
        assert_eq!(trigger_label(Trigger::Tray), "tray");
    }

    fn managed_api_account(
        id: uuid::Uuid,
        identity: Option<String>,
        confidence: usage_core::IdentityConfidence,
    ) -> usage_storage::ManagedAccount {
        usage_storage::ManagedAccount {
            id,
            provider_id: "openai".into(),
            product_id: Some("openai-api".into()),
            external_identity: identity,
            label: "API".into(),
            connection_ref: None,
            lifecycle: AccountLifecycle::Active,
            enabled: true,
            custom_path: None,
            identity_confidence: confidence,
        }
    }

    fn api_request(
        scope: &ScopeKey,
        account: &Account,
        secret: &[u8],
        trigger: Trigger,
    ) -> RefreshRequest {
        RefreshRequest {
            scope: scope.clone(),
            account: account.clone(),
            custom_root: None,
            secret: Some(secret.to_vec()),
            trigger,
        }
    }

    #[tokio::test]
    async fn identity_mismatch_blocks_attribution_before_any_fetch() {
        use usage_core::IdentityConfidence;

        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = script_http(ScriptBehavior::CostPage(cost_page()));
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let id = Uuid::new_v4();
        let original_fp = usage_host::fingerprint("openai-api", b"sk-original");
        storage
            .lock()
            .unwrap()
            .create_managed_account(&managed_api_account(
                id,
                Some(original_fp),
                IdentityConfidence::Weak,
            ))
            .unwrap();
        let acc = Account {
            id,
            provider_id: "openai".into(),
            external_identity: None,
            label: "API".into(),
            connection_ref: None,
            lifecycle: AccountLifecycle::Active,
        };
        let scope = ScopeKey {
            account_id: id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };
        // A rotated/replaced key must not attribute foreign billing
        // rows to this account: no fetch, no commit, explicit error.
        let outcomes = coordinator
            .refresh_many(vec![api_request(
                &scope,
                &acc,
                b"sk-rotated",
                Trigger::Scheduled,
            )])
            .await;
        assert!(matches!(
            outcomes[0].error,
            Some(SourceError::IdentityMismatch)
        ));
        assert!(outcomes[0].snapshot.is_none());
        assert_eq!(outcomes[0].costs_imported, 0);
        assert_eq!(*http.calls.lock().unwrap(), 0);
        let states = coordinator.states_snapshot();
        assert!(states[&scope]
            .identity_note
            .as_deref()
            .is_some_and(|note| note.starts_with("identity_mismatch:observed=")));
    }

    #[tokio::test]
    async fn first_seen_identity_persists_as_weak() {
        use usage_core::IdentityConfidence;

        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = script_http(ScriptBehavior::CostPage(cost_page()));
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let id = Uuid::new_v4();
        storage
            .lock()
            .unwrap()
            .create_managed_account(&managed_api_account(id, None, IdentityConfidence::Unknown))
            .unwrap();
        let acc = Account {
            id,
            provider_id: "openai".into(),
            external_identity: None,
            label: "API".into(),
            connection_ref: None,
            lifecycle: AccountLifecycle::Active,
        };
        let scope = ScopeKey {
            account_id: id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };
        let outcomes = coordinator
            .refresh_many(vec![api_request(&scope, &acc, b"sk-new", Trigger::Manual)])
            .await;
        assert!(outcomes[0].snapshot.is_some());
        let stored = storage.lock().unwrap().get_account(id).unwrap().unwrap();
        assert_eq!(
            stored.external_identity.as_deref(),
            Some(usage_host::fingerprint("openai-api", b"sk-new").as_str())
        );
        assert_eq!(stored.identity_confidence, IdentityConfidence::Weak);
    }
}
