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

use crate::registry::{facade_policy_for, Registry, StrategyKind};
use crate::root_resolver::scoped_product_root;
use crate::snapshot::stale_view_from_lkg;

type InflightKey = ScopeKey;
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
    account_cancellations: Mutex<HashMap<Uuid, usage_host::CancellationToken>>,
    inflight: Mutex<InflightMap>,
    semaphore: Arc<Semaphore>,
    shutdown: usage_host::CancellationToken,
    #[cfg(test)]
    pub pre_commit_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Coordinator {
    pub fn new(hosts: Arc<Hosts>, storage: Arc<Mutex<Storage>>) -> Self {
        Self {
            hosts,
            storage,
            registry: Registry::production(),
            states: Mutex::new(HashMap::new()),
            account_cancellations: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            semaphore: Arc::new(Semaphore::new(policy::REFRESH_PARALLELISM)),
            shutdown: usage_host::CancellationToken::new(),
            #[cfg(test)]
            pre_commit_hook: Mutex::new(None),
        }
    }

    /// Permanent shutdown: in-flight transport and file operations
    /// observe this token and abort; no new work starts afterwards.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Cancel active work for one account. The token is retained while the
    /// account is disabled, so a late refresh trigger cannot restart the
    /// source before an explicit re-enable creates a fresh token.
    pub fn cancel_account(&self, account_id: Uuid) {
        if let Ok(mut cancellations) = self.account_cancellations.lock() {
            let token = cancellations
                .entry(account_id)
                .or_insert_with(usage_host::CancellationToken::new);
            token.cancel();
        }
    }

    /// Clear a prior account cancellation after the account is enabled again.
    pub fn reset_account_cancellation(&self, account_id: Uuid) {
        if let Ok(mut cancellations) = self.account_cancellations.lock() {
            cancellations.remove(&account_id);
        }
    }

    fn account_cancellation(&self, account_id: Uuid) -> usage_host::CancellationToken {
        self.account_cancellations
            .lock()
            .map(|mut cancellations| {
                cancellations
                    .entry(account_id)
                    .or_insert_with(usage_host::CancellationToken::new)
                    .clone()
            })
            .unwrap_or_else(|_| usage_host::CancellationToken::new())
    }

    /// Combined lifecycle token for history import and other runtime-owned
    /// work that is not itself a provider refresh.
    pub fn cancellation_token_for(&self, account_id: Uuid) -> usage_host::CancellationToken {
        usage_host::CancellationToken::linked([
            self.shutdown.clone(),
            self.account_cancellation(account_id),
        ])
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
    /// never serializes the whole chain. Triggers for one scope share a
    /// single in-flight execution no matter which trigger started it:
    /// scheduler, manual, tray and wake calls coalesce by scope, so a
    /// second `refresh_many` for the same scope awaits the running task
    /// instead of issuing a duplicate source fetch.
    pub async fn refresh_many(
        self: &Arc<Self>,
        requests: Vec<RefreshRequest>,
    ) -> Vec<RefreshOutcome> {
        let mut join_set = tokio::task::JoinSet::new();
        for request in requests {
            let coordinator = Arc::clone(self);
            join_set.spawn(async move { coordinator.refresh_scope(request).await });
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

    pub async fn refresh_scope(&self, request: RefreshRequest) -> RefreshOutcome {
        if self.shutdown.is_cancelled() {
            return self.stale_outcome(&request.scope, Some(SourceError::Cancelled), vec![]);
        }
        let key = request.scope.clone();
        let cancel = self.cancellation_token_for(key.account_id);
        // Scope-level coalescing is registered before semaphore admission.
        // The semaphore bounds physical source work; the inflight map bounds
        // how many source tasks exist in the first place.
        let registration = self
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
            .ok()
            .flatten();
        if let Some(mut waiter) = registration {
            if waiter.changed().await.is_ok() {
                if let Some(outcome) = waiter.borrow().clone() {
                    return outcome;
                }
            }
            // A leader must always publish a result. A closed channel means
            // the leader was cancelled or panicked; do not start a duplicate
            // source task after that failure.
            return self.stale_outcome(&key, Some(SourceError::Cancelled), vec![]);
        }

        struct InflightGuard<'a> {
            inflight: &'a Mutex<InflightMap>,
            key: &'a ScopeKey,
            outcome: Option<RefreshOutcome>,
        }
        impl<'a> Drop for InflightGuard<'a> {
            fn drop(&mut self) {
                if let Ok(mut inflight) = self.inflight.lock() {
                    if let Some(sender) = inflight.remove(self.key) {
                        let _ = sender.send(self.outcome.take());
                    }
                }
            }
        }
        let mut guard = InflightGuard {
            inflight: &self.inflight,
            key: &key,
            outcome: None,
        };

        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => self.stale_outcome(&key, Some(SourceError::Cancelled), vec![]),
            permit = self.semaphore.clone().acquire_owned() => match permit {
                Ok(_permit) => {
                    if cancel.is_cancelled() {
                        self.stale_outcome(&key, Some(SourceError::Cancelled), vec![])
                    } else {
                        self.execute(request, cancel.clone()).await
                    }
                }
                Err(_) => self.stale_outcome(&key, Some(SourceError::Cancelled), vec![]),
            },
        };
        guard.outcome = Some(outcome.clone());
        drop(guard);
        outcome
    }

    async fn execute(
        &self,
        request: RefreshRequest,
        cancel: usage_host::CancellationToken,
    ) -> RefreshOutcome {
        let scope = request.scope.clone();
        self.hosts.logger.info(
            "refresh_scope_start",
            &[
                ("trigger", trigger_label(request.trigger)),
                ("product", &scope.product_id),
            ],
        );
        if cancel.is_cancelled() {
            return self.stale_outcome(&scope, Some(SourceError::Cancelled), vec![]);
        }
        let is_disabled = {
            let account_id = scope.account_id;
            self.storage
                .lock()
                .ok()
                .and_then(|s| s.get_account(account_id).ok().flatten())
                .map(|acc| {
                    !acc.enabled || matches!(acc.lifecycle, usage_core::AccountLifecycle::Archived)
                })
                .unwrap_or(true)
        };
        if is_disabled {
            return self.stale_outcome(&scope, Some(SourceError::Cancelled), vec![]);
        }
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
                self.hosts.clock.now(),
            );
            if let Ok(mut states) = self.states.lock() {
                let state = states.entry(scope.clone()).or_default();
                state.last_attempt = Some(self.hosts.clock.now());
                state.error_code = Some("identity_mismatch".into());
                state.identity_note = Some(format!("identity_mismatch:observed={observed}"));
            }
            return self.stale_outcome(&scope, Some(SourceError::IdentityMismatch), vec![attempt]);
        }
        // Lazy cooldown restoration: a scope created after coordinator
        // startup still observes persisted provider cooldowns, so a
        // restart before expiry never re-hits a rate-limited source.
        self.restore_scope_cooldown(&scope);
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
            // Cooperative cancellation replaces generation polling:
            // shutdown, disabled sources and removed accounts surface
            // here; transport/file boundaries observe the same token.
            if cancel.is_cancelled() {
                return self.stale_outcome(&scope, Some(SourceError::Cancelled), attempts);
            }
            let Some(strategy) = build_strategy(&self.hosts, kind, request.custom_root.clone())
            else {
                // Structurally inapplicable here (e.g. the App Server
                // reflects its own home login, never a custom profile).
                continue;
            };
            let source_id = strategy.source_id().0.to_string();
            if self.registry.is_disabled(strategy.kill_switch())
                && !strategy.kill_switch().is_empty()
            {
                attempts.push(self.record_attempt(
                    &scope,
                    &source_id,
                    AttemptStatus::Skipped,
                    Some("source_disabled"),
                    self.hosts.clock.now(),
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
                    self.hosts.clock.now(),
                ));
                last_error = Some(SourceError::Network);
                continue;
            }
            let descriptor =
                usage_providers::descriptor::find_descriptor(&scope.provider_id, &scope.product_id)
                    .expect("registry covers refreshed scope");
            let facade = self
                .hosts
                .facade(&facade_policy_for(&scope.provider_id, &scope.product_id));
            let local_root = scoped_product_root(
                &self.hosts,
                descriptor,
                request.custom_root.as_deref(),
                &cancel,
            );
            let ctx = FetchContext {
                account: &request.account,
                facade,
                descriptor,
                timeout: policy::SOURCE_TIMEOUT,
                local_root,
                secret: request.secret.clone(),
                cancel: cancel.clone(),
            };
            match strategy.availability(&ctx).await {
                Availability::Ready => {}
                Availability::NotConfigured => {
                    attempts.push(self.record_attempt(
                        &scope,
                        &source_id,
                        AttemptStatus::Skipped,
                        Some("not_configured"),
                        self.hosts.clock.now(),
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
                        self.hosts.clock.now(),
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
                        self.hosts.clock.now(),
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
                            let partial_error = outcome.error.clone();
                            attempts.push(self.record_attempt(
                                &scope,
                                &source_id,
                                if partial_error.is_some() {
                                    AttemptStatus::Error
                                } else {
                                    AttemptStatus::Ok
                                },
                                partial_error.as_ref().map(SourceError::safe_code),
                                started,
                            ));
                            if let Some(error) = partial_error {
                                self.note_failure(&scope, &source_id, &error, started, finished);
                            }
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
                                started,
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
                        started,
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
        let account_id = scope.account_id;
        if let Ok(cancellations) = self.account_cancellations.lock() {
            if let Some(token) = cancellations.get(&account_id) {
                if token.is_cancelled() {
                    return Err(SourceError::Cancelled);
                }
            }
        }
        if let Ok(storage) = self.storage.lock() {
            if let Ok(Some(acc)) = storage.get_account(account_id) {
                if !acc.enabled || matches!(acc.lifecycle, usage_core::AccountLifecycle::Archived) {
                    return Err(SourceError::Cancelled);
                }
            }
        }
        // Post-fetch identity routing: a strategy-observed fingerprint
        // (e.g. App Server login) that contradicts the stored account
        // discards the payload before any commit — no cross-account
        // attribution, ever.
        if let Some(observed) = payload.observed_identity.as_deref() {
            self.verify_observed_identity(scope, observed)?;
        }
        if request.account.id != scope.account_id
            || request.account.provider_id != scope.provider_id
        {
            return Err(SourceError::IdentityMismatch);
        }
        let snapshot = payload
            .snapshot
            .ok_or(SourceError::Parse { schema: "snapshot" })?;
        if snapshot.account_id != scope.account_id {
            return Err(SourceError::IdentityMismatch);
        }
        if snapshot.provider_id != scope.provider_id || snapshot.product_id != scope.product_id {
            return Err(SourceError::Parse {
                schema: "payload_scope",
            });
        }
        if payload.coverage != snapshot.coverage {
            return Err(SourceError::Parse {
                schema: "payload_coverage",
            });
        }
        if payload
            .usage
            .iter()
            .any(|record| record.account_id != scope.account_id)
            || payload
                .costs
                .iter()
                .any(|record| record.account_id != scope.account_id)
        {
            return Err(SourceError::IdentityMismatch);
        }
        if payload
            .usage
            .iter()
            .any(|record| record.product_id != scope.product_id)
            || payload
                .costs
                .iter()
                .any(|record| record.product_id != scope.product_id)
        {
            return Err(SourceError::Parse {
                schema: "payload_scope",
            });
        }
        for quota in &snapshot.quotas {
            quota
                .validate()
                .map_err(|_| SourceError::Parse { schema: "quota" })?;
        }
        let payload_json = serde_json::to_string(&snapshot)
            .map_err(|_| SourceError::Parse { schema: "snapshot" })?;
        let fetched_at = snapshot.fetched_at;
        let observed_at = snapshot.observed_at;
        #[cfg(test)]
        if let Ok(hook_lock) = self.pre_commit_hook.lock() {
            if let Some(hook) = hook_lock.as_ref() {
                hook();
            }
        }
        let commit_res = self
            .storage
            .lock()
            .map_err(|_| SourceError::Unavailable)?
            .commit_refresh_bundle_if_active(&usage_storage::RefreshBundle {
                account_id: scope.account_id,
                product_id: &scope.product_id,
                snapshot_payload_json: &payload_json,
                observed_at,
                fetched_at,
                usage: &payload.usage,
                costs: &payload.costs,
            })
            .map_err(|_| SourceError::Unavailable)?;

        let report = match commit_res {
            usage_storage::CommitBundleResult::Committed(rep) => rep,
            usage_storage::CommitBundleResult::AccountInactive => {
                return Err(SourceError::Cancelled);
            }
            usage_storage::CommitBundleResult::AccountMissing => {
                return Err(SourceError::Unavailable);
            }
        };
        let (usage_imported, costs_imported) = (report.usage_accepted, report.costs_accepted);
        let source_error = payload.source_error.clone();
        if source_error.is_none() {
            self.on_success(scope, source_id, &snapshot, payload.schema_fingerprint);
        }
        Ok(RefreshOutcome {
            scope: scope.clone(),
            snapshot: Some(snapshot),
            stale: source_error.is_some(),
            error: source_error,
            attempts: vec![],
            warnings: payload.warnings,
            costs_imported,
            usage_imported,
            selected_source: Some(source_id.to_string()),
            schema_fingerprint: Some(payload.schema_fingerprint.to_string()),
        })
    }

    /// Last-known-good fill for scopes skipped by cadence: serves the
    /// stored snapshot as stale with no error and no attempt, so an
    /// idle scope never shows a failure it never had. Cooling scopes
    /// keep their persisted error state untouched elsewhere.
    pub fn lkg_outcome(&self, scope: &ScopeKey) -> RefreshOutcome {
        self.stale_outcome_inner(scope, None, vec![], false)
    }

    fn stale_outcome(
        &self,
        scope: &ScopeKey,
        error: Option<SourceError>,
        attempts: Vec<AttemptRecord>,
    ) -> RefreshOutcome {
        self.stale_outcome_inner(scope, error, attempts, true)
    }

    fn stale_outcome_inner(
        &self,
        scope: &ScopeKey,
        error: Option<SourceError>,
        attempts: Vec<AttemptRecord>,
        update_state: bool,
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
        if update_state {
            if let Ok(mut states) = self.states.lock() {
                let state = states.entry(scope.clone()).or_default();
                state.last_attempt = Some(self.hosts.clock.now());
                state.error_code = error_code;
            }
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

    /// Post-fetch identity verification for sources that observe a
    /// login (App Server, API credentials). Local file sources yield no
    /// observed identity and skip this gate (history attribution is
    /// decided per connection at import time instead).
    fn verify_observed_identity(
        &self,
        scope: &ScopeKey,
        observed: &str,
    ) -> Result<(), SourceError> {
        use crate::account_router::{route, Routing};
        use usage_core::IdentityConfidence;

        let (stored, confidence) = self
            .storage
            .lock()
            .ok()
            .and_then(|s| s.get_account(scope.account_id).ok())
            .flatten()
            .map(|m| (m.external_identity, m.identity_confidence))
            .unwrap_or((None, IdentityConfidence::Unknown));
        // RC3 stored the descriptive type+plan fingerprint for Codex App
        // Server. It was never unique, so a new account-id observation is
        // allowed to replace that weak legacy marker instead of making an
        // existing profile fail forever after upgrade.
        let legacy_codex_identity = Self::is_legacy_codex_identity(scope, stored.as_deref());
        let stored_for_routing = (!legacy_codex_identity)
            .then_some(stored.as_deref())
            .flatten();
        match route(stored_for_routing, confidence, Some(observed)) {
            Routing::Mismatch { .. } => Err(SourceError::IdentityMismatch),
            Routing::Attributed {
                confidence: IdentityConfidence::Weak,
            } if stored.is_none() || legacy_codex_identity => {
                if let Ok(mut storage) = self.storage.lock() {
                    let _ = storage.set_account_identity(
                        scope.account_id,
                        observed,
                        IdentityConfidence::Weak,
                    );
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn is_legacy_codex_identity(scope: &ScopeKey, stored: Option<&str>) -> bool {
        scope.provider_id == "openai"
            && scope.product_id == "codex"
            && stored.is_some_and(|value| value.starts_with("codex-app-server:"))
    }

    /// Lazy per-scope cooldown restoration (P0-09). Memory is
    /// authoritative once populated; storage is consulted exactly when
    /// the scope has no live cooldown, which covers scopes created
    /// after coordinator startup and post-restart scopes alike.
    fn restore_scope_cooldown(&self, scope: &ScopeKey) {
        let live = self
            .states
            .lock()
            .ok()
            .and_then(|states| states.get(scope).and_then(|s| s.cooldown_until));
        if live.is_some_and(|until| until > self.hosts.clock.now()) {
            return;
        }
        let source_ids = scope_source_ids(scope);
        let persisted = self
            .storage
            .lock()
            .ok()
            .and_then(|s| s.load_cooldowns().ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(account_id, source, until_at)| {
                let account = account_id.parse::<Uuid>().ok()?;
                let until = until_at.parse::<DateTime<Utc>>().ok()?;
                (account == scope.account_id
                    && source_ids.iter().any(|source_id| source_id == &source)
                    && until > self.hosts.clock.now())
                .then_some(until)
            })
            .max();
        if let (Some(until), Ok(mut states)) = (persisted, self.states.lock()) {
            let state = states.entry(scope.clone()).or_default();
            state.cooldown_until = Some(until);
            state.error_code = Some("rate_limited".into());
        }
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
        started: DateTime<Utc>,
    ) -> AttemptRecord {
        // Real measured interval: storage and diagnostics see when the
        // attempt actually ran instead of a fabricated now/now pair.
        let finished = self.hosts.clock.now();
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
                started_at: started,
                finished_at: finished,
            });
        }
        AttemptRecord {
            source: source.to_string(),
            status,
            safe_code: safe_code.map(|s| s.to_string()),
            started_at: started,
            finished_at: finished,
        }
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

fn scope_source_ids(scope: &ScopeKey) -> Vec<String> {
    // A scope can have several fallback sources. Cooldowns are persisted per
    // account+source, so restore every source that can actually serve it.
    crate::registry::strategies_for(&scope.provider_id, &scope.product_id)
        .iter()
        .map(strategy_source_name)
        .collect()
}

fn strategy_source_name(kind: &StrategyKind) -> String {
    match kind {
        StrategyKind::CodexAppServer => "codex-app-server".into(),
        StrategyKind::CodexJsonl => "codex-local-jsonl".into(),
        StrategyKind::ClaudePtyUsage => "claude-pty-usage".into(),
        StrategyKind::ClaudeOAuthUsage => "claude-oauth-usage".into(),
        StrategyKind::ClaudeJsonl => "claude-local-jsonl".into(),
        StrategyKind::OpenAiCosts => "openai-api-costs".into(),
    }
}

fn build_strategy(
    hosts: &Hosts,
    kind: StrategyKind,
    custom_root: Option<PathBuf>,
) -> Option<Box<dyn FetchStrategy>> {
    match kind {
        // Account-scoped quota (§11.5): the server is spawned with
        // CODEX_HOME set to the scope's own root, so it reads THAT
        // profile's auth. Proven: an empty root reports an empty
        // account (AuthenticationRequired), never the default login —
        // no cross-profile fallback is structurally possible.
        StrategyKind::CodexAppServer => Some(Box::new(
            usage_providers::codex_appserver::CodexAppServerStrategy {
                executables: crate::registry::resolve_codex_executables(hosts),
                stub: None,
                custom_root: custom_root.clone(),
            },
        )),
        StrategyKind::CodexJsonl => Some(Box::new(
            usage_providers::codex::CodexJsonlStrategy::new(custom_root),
        )),
        // Account-scoped quota (§11.5): the CLI is driven with
        // CLAUDE_CONFIG_DIR set to the scope's own root, so it reads
        // THAT profile's auth. Custom profiles keep JSONL too; a root
        // without login surfaces auth state, never another profile.
        StrategyKind::ClaudePtyUsage => {
            Some(Box::new(usage_providers::claude::ClaudePtyUsageStrategy {
                executables: crate::registry::resolve_claude_executables(hosts),
                stub: None,
                custom_root: custom_root.clone(),
            }))
        }
        StrategyKind::ClaudeOAuthUsage if custom_root.is_some() => None,
        // OAuth stays default-only: there is no per-profile scoped
        // credential source, and inventing one would violate §11.5.
        StrategyKind::ClaudeOAuthUsage => Some(Box::new(
            usage_providers::claude::ClaudeOAuthUsageStrategy { oauth_url: None },
        )),
        StrategyKind::ClaudeJsonl => Some(Box::new(
            usage_providers::claude::ClaudeJsonlStrategy::new(custom_root),
        )),
        StrategyKind::OpenAiCosts => {
            Some(Box::new(usage_providers::openai_api::OpenAiCostsStrategy {
                base_url: custom_root
                    .as_ref()
                    .and_then(|p| p.to_str())
                    .map(str::to_string),
            }))
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
            req: HttpJsonRequest<'_>,
        ) -> Result<HttpJsonResponse, usage_host::HostError> {
            *self.calls.lock().unwrap() += 1;
            if self.delay_ms > 0 {
                tokio::select! {
                    biased;
                    _ = req.cancel.cancelled() => return Err(usage_host::HostError::Cancelled),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)) => {}
                }
            }
            if req.cancel.is_cancelled() {
                return Err(usage_host::HostError::Cancelled);
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
        let files = Arc::new(ScopedFiles);
        Arc::new(Hosts {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http,
            files: files.clone(),
            file_scope: files,
            network: Arc::new(ObservedNetwork::default()),
            process: Arc::new(usage_host::AllowlistedProcess::default()),
            pty: Arc::new(usage_host::AllowlistedPty::default()),
        })
    }

    struct AdjustableClock {
        now: Mutex<DateTime<Utc>>,
    }

    impl AdjustableClock {
        fn new(now: DateTime<Utc>) -> Arc<Self> {
            Arc::new(Self {
                now: Mutex::new(now),
            })
        }

        fn advance(&self, duration: chrono::Duration) {
            let mut now = self.now.lock().unwrap();
            *now += duration;
        }
    }

    impl usage_host::ClockHost for AdjustableClock {
        fn now(&self) -> DateTime<Utc> {
            *self.now.lock().unwrap()
        }
    }

    fn test_hosts_with_clock(
        http: Arc<ScriptHttp>,
        clock: Arc<dyn usage_host::ClockHost>,
    ) -> Arc<Hosts> {
        let files = Arc::new(ScopedFiles);
        Arc::new(Hosts {
            clock,
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http,
            files: files.clone(),
            file_scope: files,
            network: Arc::new(ObservedNetwork::default()),
            process: Arc::new(usage_host::AllowlistedProcess::default()),
            pty: Arc::new(usage_host::AllowlistedPty::default()),
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
    async fn offline_skips_official_api_without_http_calls() {
        // Offline hosts never dial provider APIs: the OfficialApi
        // strategy is skipped pre-flight (no HTTP call), while local
        // ends would still run in the same tick.
        let http = script_http(ScriptBehavior::CostPage(cost_page()));
        let files = Arc::new(ScopedFiles);
        let network = Arc::new(ObservedNetwork::default());
        network.set(usage_host::OnlineState::Offline);
        let hosts = Arc::new(Hosts {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http: http.clone(),
            files: files.clone(),
            file_scope: files,
            network,
            process: Arc::new(usage_host::AllowlistedProcess::default()),
            pty: Arc::new(usage_host::AllowlistedPty::default()),
        });
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let coordinator = Arc::new(Coordinator::new(hosts, storage.clone()));
        let acc = account("openai", "T");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };
        let outcomes = coordinator
            .refresh_many(vec![RefreshRequest {
                scope: scope.clone(),
                account: acc.clone(),
                custom_root: None,
                secret: Some(b"sk-test".to_vec()),
                trigger: Trigger::Scheduled,
            }])
            .await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(*http.calls.lock().unwrap(), 0, "no HTTP while offline");
        assert!(outcomes[0].attempts.iter().any(|attempt| {
            attempt.source == "openai-api-costs" && matches!(attempt.status, AttemptStatus::Skipped)
        }));
    }

    /// Ordered Claude ends tolerate a dormant OAuth fallback (§10.3.2
    /// is conditional: no verified endpoint + no scoped credential =
    /// NotConfigured, never an error). Proved at strategy level so it
    /// holds on any host without spawning real CLIs: a login-less end
    /// is skipped, the dormant OAuth end is skipped, and the history
    /// end serves with empty quotas. (Coordinator order itself comes
    /// from the descriptor and is pinned by
    /// `claude_strategies_follow_descriptor_order`.)
    #[tokio::test]
    async fn claude_ordering_skips_dormant_oauth_on_any_host() {
        use usage_providers::claude::{ClaudeJsonlStrategy, ClaudeOAuthUsageStrategy};
        use usage_providers::strategy::{Availability, FetchContext, FetchStrategy};
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(
            projects.join("t.jsonl"),
            "{\"timestamp\":\"2026-09-08T12:00:00Z\",\"uuid\":\"u1\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20}}}\n",
        )
        .unwrap();
        let http = script_http(ScriptBehavior::CostPage(cost_page()));
        let hosts = test_hosts(http);
        let acc = account("anthropic", "T");
        let descriptor =
            usage_providers::descriptor::find_descriptor("anthropic", "claude-code").unwrap();
        let cancel = usage_host::CancellationToken::new();
        let ctx = FetchContext {
            account: &acc,
            facade: hosts.facade(&crate::registry::facade_policy_for(
                "anthropic",
                "claude-code",
            )),
            descriptor,
            timeout: std::time::Duration::from_secs(5),
            local_root: Some(
                hosts
                    .file_scope
                    .scope_root(dir.path(), &cancel)
                    .expect("scope temp root"),
            ),
            secret: None,
            cancel,
        };
        // S1 without executable: NotConfigured (skip, no spawn).
        let pty = usage_providers::claude::ClaudePtyUsageStrategy::new(vec![]);
        // S2 without URL/secret: NotConfigured (dormant, never fires).
        let oauth = ClaudeOAuthUsageStrategy { oauth_url: None };
        // History end serves its own trust state with empty quotas.
        let jsonl = ClaudeJsonlStrategy::new(Some(dir.path().to_path_buf()));
        assert_eq!(pty.availability(&ctx).await, Availability::NotConfigured);
        assert_eq!(oauth.availability(&ctx).await, Availability::NotConfigured);
        assert_eq!(jsonl.availability(&ctx).await, Availability::Ready);
        let payload = jsonl.fetch(&ctx).await.expect("history serves");
        let snapshot = payload.snapshot.expect("history snapshot");
        assert!(snapshot.quotas.is_empty());
        assert_eq!(snapshot.coverage, usage_core::Coverage::UnverifiedSemantics);
    }

    #[tokio::test]
    async fn lkg_outcome_serves_stored_snapshot_without_error_or_attempts() {
        // Cadence-skipped scopes must render their last-known-good
        // payload silently: stale view, no error, no attempt recorded.
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
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        // Empty storage: stale shell, still error-free.
        let empty = coordinator.lkg_outcome(&scope);
        assert!(empty.stale);
        assert!(empty.error.is_none());
        assert!(empty.snapshot.is_none());
        assert!(empty.attempts.is_empty());
        assert!(!coordinator.states_snapshot().contains_key(&scope));
        // After a successful refresh the stored snapshot is served.
        let first = coordinator
            .refresh_many(vec![RefreshRequest {
                scope: scope.clone(),
                account: acc.clone(),
                custom_root: Some(dir.path().join("sessions")),
                secret: None,
                trigger: Trigger::Manual,
            }])
            .await;
        assert_eq!(first.len(), 1);
        assert!(!first[0].stale);
        let filled = coordinator.lkg_outcome(&scope);
        assert!(filled.stale);
        assert!(filled.error.is_none());
        assert!(filled.attempts.is_empty());
        let snapshot = filled.snapshot.expect("LKG snapshot");
        assert!(!snapshot.quotas.is_empty());
        assert!(matches!(
            snapshot.freshness,
            usage_core::Freshness::Stale(_)
        ));
        let state_before = coordinator
            .states_snapshot()
            .get(&scope)
            .cloned()
            .expect("refresh state");
        let _ = coordinator.lkg_outcome(&scope);
        let state_after = coordinator
            .states_snapshot()
            .get(&scope)
            .cloned()
            .expect("refresh state after LKG");
        assert_eq!(state_after.last_attempt, state_before.last_attempt);
        assert_eq!(state_after.error_code, state_before.error_code);
    }

    #[tokio::test]
    async fn production_triggers_coalesce_into_one_execution() {
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
        // Two production triggers (scheduler + manual) start together
        // for one scope: both await the shared task, the strategy runs
        // exactly once.
        let first_requests: Vec<_> = (0..(policy::REFRESH_PARALLELISM + 1))
            .map(|_| RefreshRequest {
                scope: scope.clone(),
                account: acc.clone(),
                custom_root: None,
                secret: Some(b"sk-test".to_vec()),
                trigger: Trigger::Scheduled,
            })
            .collect();
        let first = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.refresh_many(first_requests).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let second = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .refresh_many(vec![RefreshRequest {
                        scope,
                        account: acc,
                        custom_root: None,
                        secret: Some(b"sk-test".to_vec()),
                        trigger: Trigger::Manual,
                    }])
                    .await
            })
        };
        let (first, second) = tokio::join!(first, second);
        let (first, second) = (first.unwrap(), second.unwrap());
        assert_eq!(first.len(), policy::REFRESH_PARALLELISM + 1);
        assert_eq!(second.len(), 1);
        assert!(first[0].snapshot.is_some());
        assert!(second[0].snapshot.is_some());
        assert_eq!(*http.calls.lock().unwrap(), 1);
        assert_eq!(first[0].costs_imported, 1);
    }

    #[tokio::test]
    async fn different_scopes_run_in_parallel() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 200,
        });
        // NOTE: ScriptHttp serves the same page list to every scope, so
        // this test uses two API accounts on the same product scope key
        // shape but different accounts: both must complete, each with
        // its own execution.
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let acc_a = account("openai", "A");
        let acc_b = account("openai", "B");
        storage.lock().unwrap().upsert_account(&acc_a).unwrap();
        storage.lock().unwrap().upsert_account(&acc_b).unwrap();
        let outcomes = coordinator
            .refresh_many(vec![
                RefreshRequest {
                    scope: ScopeKey {
                        account_id: acc_a.id,
                        provider_id: "openai".into(),
                        product_id: "openai-api".into(),
                    },
                    account: acc_a,
                    custom_root: None,
                    secret: Some(b"sk-test".to_vec()),
                    trigger: Trigger::Scheduled,
                },
                RefreshRequest {
                    scope: ScopeKey {
                        account_id: acc_b.id,
                        provider_id: "openai".into(),
                        product_id: "openai-api".into(),
                    },
                    account: acc_b,
                    custom_root: None,
                    secret: Some(b"sk-test".to_vec()),
                    trigger: Trigger::Manual,
                },
            ])
            .await;
        assert_eq!(outcomes.len(), 2);
        assert_eq!(*http.calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn shutdown_cancels_in_flight_refresh() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 500,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };
        let handle = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .refresh_many(vec![RefreshRequest {
                        scope,
                        account: acc,
                        custom_root: None,
                        secret: Some(b"sk-test".to_vec()),
                        trigger: Trigger::Scheduled,
                    }])
                    .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        coordinator.shutdown();
        let outcomes = handle.await.unwrap();
        assert!(outcomes.iter().all(|o| o.stale));
    }

    #[tokio::test]
    async fn disabling_account_cancels_in_flight_refresh() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 500,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };
        let account_id = acc.id;
        let handle = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .refresh_many(vec![RefreshRequest {
                        scope,
                        account: acc,
                        custom_root: None,
                        secret: Some(b"sk-test".to_vec()),
                        trigger: Trigger::Scheduled,
                    }])
                    .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        coordinator.cancel_account(account_id);
        let outcomes = handle.await.unwrap();
        assert!(outcomes.iter().all(|o| o.stale));
        assert!(outcomes
            .iter()
            .all(|o| matches!(o.error, Some(SourceError::Cancelled))));
        // Active disable must guarantee no commit
        assert!(storage
            .lock()
            .unwrap()
            .last_known_good(account_id, "openai-api")
            .unwrap()
            .is_none());
        assert_eq!(
            storage
                .lock()
                .unwrap()
                .storage_status()
                .unwrap()
                .cost_records,
            0
        );
    }

    #[tokio::test]
    async fn toctou_pre_commit_disable_aborts_commit_atomically() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 0,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let account_id = acc.id;
        let scope = ScopeKey {
            account_id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };

        // Install hook that runs exactly at the pre-commit boundary after source fetch finishes
        {
            let storage_clone = storage.clone();
            *coordinator.pre_commit_hook.lock().unwrap() = Some(Arc::new(move || {
                storage_clone
                    .lock()
                    .unwrap()
                    .update_managed_account(account_id, None, Some(false), None)
                    .unwrap();
            }));
        }

        let outcomes = coordinator
            .refresh_many(vec![RefreshRequest {
                scope,
                account: acc,
                custom_root: None,
                secret: Some(b"sk-test".to_vec()),
                trigger: Trigger::Scheduled,
            }])
            .await;

        assert!(outcomes.iter().all(|o| o.stale));
        assert!(outcomes
            .iter()
            .all(|o| matches!(o.error, Some(SourceError::Cancelled))));
        // Transaction inside storage aborted atomically: zero snapshots and zero cost records committed
        assert!(storage
            .lock()
            .unwrap()
            .last_known_good(account_id, "openai-api")
            .unwrap()
            .is_none());
        assert_eq!(
            storage
                .lock()
                .unwrap()
                .storage_status()
                .unwrap()
                .cost_records,
            0
        );
        assert_eq!(
            storage
                .lock()
                .unwrap()
                .storage_status()
                .unwrap()
                .usage_records,
            0
        );
    }

    #[tokio::test]
    async fn toctou_pre_commit_delete_aborts_commit_atomically() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 0,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let account_id = acc.id;
        let scope = ScopeKey {
            account_id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };

        // Install hook that deletes account exactly pre-commit
        {
            let storage_clone = storage.clone();
            *coordinator.pre_commit_hook.lock().unwrap() = Some(Arc::new(move || {
                storage_clone
                    .lock()
                    .unwrap()
                    .delete_account_and_history(account_id)
                    .unwrap();
            }));
        }

        let outcomes = coordinator
            .refresh_many(vec![RefreshRequest {
                scope,
                account: acc,
                custom_root: None,
                secret: Some(b"sk-test".to_vec()),
                trigger: Trigger::Scheduled,
            }])
            .await;

        assert!(outcomes.iter().all(|o| o.stale));
        assert!(outcomes
            .iter()
            .all(|o| matches!(o.error, Some(SourceError::Unavailable))));
        // Zero snapshots or costs committed
        assert_eq!(
            storage
                .lock()
                .unwrap()
                .storage_status()
                .unwrap()
                .cost_records,
            0
        );
        assert_eq!(
            storage
                .lock()
                .unwrap()
                .storage_status()
                .unwrap()
                .usage_records,
            0
        );
    }

    #[tokio::test]
    async fn deleting_account_cancels_refresh_without_execution_or_commit() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 200,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };

        // Delete account from storage and cancel it
        storage
            .lock()
            .unwrap()
            .delete_account_and_history(acc.id)
            .unwrap();
        coordinator.cancel_account(acc.id);

        let outcome = coordinator
            .refresh_scope(RefreshRequest {
                scope,
                account: acc,
                custom_root: None,
                secret: Some(b"sk-test".to_vec()),
                trigger: Trigger::Manual,
            })
            .await;

        assert!(outcome.stale);
        assert!(matches!(outcome.error, Some(SourceError::Cancelled)));
        assert_eq!(
            *http.calls.lock().unwrap(),
            0,
            "Deleted account must never make HTTP calls"
        );
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

    #[tokio::test]
    async fn persisted_cooldown_survives_coordinator_restart_without_real_sleep() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = script_http(ScriptBehavior::RateLimited);
        let clock = AdjustableClock::new(Utc::now());
        let hosts = test_hosts_with_clock(http.clone(), clock.clone());
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };
        let request = || RefreshRequest {
            scope: scope.clone(),
            account: acc.clone(),
            custom_root: None,
            secret: Some(b"sk-test".to_vec()),
            trigger: Trigger::Scheduled,
        };

        let coordinator = Arc::new(Coordinator::new(hosts.clone(), storage.clone()));
        let first = coordinator.refresh_many(vec![request()]).await;
        assert!(matches!(
            first[0].error,
            Some(SourceError::RateLimited { .. })
        ));
        assert_eq!(*http.calls.lock().unwrap(), 1);
        drop(coordinator);

        // A fresh coordinator lazily restores the persisted server cooldown.
        let restarted = Arc::new(Coordinator::new(hosts, storage.clone()));
        let blocked = restarted.refresh_many(vec![request()]).await;
        assert!(matches!(blocked[0].error, Some(SourceError::Cooldown)));
        assert_eq!(*http.calls.lock().unwrap(), 1);

        // Advance the injected clock; no wall-clock sleep is involved.
        clock.advance(chrono::Duration::seconds(61));
        let after_expiry = restarted.refresh_many(vec![request()]).await;
        assert!(matches!(
            after_expiry[0].error,
            Some(SourceError::RateLimited { .. })
        ));
        assert_eq!(*http.calls.lock().unwrap(), 2);
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
            notifications_muted: false,
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

    #[tokio::test]
    async fn leader_drop_cleans_up_inflight_and_allows_subsequent_refresh() {
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
        let make_request = |scope: ScopeKey, acc: Account| RefreshRequest {
            scope,
            account: acc,
            custom_root: None,
            secret: Some(b"sk-test".to_vec()),
            trigger: Trigger::Manual,
        };

        let c1 = coordinator.clone();
        let r1 = make_request(scope.clone(), acc.clone());
        let leader_handle = tokio::spawn(async move { c1.refresh_scope(r1).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let c2 = coordinator.clone();
        let r2 = make_request(scope.clone(), acc.clone());
        let waiter_handle = tokio::spawn(async move { c2.refresh_scope(r2).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        leader_handle.abort();

        let waiter_res = tokio::time::timeout(std::time::Duration::from_secs(1), waiter_handle)
            .await
            .expect("waiter must not hang")
            .unwrap();
        assert!(matches!(waiter_res.error, Some(SourceError::Cancelled)));

        let r3 = make_request(scope, acc);
        let next = coordinator.refresh_scope(r3).await;
        assert!(next.snapshot.is_some());
    }

    #[tokio::test]
    async fn cancel_account_before_first_refresh_installs_tombstone_and_prevents_fetch() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 0,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };

        // Cancel the account BEFORE any refresh has run or any token was registered
        coordinator.cancel_account(acc.id);

        let outcome = coordinator
            .refresh_scope(RefreshRequest {
                scope,
                account: acc,
                custom_root: None,
                secret: Some(b"sk-test".to_vec()),
                trigger: Trigger::Scheduled,
            })
            .await;

        assert!(outcome.stale);
        assert!(matches!(outcome.error, Some(SourceError::Cancelled)));
        assert_eq!(
            *http.calls.lock().unwrap(),
            0,
            "No network call must be made for cancelled account"
        );
    }

    #[tokio::test]
    async fn queued_refresh_cancelled_before_semaphore_admission_does_not_execute() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 200,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));

        // Saturate the coordinator's semaphore
        let mut blocker_permits = Vec::new();
        for _ in 0..policy::REFRESH_PARALLELISM {
            blocker_permits.push(coordinator.semaphore.clone().acquire_owned().await.unwrap());
        }

        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };

        // Start refresh in background; it will be queued waiting for a semaphore permit
        let c_clone = coordinator.clone();
        let acc_clone = acc.clone();
        let scope_clone = scope.clone();
        let refresh_handle = tokio::spawn(async move {
            c_clone
                .refresh_scope(RefreshRequest {
                    scope: scope_clone,
                    account: acc_clone,
                    custom_root: None,
                    secret: Some(b"sk-test".to_vec()),
                    trigger: Trigger::Scheduled,
                })
                .await
        });

        // Cancel the queued account while it is blocked on semaphore
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        coordinator.cancel_account(acc.id);

        // Now release the semaphore permits
        drop(blocker_permits);

        let outcome = refresh_handle.await.unwrap();
        assert!(outcome.stale);
        assert!(matches!(outcome.error, Some(SourceError::Cancelled)));
        assert_eq!(
            *http.calls.lock().unwrap(),
            0,
            "Cancelled queued job must not execute upon acquiring permit"
        );
    }

    #[tokio::test]
    async fn disabled_account_in_storage_is_cancelled_without_execution() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let http = Arc::new(ScriptHttp {
            respond: Mutex::new(ScriptBehavior::CostPage(cost_page())),
            calls: Mutex::new(0),
            delay_ms: 0,
        });
        let coordinator = Arc::new(Coordinator::new(test_hosts(http.clone()), storage.clone()));
        let acc = account("openai", "API");
        storage.lock().unwrap().upsert_account(&acc).unwrap();
        storage
            .lock()
            .unwrap()
            .update_managed_account(acc.id, None, Some(false), None)
            .unwrap();
        let scope = ScopeKey {
            account_id: acc.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };

        let outcome = coordinator
            .refresh_scope(RefreshRequest {
                scope,
                account: acc,
                custom_root: None,
                secret: Some(b"sk-test".to_vec()),
                trigger: Trigger::Manual,
            })
            .await;

        assert!(outcome.stale);
        assert!(matches!(outcome.error, Some(SourceError::Cancelled)));
        assert_eq!(*http.calls.lock().unwrap(), 0);
    }
}

#[cfg(test)]
mod app_server_scope_tests {
    use super::*;
    use std::sync::Arc;
    use usage_host::{
        AllowlistedProcess, AllowlistedPty, MemoryKeychain, NullLogger, ObservedNetwork,
        ReqwestHttpHost, ScopedFiles, SystemClock,
    };

    fn bare_hosts() -> Hosts {
        let files = Arc::new(ScopedFiles);
        Hosts {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http: Arc::new(ReqwestHttpHost::default()),
            files: files.clone(),
            file_scope: files,
            network: Arc::new(ObservedNetwork::default()),
            process: Arc::new(AllowlistedProcess::default()),
            pty: Arc::new(AllowlistedPty::default()),
        }
    }

    #[test]
    fn app_server_serves_default_scopes_only() {
        // Superseded by account-scoped routing
        // (codex_quota_end_serves_every_scope_with_own_root): the App
        // Server is spawned per scope with that scope's CODEX_HOME.
        // Kept as a smoke pin for the default scope path.
        let hosts = bare_hosts();
        let strategy = build_strategy(&hosts, StrategyKind::CodexAppServer, None);
        assert!(strategy.is_some());
    }

    #[test]
    fn claude_quota_ends_serve_default_scopes_only() {
        // Account-scoped quota (§11.5): the PTY end is built for every
        // scope with the scope's own root (CLAUDE_CONFIG_DIR); the
        // OAuth end stays default-only (no per-profile credential).
        // Custom roots never inherit the default login's quota.
        let hosts = bare_hosts();
        assert!(build_strategy(&hosts, StrategyKind::ClaudePtyUsage, None).is_some());
        assert!(build_strategy(&hosts, StrategyKind::ClaudeOAuthUsage, None).is_some());
        assert!(build_strategy(&hosts, StrategyKind::ClaudeJsonl, None).is_some());
        let custom = Some(PathBuf::from("/tmp/custom-claude"));
        assert!(build_strategy(&hosts, StrategyKind::ClaudePtyUsage, custom.clone()).is_some());
        assert!(build_strategy(&hosts, StrategyKind::ClaudeOAuthUsage, custom.clone()).is_none());
        assert!(build_strategy(&hosts, StrategyKind::ClaudeJsonl, custom).is_some());
    }

    #[test]
    fn codex_quota_end_serves_every_scope_with_own_root() {
        // Same account-scoping for Codex: every scope gets an App
        // Server strategy bound to its own CODEX_HOME.
        let hosts = bare_hosts();
        assert!(build_strategy(&hosts, StrategyKind::CodexAppServer, None).is_some());
        let custom = Some(PathBuf::from("/tmp/custom-codex"));
        assert!(build_strategy(&hosts, StrategyKind::CodexAppServer, custom.clone()).is_some());
        assert!(build_strategy(&hosts, StrategyKind::CodexJsonl, custom).is_some());
    }
}
