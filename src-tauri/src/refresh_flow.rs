use chrono::{DateTime, Duration, Local, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::AppHandle;
use usage_core::{Account, AccountLifecycle, CostKind};
use usage_host::NetworkHost;
use usage_providers::descriptor::{find_descriptor, AccountModel};
use usage_runtime::{Coordinator, RefreshOutcome, RefreshRequest, ScopeKey, Trigger};
use uuid::Uuid;

use crate::appstate::AppState;
use crate::dto::{
    from_snapshot, unavailable, AppSnapshot, MoneyDto, OverviewSegment, ProviderCostDto,
    ProviderDto,
};

/// Stable local account ids (v1 compatible, never auto-merged).
pub fn stable_local_account(namespace: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("usage.ai/local/{namespace}").as_bytes(),
    )
}

fn descriptor_default_root(
    local_dir_env: Option<&str>,
    local_dir_name: Option<&str>,
) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let base = match local_dir_env.and_then(std::env::var_os) {
        Some(custom) => PathBuf::from(custom),
        None => home.join(local_dir_name?),
    };
    Some(base)
}

/// Product home root for local discovery: explicit custom path wins,
/// otherwise env override, otherwise `$HOME/<dot-dir>`.
pub fn product_roots(
    local_dir_env: Option<&str>,
    local_dir_name: Option<&str>,
    custom_path: Option<&str>,
) -> Vec<PathBuf> {
    let mut roots = vec![];
    if let Some(custom) = custom_path.filter(|p| !p.trim().is_empty()) {
        roots.push(PathBuf::from(custom));
    }
    if let Some(default) = descriptor_default_root(local_dir_env, local_dir_name) {
        if !roots.contains(&default) {
            roots.push(default);
        }
    }
    roots
}

pub fn ensure_local_accounts(state: &AppState) {
    let accounts = [
        Account {
            id: stable_local_account("codex"),
            provider_id: "openai".into(),
            external_identity: None,
            label: "Аккаунт 1".into(),
            connection_ref: Some("codex-local".into()),
            lifecycle: AccountLifecycle::Active,
        },
        Account {
            id: stable_local_account("claude-code"),
            provider_id: "anthropic".into(),
            external_identity: None,
            label: "Аккаунт 1".into(),
            connection_ref: Some("claude-local".into()),
            lifecycle: AccountLifecycle::Active,
        },
    ];
    if let Ok(mut storage) = state.storage.lock() {
        for account in &accounts {
            let _ = storage.upsert_account(account);
        }
    }
}

/// Build one refresh request per enabled account/product scope:
/// stable local scopes, managed local scopes (custom roots), and
/// managed API scopes (Keychain secret supplied per call only).
pub async fn collect_requests(state: &AppState) -> Vec<(RefreshRequest, AccountMeta)> {
    ensure_local_accounts(state);
    let mut out = vec![];
    out.push((
        RefreshRequest {
            scope: ScopeKey {
                account_id: stable_local_account("codex"),
                provider_id: "openai".into(),
                product_id: "codex".into(),
            },
            account: Account {
                id: stable_local_account("codex"),
                provider_id: "openai".into(),
                external_identity: None,
                label: "Аккаунт 1".into(),
                connection_ref: Some("codex-local".into()),
                lifecycle: AccountLifecycle::Active,
            },
            custom_root: None,
            secret: None,
            trigger: Trigger::Scheduled,
        },
        AccountMeta {
            alias: "Аккаунт 1".into(),
        },
    ));
    out.push((
        RefreshRequest {
            scope: ScopeKey {
                account_id: stable_local_account("claude-code"),
                provider_id: "anthropic".into(),
                product_id: "claude-code".into(),
            },
            account: Account {
                id: stable_local_account("claude-code"),
                provider_id: "anthropic".into(),
                external_identity: None,
                label: "Аккаунт 1".into(),
                connection_ref: Some("claude-local".into()),
                lifecycle: AccountLifecycle::Active,
            },
            custom_root: None,
            secret: None,
            trigger: Trigger::Scheduled,
        },
        AccountMeta {
            alias: "Аккаунт 1".into(),
        },
    ));
    let managed: Vec<usage_storage::ManagedAccount> = state
        .storage
        .lock()
        .ok()
        .and_then(|s| s.list_managed_accounts().ok())
        .unwrap_or_default();
    for account in managed {
        if !account.enabled || matches!(account.lifecycle, AccountLifecycle::Archived) {
            continue;
        }
        let product = account.product_id.clone().unwrap_or_default();
        // Stable local scopes above already cover default roots.
        if product == "codex" || product == "claude-code" {
            let custom = account.custom_path.clone().filter(|p| !p.trim().is_empty());
            if custom.is_none() {
                continue;
            }
            out.push((
                RefreshRequest {
                    scope: ScopeKey {
                        account_id: account.id,
                        provider_id: account.provider_id.clone(),
                        product_id: product.clone(),
                    },
                    account: Account {
                        id: account.id,
                        provider_id: account.provider_id.clone(),
                        external_identity: account.external_identity.clone(),
                        label: account.label.clone(),
                        connection_ref: account.connection_ref.clone(),
                        lifecycle: account.lifecycle,
                    },
                    custom_root: custom.map(PathBuf::from),
                    secret: None,
                    trigger: Trigger::Scheduled,
                },
                AccountMeta {
                    alias: account.label.clone(),
                },
            ));
            continue;
        }
        if product == "openai-api" {
            let secret = state
                .hosts
                .keychain
                .read(
                    crate::platform::hosts::KEYCHAIN_SERVICE,
                    &account.id.to_string(),
                )
                .await
                .ok()
                .map(|s| s.expose().to_vec())
                .filter(|s| !s.is_empty());
            out.push((
                RefreshRequest {
                    scope: ScopeKey {
                        account_id: account.id,
                        provider_id: account.provider_id.clone(),
                        product_id: product.clone(),
                    },
                    account: Account {
                        id: account.id,
                        provider_id: account.provider_id.clone(),
                        external_identity: account.external_identity.clone(),
                        label: account.label.clone(),
                        connection_ref: account.connection_ref.clone(),
                        lifecycle: account.lifecycle,
                    },
                    custom_root: None,
                    secret,
                    trigger: Trigger::Scheduled,
                },
                AccountMeta {
                    alias: account.label.clone(),
                },
            ));
        }
    }
    out
}

#[derive(Clone)]
pub struct AccountMeta {
    pub alias: String,
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
    let mut providers: Vec<ProviderDto> = outcomes
        .iter()
        .map(|outcome| outcome_to_dto(outcome, metas))
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
    let (overview, model_breakdown, account_breakdown, project_breakdown) = build_overview(state);
    let costs_by_period = build_costs_by_period(state, &providers);
    // Local-only sources work without internet; the flag reflects the
    // observed network state, never a hard-coded constant.
    let offline = matches!(state.network.state(), usage_host::OnlineState::Offline);
    AppSnapshot {
        mode: "live".into(),
        offline,
        providers,
        overview,
        model_breakdown,
        account_breakdown,
        project_breakdown,
        costs_by_period,
        next_refresh_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
    }
}

fn outcome_to_dto(outcome: &RefreshOutcome, metas: &HashMap<ScopeKey, AccountMeta>) -> ProviderDto {
    let descriptor = find_descriptor(&outcome.scope.provider_id, &outcome.scope.product_id);
    let (provider_name, product_name) = descriptor
        .map(|d| (d.provider_name, d.product_name))
        .unwrap_or(("Unknown", "Unknown"));
    let alias = metas
        .get(&outcome.scope)
        .map(|m| m.alias.as_str())
        .unwrap_or("Аккаунт 1");
    match &outcome.snapshot {
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
    }
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
    let today = Local::now().date_naive();
    let start_local = local_midnight(today);
    let end_local = local_midnight(today.succ_opt().unwrap_or(today));
    let (start, end) = (
        start_local.with_timezone(&Utc),
        end_local.with_timezone(&Utc),
    );
    let Ok(storage) = state.storage.lock() else {
        return;
    };
    if let Ok(tokens) =
        storage.tokens_for_account_between(account_id, &provider.product_id, start, end)
    {
        if tokens > 0 {
            provider.tokens_today = Some(tokens);
        }
    }
    if let Ok(costs) = storage.cost_records_between(start, end) {
        let mut reported: HashMap<String, rust_decimal::Decimal> = HashMap::new();
        let mut estimated: HashMap<String, rust_decimal::Decimal> = HashMap::new();
        for record in costs
            .iter()
            .filter(|r| r.account_id == account_id && r.product_id == provider.product_id)
        {
            let target = if record.kind == CostKind::Reported {
                &mut reported
            } else {
                &mut estimated
            };
            *target
                .entry(record.currency.clone())
                .or_insert(rust_decimal::Decimal::ZERO) += record.amount_decimal;
        }
        for currency in reported.keys().cloned().collect::<Vec<_>>() {
            estimated.remove(&currency);
        }
        if reported.len() == 1 {
            let (currency, amount) = reported.into_iter().next().unwrap();
            provider.reported_cost_today = Some(MoneyDto {
                amount: amount.to_string(),
                currency,
            });
        }
        if estimated.len() == 1 {
            let (currency, amount) = estimated.into_iter().next().unwrap();
            provider.estimated_cost_today = Some(MoneyDto {
                amount: amount.to_string(),
                currency,
            });
        }
    }
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

pub fn period_ranges() -> Vec<(&'static str, DateTime<Utc>, DateTime<Utc>)> {
    let today_date = Local::now().date_naive();
    let today = local_midnight(today_date);
    let tomorrow = local_midnight(today_date.succ_opt().unwrap_or(today_date));
    let yesterday = local_midnight(today_date.pred_opt().unwrap_or(today_date));
    let week = local_midnight(
        today_date
            .checked_sub_signed(Duration::days(6))
            .unwrap_or(today_date),
    );
    let month = local_midnight(
        today_date
            .checked_sub_signed(Duration::days(29))
            .unwrap_or(today_date),
    );
    vec![
        (
            "today",
            today.with_timezone(&Utc),
            tomorrow.with_timezone(&Utc),
        ),
        (
            "yesterday",
            yesterday.with_timezone(&Utc),
            today.with_timezone(&Utc),
        ),
        (
            "7days",
            week.with_timezone(&Utc),
            tomorrow.with_timezone(&Utc),
        ),
        (
            "30days",
            month.with_timezone(&Utc),
            tomorrow.with_timezone(&Utc),
        ),
    ]
}

#[allow(clippy::type_complexity)]
pub fn build_overview(
    state: &AppState,
) -> (
    HashMap<String, Vec<OverviewSegment>>,
    HashMap<String, Vec<OverviewSegment>>,
    HashMap<String, Vec<OverviewSegment>>,
    HashMap<String, Vec<OverviewSegment>>,
) {
    let mut result = HashMap::new();
    let mut models: HashMap<String, Vec<OverviewSegment>> = HashMap::new();
    let mut accounts: HashMap<String, Vec<OverviewSegment>> = HashMap::new();
    let mut projects: HashMap<String, Vec<OverviewSegment>> = HashMap::new();
    if let Ok(storage) = state.storage.lock() {
        for (key, start, end) in period_ranges() {
            let rows = storage.token_totals_between(start, end).unwrap_or_default();
            result.insert(
                key.to_string(),
                rows.into_iter()
                    .map(|(product, value)| OverviewSegment {
                        label: match product.as_str() {
                            "codex" => "Codex".into(),
                            "claude-code" => "Claude Code".into(),
                            other => other.into(),
                        },
                        value,
                        color: match product.as_str() {
                            "codex" => "#ff7a5c",
                            "claude-code" => "#b895ff",
                            _ => "#46c9a7",
                        }
                        .into(),
                    })
                    .collect(),
            );
            let model_rows = storage.model_totals_between(start, end).unwrap_or_default();
            models.insert(
                key.to_string(),
                model_rows
                    .into_iter()
                    .map(|(model, value)| OverviewSegment {
                        label: model
                            .filter(|m| !m.trim().is_empty())
                            .unwrap_or_else(|| "Модель не определена".into()),
                        value,
                        color: "#8ea2ff".into(),
                    })
                    .collect(),
            );
            let account_rows = storage
                .account_product_totals_between(start, end)
                .unwrap_or_default();
            accounts.insert(
                key.to_string(),
                account_rows
                    .into_iter()
                    .map(|(alias, _id, product, value)| OverviewSegment {
                        label: format!("{alias} · {product}"),
                        value,
                        color: "#e0a44f".into(),
                    })
                    .collect(),
            );
            let project_rows = storage
                .project_totals_between(start, end)
                .unwrap_or_default();
            projects.insert(
                key.to_string(),
                project_rows
                    .into_iter()
                    .map(|(scope, value)| OverviewSegment {
                        label: scope
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| "Без проекта".into()),
                        value,
                        color: "#5fb3a1".into(),
                    })
                    .collect(),
            );
        }
    }
    (result, models, accounts, projects)
}

/// Per-period reported/estimated cost summaries. The period selector
/// drives these backend queries — the UI never relabels one range as
/// another.
pub fn build_costs_by_period(
    state: &AppState,
    providers: &[ProviderDto],
) -> HashMap<String, Vec<ProviderCostDto>> {
    let mut out: HashMap<String, Vec<ProviderCostDto>> = HashMap::new();
    let Ok(storage) = state.storage.lock() else {
        return out;
    };
    for (key, start, end) in period_ranges() {
        let costs = storage.cost_records_between(start, end).unwrap_or_default();
        let mut rows = vec![];
        for provider in providers {
            let Ok(account_id) = provider.account_id.parse::<Uuid>() else {
                continue;
            };
            let mut reported: HashMap<String, rust_decimal::Decimal> = HashMap::new();
            let mut estimated: HashMap<String, rust_decimal::Decimal> = HashMap::new();
            for record in costs
                .iter()
                .filter(|r| r.account_id == account_id && r.product_id == provider.product_id)
            {
                let target = if record.kind == CostKind::Reported {
                    &mut reported
                } else {
                    &mut estimated
                };
                *target
                    .entry(record.currency.clone())
                    .or_insert(rust_decimal::Decimal::ZERO) += record.amount_decimal;
            }
            for currency in reported.keys().cloned().collect::<Vec<_>>() {
                estimated.remove(&currency);
            }
            if reported.is_empty() && estimated.is_empty() {
                continue;
            }
            let mut reported_list: Vec<MoneyDto> = reported
                .into_iter()
                .map(|(currency, amount)| MoneyDto {
                    amount: amount.to_string(),
                    currency,
                })
                .collect();
            reported_list.sort_by(|a, b| a.currency.cmp(&b.currency));
            let mut estimated_list: Vec<MoneyDto> = estimated
                .into_iter()
                .map(|(currency, amount)| MoneyDto {
                    amount: amount.to_string(),
                    currency,
                })
                .collect();
            estimated_list.sort_by(|a, b| a.currency.cmp(&b.currency));
            rows.push(ProviderCostDto {
                account_id: provider.account_id.clone(),
                product_id: provider.product_id.clone(),
                label: format!("{} · {}", provider.provider_name, provider.alias),
                reported: reported_list,
                estimated: estimated_list,
            });
        }
        out.insert(key.to_string(), rows);
    }
    out
}

/// Scoped history import across descriptor roots plus user custom
/// paths. Warnings and counters feed import diagnostics.
pub async fn import_all_history(state: &AppState) {
    use usage_providers::{claude::ClaudeLocalAdapter, codex::CodexLocalAdapter};

    let managed: Vec<usage_storage::ManagedAccount> = state
        .storage
        .lock()
        .ok()
        .and_then(|s| s.list_managed_accounts().ok())
        .unwrap_or_default();
    for descriptor in usage_providers::descriptor::all_descriptors()
        .iter()
        .filter(|d| d.account_model == AccountModel::LocalClient)
    {
        let mut roots = product_roots(descriptor.local_dir_env, descriptor.local_dir_name, None);
        for account in managed.iter().filter(|a| {
            a.enabled
                && a.product_id.as_deref() == Some(descriptor.product_id)
                && a.custom_path
                    .as_deref()
                    .is_some_and(|p| !p.trim().is_empty())
        }) {
            if let Some(custom) = account.custom_path.clone() {
                let path = PathBuf::from(&custom);
                if !roots.contains(&path) {
                    roots.push(path);
                }
            }
        }
        let Some(glob_pattern) = descriptor.local_glob else {
            continue;
        };
        // Stable local account attribution for default roots; custom
        // roots attribute to their managed account histories below.
        let (adapter, account): (Box<dyn usage_core::ProviderAdapter>, Account) =
            match descriptor.product_id {
                "codex" => (
                    Box::new(CodexLocalAdapter::default()),
                    Account {
                        id: stable_local_account("codex"),
                        provider_id: "openai".into(),
                        external_identity: None,
                        label: "Аккаунт 1".into(),
                        connection_ref: Some("codex-local".into()),
                        lifecycle: AccountLifecycle::Active,
                    },
                ),
                _ => (
                    Box::new(ClaudeLocalAdapter::default()),
                    Account {
                        id: stable_local_account("claude-code"),
                        provider_id: "anthropic".into(),
                        external_identity: None,
                        label: "Аккаунт 1".into(),
                        connection_ref: Some("claude-local".into()),
                        lifecycle: AccountLifecycle::Active,
                    },
                ),
            };
        let report = usage_runtime::import_product(
            &state.hosts,
            &state.storage,
            adapter.as_ref(),
            &account,
            roots,
            glob_pattern,
            1,
        )
        .await;
        if let Ok(mut stats) = state.import_stats.lock() {
            stats.insert(descriptor.product_id.to_string(), report);
        }
    }
    // Retention covers every category; active checkpoints are kept.
    if let Ok(mut storage) = state.storage.lock() {
        let days = read_retention_days(state);
        let _ = storage.prune_retention(Utc::now() - Duration::days(days));
    }
}

fn read_retention_days(state: &AppState) -> i64 {
    crate::commands::settings::read_settings(state)
        .retention_days
        .max(30) as i64
}

/// Primary tray metric: minimum remaining percent across fresh quotas.
pub fn primary_remaining(providers: &[ProviderDto]) -> Option<f64> {
    providers
        .iter()
        .filter(|p| p.freshness == usage_core::Freshness::Fresh)
        .flat_map(|p| p.quotas.iter())
        .filter_map(|q| q.remaining_percent)
        .fold(None, |min: Option<f64>, value| {
            Some(min.map_or(value, |m: f64| m.min(value)))
        })
}

pub fn coordinator_of(state: &AppState) -> Arc<Coordinator> {
    Arc::clone(&state.coordinator)
}
