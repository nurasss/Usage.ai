use chrono::{Duration, Local, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::AppHandle;
use usage_host::NetworkHost;
use usage_providers::descriptor::{find_descriptor, AccountModel};
use usage_runtime::{AccountMeta, Coordinator, RefreshOutcome, RefreshRequest, ScopeKey, Trigger};
use uuid::Uuid;

use crate::appstate::AppState;
use crate::dto::{
    from_snapshot, unavailable, AppSnapshot, MoneyDto, OverviewSegment, ProviderCostDto,
    ProviderDto,
};

/// Tauri wiring only: provider/account orchestration is owned by
/// `usage-runtime`, while this module adapts its results to app DTOs.
pub async fn collect_requests(state: &AppState) -> Vec<(RefreshRequest, AccountMeta)> {
    usage_runtime::collect_requests(state.hosts.as_ref(), state.storage.as_ref()).await
}

/// Tauri wiring only: runtime returns safe import counters; the shell keeps
/// them in the diagnostics state for the UI.
pub async fn import_all_history(state: &AppState) {
    let reports = usage_runtime::import_all_history(
        &state.hosts,
        &state.storage,
        state.coordinator.as_ref(),
        read_retention_days(state),
    )
    .await;
    if let Ok(mut stats) = state.import_stats.lock() {
        *stats = reports;
    }
}

/// Full refresh: history import, coordinated refresh, snapshot
/// assembly, cache, notifications and tray metric.
pub async fn run_full_refresh(state: &AppState, app: &AppHandle, trigger: Trigger) -> AppSnapshot {
    import_all_history(state).await;
    let mut pairs = collect_requests(state).await;
    for (request, _) in pairs.iter_mut() {
        request.trigger = trigger;
    }
    let metas: HashMap<ScopeKey, AccountMeta> = pairs
        .iter()
        .map(|(request, meta)| (request.scope.clone(), meta.clone()))
        .collect();
    let requests = pairs.into_iter().map(|(request, _)| request).collect();
    let outcomes = state.coordinator.refresh_many(requests).await;
    let mut snapshot = assemble_snapshot(state, &outcomes, &metas);
    // The next-refresh label follows the configured interval instead of
    // a fixed offset; manual mode has no next automatic refresh.
    snapshot.next_refresh_at =
        match crate::commands::settings::read_settings(state).refresh_interval_minutes {
            Some(minutes) => (Utc::now() + Duration::minutes(minutes as i64)).to_rfc3339(),
            None => String::new(),
        };
    cache_snapshot(state, &snapshot);
    crate::commands::notifications::deliver(app, state, &snapshot);
    crate::platform::tray::update_metric(app, state, &snapshot);
    if let Ok(mut cached) = state.cached.lock() {
        *cached = Some(snapshot.clone());
    }
    snapshot
}

pub fn assemble_snapshot(
    state: &AppState,
    outcomes: &[RefreshOutcome],
    metas: &HashMap<ScopeKey, AccountMeta>,
) -> AppSnapshot {
    let states = state.coordinator.states_snapshot();
    let mut providers: Vec<ProviderDto> = outcomes
        .iter()
        .map(|outcome| outcome_to_dto(outcome, metas, &states))
        .collect();
    // Discovery-only products stay visible as honest blockers, driven
    // by descriptors rather than a hard-coded list.
    for descriptor in usage_providers::descriptor::all_descriptors()
        .iter()
        .filter(|d| d.account_model == AccountModel::DiscoveryOnly)
    {
        if !providers.iter().any(|p| {
            p.provider_id == descriptor.provider_id && p.product_id == descriptor.product_id
        }) {
            providers.push(unavailable(
                descriptor.provider_id,
                descriptor.product_id,
                descriptor.provider_name,
                descriptor.product_name,
                usage_core::ConnectionState::UnsupportedSource,
            ));
        }
    }
    for provider in &mut providers {
        attach_usage_and_cost(state, provider);
        attach_balances(state, provider);
    }
    let aggregate_inputs = providers
        .iter()
        .filter_map(|provider| {
            provider.account_id.parse::<Uuid>().ok().map(|account_id| {
                usage_runtime::ProviderCostInput {
                    account_id,
                    product_id: provider.product_id.clone(),
                    provider_name: provider.provider_name.clone(),
                    alias: provider.alias.clone(),
                }
            })
        })
        .collect::<Vec<_>>();
    let aggregates = state
        .storage
        .lock()
        .map(|storage| usage_runtime::build_aggregates(&storage, &aggregate_inputs))
        .unwrap_or_default();
    // Local-only sources work without internet; the flag reflects the
    // observed network state, never a hard-coded constant.
    let offline = matches!(state.network.state(), usage_host::OnlineState::Offline);
    AppSnapshot {
        mode: "live".into(),
        offline,
        providers,
        overview: adapt_segments(aggregates.overview),
        model_breakdown: adapt_segments(aggregates.model_breakdown),
        account_breakdown: adapt_segments(aggregates.account_breakdown),
        project_breakdown: adapt_segments(aggregates.project_breakdown),
        costs_by_period: aggregates
            .costs_by_period
            .into_iter()
            .map(|(period, rows)| {
                (
                    period,
                    rows.into_iter()
                        .map(|row| ProviderCostDto {
                            account_id: row.account_id,
                            product_id: row.product_id,
                            label: row.label,
                            reported: row
                                .reported
                                .into_iter()
                                .map(|money| MoneyDto {
                                    amount: money.amount,
                                    currency: money.currency,
                                })
                                .collect(),
                            estimated: row
                                .estimated
                                .into_iter()
                                .map(|money| MoneyDto {
                                    amount: money.amount,
                                    currency: money.currency,
                                })
                                .collect(),
                        })
                        .collect(),
                )
            })
            .collect(),
        unverified_overview: adapt_segments(aggregates.unverified_overview),
        excluded_unverified_count: aggregates.excluded_unverified_count,
        next_refresh_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
    }
}

fn adapt_segments(
    values: HashMap<String, Vec<usage_runtime::Segment>>,
) -> HashMap<String, Vec<OverviewSegment>> {
    values
        .into_iter()
        .map(|(period, segments)| {
            (
                period,
                segments
                    .into_iter()
                    .map(|segment| OverviewSegment {
                        label: segment.label,
                        value: segment.value,
                        color: segment.color,
                    })
                    .collect(),
            )
        })
        .collect()
}

fn outcome_to_dto(
    outcome: &RefreshOutcome,
    metas: &HashMap<ScopeKey, AccountMeta>,
    states: &HashMap<ScopeKey, usage_runtime::RefreshState>,
) -> ProviderDto {
    let descriptor = find_descriptor(&outcome.scope.provider_id, &outcome.scope.product_id);
    let (provider_name, product_name) = descriptor
        .map(|d| (d.provider_name, d.product_name))
        .unwrap_or(("Unknown", "Unknown"));
    let alias = metas
        .get(&outcome.scope)
        .map(|m| m.alias.as_str())
        .unwrap_or("Локальная история");
    let mut dto = match &outcome.snapshot {
        Some(snapshot) => from_snapshot(snapshot.clone(), provider_name, product_name, alias),
        None => {
            let state = outcome
                .error
                .as_ref()
                .map(crate::dto::source_state)
                .unwrap_or(usage_core::ConnectionState::Unavailable);
            let mut dto = unavailable(
                &outcome.scope.provider_id,
                &outcome.scope.product_id,
                provider_name,
                product_name,
                state,
            );
            dto.account_id = outcome.scope.account_id.to_string();
            dto.alias = alias.into();
            dto
        }
    };
    // The snapshot remains the last-known-good payload, while the current
    // failed attempt is exposed independently and prevents a healthy label.
    if let Some(error) = &outcome.error {
        dto.connection_state = crate::dto::source_state(error);
        dto.current_error = Some(error.safe_code().into());
    }
    if outcome.stale && !matches!(dto.freshness, usage_core::Freshness::Stale(_)) {
        dto.freshness = usage_core::Freshness::Stale(0);
    }
    if let Some(runtime_state) = states.get(&outcome.scope) {
        dto.last_successful_refresh = runtime_state.last_success.map(|d| d.to_rfc3339());
        dto.last_refresh_attempt = runtime_state.last_attempt.map(|d| d.to_rfc3339());
    }
    // Fallback labeling (§9.3.3): codex quotas that did not come from
    // the verified App Server source are locally observed observations,
    // never authoritative values.
    dto.quota_fallback = outcome.scope.product_id == "codex"
        && !dto.quotas.is_empty()
        && outcome.selected_source.as_deref() != Some("codex-app-server");
    dto
}

pub fn cache_snapshot(state: &AppState, value: &AppSnapshot) {
    if let Ok(json) = serde_json::to_string(value) {
        if let Ok(mut storage) = state.storage.lock() {
            let _ = storage.set_cache("app_snapshot", &json);
        }
    }
}

pub fn attach_usage_and_cost(state: &AppState, provider: &mut ProviderDto) {
    let Ok(account_id) = provider.account_id.parse::<Uuid>() else {
        return;
    };
    let Ok(storage) = state.storage.lock() else {
        return;
    };
    let usage =
        usage_runtime::account_usage_and_cost_today(&storage, account_id, &provider.product_id);
    provider.tokens_today = usage.tokens_today;
    provider.reported_cost_today = usage.reported_cost_today.map(|m| MoneyDto {
        amount: m.amount,
        currency: m.currency,
    });
    provider.estimated_cost_today = usage.estimated_cost_today.map(|m| MoneyDto {
        amount: m.amount,
        currency: m.currency,
    });
}

pub fn attach_balances(state: &AppState, provider: &mut ProviderDto) {
    let Ok(account_id) = provider.account_id.parse::<Uuid>() else {
        return;
    };
    if let Ok(storage) = state.storage.lock() {
        if let Ok(rows) = storage.balances_for(account_id, &provider.product_id) {
            provider.balances = rows
                .into_iter()
                .map(|(currency, amount)| MoneyDto { amount, currency })
                .collect();
        }
    }
}

pub fn local_midnight(date: chrono::NaiveDate) -> chrono::DateTime<Local> {
    use chrono::TimeZone;
    let naive = date.and_hms_opt(0, 0, 0).expect("midnight is valid");
    Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| Local.from_local_datetime(&naive).latest())
        .unwrap_or_else(Local::now)
}

fn read_retention_days(state: &AppState) -> i64 {
    crate::commands::settings::read_settings(state)
        .retention_days
        .max(30) as i64
}

/// Primary tray metric: minimum remaining percent across fresh, authoritative quotas.
/// Providers with UnverifiedSemantics or other non-authoritative coverage are excluded
/// so they never render as an authoritative menu-bar metric.
pub fn primary_remaining(providers: &[ProviderDto]) -> Option<f64> {
    providers
        .iter()
        .filter(|p| p.freshness == usage_core::Freshness::Fresh && p.coverage.allows_tray_metric())
        .flat_map(|p| p.quotas.iter())
        .filter_map(|q| q.remaining_percent)
        .fold(None, |min: Option<f64>, value| {
            Some(min.map_or(value, |m: f64| m.min(value)))
        })
}

pub fn coordinator_of(state: &AppState) -> Arc<Coordinator> {
    Arc::clone(&state.coordinator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use usage_core::{
        Capability, ConnectionState, Coverage, Freshness, MetricUnit, Quota, Snapshot, WindowKind,
    };
    use usage_providers::strategy::SourceError;
    use usage_runtime::{RefreshOutcome, RefreshState, ScopeKey};
    use uuid::Uuid;

    #[test]
    fn swr_outcome_to_dto_preserves_lkg_with_stale_and_current_error() {
        // Step 1: Initial successful refresh -> LKG snapshot created
        let scope = ScopeKey {
            account_id: Uuid::new_v4(),
            provider_id: "openai".into(),
            product_id: "codex".into(),
        };
        let mut metas = HashMap::new();
        metas.insert(
            scope.clone(),
            AccountMeta {
                alias: "Локальная история".into(),
            },
        );

        let quota = Quota {
            pool_id: "codex-weekly".into(),
            window_id: None,
            name: "Weekly".into(),
            used_percent: Some(rust_decimal::Decimal::new(28, 0)),
            remaining_percent: Some(rust_decimal::Decimal::new(72, 0)),
            used: Some(28),
            limit: Some(100),
            unit: MetricUnit::Percent,
            resets_at: Some(Utc::now() + Duration::days(3)),
            window_start: None,
            window_kind: WindowKind::Rolling,
            source: "Codex local".into(),
        };

        let initial_snapshot = Snapshot {
            account_id: scope.account_id,
            provider_id: scope.provider_id.clone(),
            product_id: scope.product_id.clone(),
            plan_label: Some("Plus".into()),
            capabilities: [Capability::SubscriptionQuota].into_iter().collect(),
            quotas: vec![quota],
            balances: vec![],
            observed_at: Some(Utc::now() - Duration::minutes(18)),
            fetched_at: Utc::now() - Duration::minutes(18),
            connection_state: ConnectionState::Connected,
            freshness: Freshness::Fresh,
            coverage: Coverage::UnverifiedSemantics,
        };

        let success_outcome = RefreshOutcome {
            scope: scope.clone(),
            snapshot: Some(initial_snapshot.clone()),
            stale: false,
            error: None,
            attempts: vec![],
            warnings: vec![],
            costs_imported: 0,
            usage_imported: 0,
            selected_source: Some("codex-local-jsonl".into()),
            schema_fingerprint: Some("v1".into()),
        };

        let mut states = HashMap::new();
        states.insert(
            scope.clone(),
            RefreshState {
                last_success: Some(Utc::now() - Duration::minutes(18)),
                last_attempt: Some(Utc::now() - Duration::minutes(18)),
                ..Default::default()
            },
        );

        let dto_initial = outcome_to_dto(&success_outcome, &metas, &states);
        assert_eq!(dto_initial.connection_state, ConnectionState::Connected);
        assert_eq!(dto_initial.freshness, Freshness::Fresh);
        assert!(dto_initial.current_error.is_none());
        assert_eq!(dto_initial.quotas.len(), 1);
        assert_eq!(dto_initial.quotas[0].remaining_percent, Some(72.0));

        // Step 2 & 3: Next refresh fails with NetworkError -> serves stale LKG
        let mut stale_lkg = initial_snapshot;
        stale_lkg.freshness = Freshness::Stale(1080);

        let failed_outcome = RefreshOutcome {
            scope: scope.clone(),
            snapshot: Some(stale_lkg),
            stale: true,
            error: Some(SourceError::Network),
            attempts: vec![],
            warnings: vec![],
            costs_imported: 0,
            usage_imported: 0,
            selected_source: None,
            schema_fingerprint: None,
        };

        states.insert(
            scope.clone(),
            RefreshState {
                last_success: Some(Utc::now() - Duration::minutes(18)),
                last_attempt: Some(Utc::now()),
                error_code: Some("network_error".into()),
                ..Default::default()
            },
        );

        let dto_stale = outcome_to_dto(&failed_outcome, &metas, &states);

        // Step 4: returned DTO contains old metric values
        assert_eq!(dto_stale.quotas.len(), 1);
        assert_eq!(dto_stale.quotas[0].remaining_percent, Some(72.0));

        // Step 5: stale == true
        assert!(matches!(dto_stale.freshness, Freshness::Stale(_)));

        // Step 6: current error present
        assert_eq!(dto_stale.current_error, Some("network_error".into()));
        assert_eq!(dto_stale.connection_state, ConnectionState::NetworkError);
        assert!(dto_stale.last_successful_refresh.is_some());
        assert!(dto_stale.last_refresh_attempt.is_some());
    }

    #[tokio::test]
    async fn e2e_refresh_flow_refresh_to_storage_to_dto() {
        use std::sync::{Arc, Mutex};
        use usage_host::{
            async_trait, HostError, Hosts, HttpHost, HttpJsonRequest, HttpJsonResponse,
        };
        use usage_runtime::{Coordinator, RefreshRequest, Trigger};
        use usage_storage::Storage;

        struct TestHttp {
            body: String,
        }

        #[async_trait]
        impl HttpHost for TestHttp {
            async fn get_json(
                &self,
                _req: HttpJsonRequest<'_>,
            ) -> Result<HttpJsonResponse, HostError> {
                Ok(HttpJsonResponse {
                    status: 200,
                    retry_after_secs: None,
                    body: Some(serde_json::from_str(&self.body).unwrap()),
                })
            }
        }

        let now = chrono::Utc::now();
        let start_time = (now - chrono::Duration::hours(2)).timestamp();
        let end_time = now.timestamp();
        let body = format!(
            r#"{{
            "object": "page",
            "has_more": false,
            "next_page": null,
            "data": [{{
                "object": "bucket",
                "start_time": {start_time},
                "end_time": {end_time},
                "results": [{{
                    "object": "organization.costs.result",
                    "amount": {{"value": 12.34, "currency": "usd"}},
                    "line_item": "gpt-4o",
                    "project_id": "proj_123"
                }}]
            }}]
        }}"#
        );

        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let hosts = Hosts {
            http: Arc::new(TestHttp { body }),
            ..Default::default()
        };
        let coordinator = Arc::new(Coordinator::new(Arc::new(hosts), storage.clone()));

        let secret = b"sk-test-secret-12345".to_vec();
        let fp = usage_host::fingerprint("openai-api", &secret);
        let account = usage_core::Account {
            id: Uuid::new_v4(),
            provider_id: "openai".into(),
            external_identity: Some(fp),
            label: "Production API".into(),
            connection_ref: Some("keychain:openai:test".into()),
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        storage.lock().unwrap().upsert_account(&account).unwrap();

        let scope = ScopeKey {
            account_id: account.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
        };

        let request = RefreshRequest {
            scope: scope.clone(),
            account: account.clone(),
            custom_root: None,
            secret: Some(secret),
            trigger: Trigger::Manual,
        };

        let outcome = coordinator.refresh_scope(request).await;

        // Verify outcome was successful and not stale
        assert_eq!(outcome.error, None);
        assert!(!outcome.stale, "outcome was stale: {:?}", outcome);
        assert_eq!(outcome.costs_imported, 1);

        // Verify SQLite storage has the exact decimal money record
        let costs = storage
            .lock()
            .unwrap()
            .cost_records_between(
                chrono::Utc::now() - chrono::Duration::days(30),
                chrono::Utc::now() + chrono::Duration::days(1),
            )
            .unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(
            costs[0].amount_decimal,
            rust_decimal::Decimal::from_str_exact("12.34").unwrap()
        );
        assert_eq!(costs[0].currency, "USD");

        // Verify DTO adaptation
        let mut metas = HashMap::new();
        metas.insert(
            scope.clone(),
            AccountMeta {
                alias: "Production API".into(),
            },
        );
        let states = coordinator.states_snapshot();
        let dto = outcome_to_dto(&outcome, &metas, &states);
        assert_eq!(dto.connection_state, ConnectionState::Connected);
        assert_eq!(dto.freshness, Freshness::Fresh);
        assert!(dto.current_error.is_none());
    }

    fn sample_provider(coverage: Coverage, remaining: f64) -> ProviderDto {
        ProviderDto {
            account_id: Uuid::new_v4().to_string(),
            provider_id: "test".into(),
            product_id: "test-prod".into(),
            provider_name: "Test".into(),
            product_name: "TestProd".into(),
            glyph: "T".into(),
            color: "#fff".into(),
            alias: "Test".into(),
            plan_label: None,
            connection_state: ConnectionState::Connected,
            freshness: Freshness::Fresh,
            coverage,
            fetched_at: "2026-09-09T00:00:00Z".into(),
            observed_at: None,
            capabilities: vec![],
            quotas: vec![crate::dto::QuotaDto {
                pool_id: "default".into(),
                window_id: Some("w1".into()),
                name: "Quota".into(),
                remaining_percent: Some(remaining),
                used: Some((100.0 - remaining) as u64),
                limit: Some(100),
                unit: "percent".into(),
                resets_at: None,
                window_start: None,
                window_kind: "rolling".into(),
                source: "test".into(),
            }],
            tokens_today: None,
            reported_cost_today: None,
            estimated_cost_today: None,
            balances: vec![],
            current_error: None,
            last_successful_refresh: None,
            last_refresh_attempt: None,
            quota_fallback: false,
        }
    }

    #[test]
    fn tray_metric_selection_cases_a_b_c_d() {
        // Case A — verified source (Coverage::Complete) with remaining 20%
        let p_verified = sample_provider(Coverage::Complete, 20.0);
        assert_eq!(
            primary_remaining(std::slice::from_ref(&p_verified)),
            Some(20.0),
            "Case A: verified provider must be eligible for tray metric"
        );

        // Case B — unverified source (Coverage::UnverifiedSemantics) with remaining 20%
        let p_unverified = sample_provider(Coverage::UnverifiedSemantics, 20.0);
        assert_eq!(
            primary_remaining(std::slice::from_ref(&p_unverified)),
            None,
            "Case B: unverified provider must NOT be eligible for ordinary tray metric"
        );

        // Case C — mixed: verified = 63%, unverified = 20%
        let p_ver_63 = sample_provider(Coverage::Complete, 63.0);
        let p_unver_20 = sample_provider(Coverage::UnverifiedSemantics, 20.0);
        assert_eq!(
            primary_remaining(&[p_ver_63, p_unver_20]),
            Some(63.0),
            "Case C: verified provider (63%) must be chosen; unverified (20%) must not override"
        );

        // Case D — only unverified quota available
        let p_unver_only = sample_provider(Coverage::UnverifiedSemantics, 15.0);
        assert_eq!(
            primary_remaining(&[p_unver_only]),
            None,
            "Case D: only unverified quota available must yield None (no numeric metric in tray)"
        );
    }

    #[test]
    fn full_side_effect_regression_unverified_semantics_produces_no_authoritative_side_effects() {
        // Input snapshot: Fresh + Connected + UnverifiedSemantics + 20% remaining
        let p = sample_provider(Coverage::UnverifiedSemantics, 20.0);

        // 1. Tray check: DOES NOT show normal 20%
        let tray = primary_remaining(std::slice::from_ref(&p));
        assert!(
            tray.is_none(),
            "tray must NOT show normal 20% for unverified source"
        );

        // 2. Notification evaluator check: emits none
        let notice_view = usage_runtime::ProviderNoticeView {
            account_id: p.account_id.clone(),
            provider_name: p.provider_name.clone(),
            product_name: p.product_name.clone(),
            connection_state: p.connection_state.clone(),
            fresh: true,
            coverage: p.coverage,
            quotas: vec![usage_runtime::QuotaNoticeView {
                pool_id: "default".into(),
                window_id: Some("w1".into()),
                name: "Quota".into(),
                remaining: Some(20.0),
                resets_at: None,
                window_start: None,
                window_kind: "rolling".into(),
            }],
        };
        let mut planner = usage_runtime::Planner::default();
        let notices = planner.plan_provider(&notice_view, chrono::Utc::now());
        assert!(
            notices.is_empty(),
            "notifications must emit NONE for unverified provider"
        );
    }
}
