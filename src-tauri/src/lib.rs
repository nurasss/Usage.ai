use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc};
use glob::glob;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::Mutex,
    time::{Duration as StdDuration, UNIX_EPOCH},
};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, PhysicalPosition, Runtime};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use usage_core::{
    Account, AccountLifecycle, Capability, ConnectionState, CostKind, Coverage, Freshness,
    MoneyBalance, ProviderAdapter, ProviderError,
};
use usage_providers::{
    anonymous_path_identity, claude::ClaudeLocalAdapter, codex::CodexLocalAdapter,
    discovery::DiscoveryOnlyAdapter, openai_api::OpenAiApiAdapter,
};
use usage_storage::{Budget, ImportCheckpoint, ManagedAccount, Storage};
use uuid::Uuid;

const SOURCE_TIMEOUT: StdDuration = StdDuration::from_secs(15);
const FALLBACK_COOLDOWN: StdDuration = StdDuration::from_secs(300);

#[derive(Clone, Debug)]
struct RefreshMeta {
    last_attempt: Option<DateTime<Utc>>,
    last_success: Option<DateTime<Utc>>,
    last_observed: Option<DateTime<Utc>>,
    cooldown_until: Option<DateTime<Utc>>,
    error_code: Option<String>,
    connector_version: String,
    parser_version: String,
    last_state: Option<ConnectionState>,
    quota_windows: HashMap<String, (Option<String>, Option<f64>)>,
    quota_observations: HashMap<String, Vec<(DateTime<Utc>, f64)>>,
}

impl Default for RefreshMeta {
    fn default() -> Self {
        Self {
            last_attempt: None,
            last_success: None,
            last_observed: None,
            cooldown_until: None,
            error_code: None,
            connector_version: "adapter-v1".into(),
            parser_version: "parser-v1".into(),
            last_state: None,
            quota_windows: HashMap::new(),
            quota_observations: HashMap::new(),
        }
    }
}

fn cooldown_remaining(meta: &HashMap<String, RefreshMeta>, key: &str) -> Option<StdDuration> {
    meta.get(key)?.cooldown_until.and_then(|until| {
        let ms = (until - Utc::now()).num_milliseconds();
        if ms <= 0 {
            None
        } else {
            Some(StdDuration::from_millis(ms as u64))
        }
    })
}

struct AppState {
    storage: Mutex<Storage>,
    cached: Mutex<Option<AppSnapshot>>,
    meta: Mutex<HashMap<String, RefreshMeta>>,
    settings_path: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    launch_at_login: bool,
    refresh_interval_minutes: Option<u64>,
    menu_bar_mode: String,
    global_shortcut: String,
    theme: String,
    retention_days: u64,
    quota_warning_percent: u8,
    notifications_enabled: bool,
    quiet_hours_start: Option<String>,
    quiet_hours_end: Option<String>,
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
struct AppSnapshot {
    mode: String,
    offline: bool,
    providers: Vec<ProviderDto>,
    overview: HashMap<String, Vec<OverviewSegment>>,
    #[serde(default)]
    model_breakdown: HashMap<String, Vec<OverviewSegment>>,
    #[serde(default)]
    account_breakdown: HashMap<String, Vec<OverviewSegment>>,
    #[serde(default)]
    project_breakdown: HashMap<String, Vec<OverviewSegment>>,
    next_refresh_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderDto {
    account_id: String,
    provider_id: String,
    product_id: String,
    provider_name: String,
    product_name: String,
    alias: String,
    plan_label: Option<String>,
    connection_state: ConnectionState,
    freshness: Freshness,
    coverage: Coverage,
    fetched_at: String,
    observed_at: Option<String>,
    capabilities: Vec<String>,
    quotas: Vec<QuotaDto>,
    tokens_today: Option<u64>,
    reported_cost_today: Option<MoneyDto>,
    estimated_cost_today: Option<MoneyDto>,
    #[serde(default)]
    balances: Vec<MoneyDto>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuotaDto {
    pool_id: String,
    window_id: Option<String>,
    name: String,
    remaining_percent: Option<f64>,
    used: Option<u64>,
    limit: Option<u64>,
    unit: String,
    resets_at: Option<String>,
    window_start: Option<String>,
    window_kind: String,
    source: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct MoneyDto {
    amount: String,
    currency: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct OverviewSegment {
    label: String,
    value: u64,
    color: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsDto {
    provider: String,
    product: String,
    account_alias: String,
    connection_state: ConnectionState,
    last_refresh_attempt: Option<String>,
    last_successful_refresh: Option<String>,
    last_data_observed_at: Option<String>,
    freshness: Freshness,
    coverage: Coverage,
    status_class: Option<String>,
    connector_version: String,
    parser_version: String,
    schema_fingerprint: Option<String>,
    capabilities_detected: Vec<String>,
    cooldown_until: Option<String>,
    last_safe_error_code: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountDto {
    id: String,
    provider_id: String,
    product_id: Option<String>,
    label: String,
    lifecycle: String,
    connection_ref: Option<String>,
    enabled: bool,
    custom_path: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BudgetDto {
    account_id: String,
    product_id: String,
    currency: String,
    amount: String,
    period: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    update_channel: Option<String>,
    updates_enabled: bool,
}

#[tauri::command]
async fn get_snapshot(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<AppSnapshot, String> {
    if let Some(value) = state.cached.lock().map_err(|_| "state_lock")?.clone() {
        return Ok(value);
    }
    if let Ok(storage) = state.storage.lock() {
        if let Ok(Some(json)) = storage.cache("app_snapshot") {
            if let Ok(value) = serde_json::from_str::<AppSnapshot>(&json) {
                *state.cached.lock().map_err(|_| "state_lock")? = Some(value.clone());
                return Ok(value);
            }
        }
    }
    import_known_history(state.inner()).await;
    let value = build_live_snapshot(state.inner()).await;
    cache_snapshot(state.inner(), &value);
    deliver_notifications(&app, state.inner(), &value);
    *state.cached.lock().map_err(|_| "state_lock")? = Some(value.clone());
    Ok(value)
}

#[tauri::command]
async fn refresh_all(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<AppSnapshot, String> {
    // Manual refresh never bypasses an active per-account cooldown:
    // build_live_snapshot skips sources whose cooldown_until is in the future
    // and marks them Cooldown instead of re-hitting the source.
    import_known_history(state.inner()).await;
    let value = build_live_snapshot(state.inner()).await;
    cache_snapshot(state.inner(), &value);
    deliver_notifications(&app, state.inner(), &value);
    *state.cached.lock().map_err(|_| "state_lock")? = Some(value.clone());
    Ok(value)
}

fn cache_snapshot(state: &AppState, value: &AppSnapshot) {
    if let Ok(json) = serde_json::to_string(value) {
        if let Ok(mut storage) = state.storage.lock() {
            let _ = storage.set_cache("app_snapshot", &json);
        }
    }
}

fn deliver_notifications(app: &tauri::AppHandle, state: &AppState, value: &AppSnapshot) {
    let settings = read_settings(state);
    if !settings.notifications_enabled || quiet_now(&settings) {
        return;
    }
    let auth_states = [
        ConnectionState::AuthenticationRequired,
        ConnectionState::SessionExpired,
        ConnectionState::PermissionDenied,
        ConnectionState::InsufficientScope,
    ];
    for provider in &value.providers {
        let account_key = format!(
            "{}/{}/{}",
            provider.provider_id, provider.product_id, provider.account_id
        );
        // Disconnect / auth transitions fire even on stale snapshots: the
        // error state itself is the signal, and it is deduplicated per state.
        if auth_states.contains(&provider.connection_state) {
            let state_code = format!("{:?}", provider.connection_state);
            let is_new = state
                .meta
                .lock()
                .ok()
                .map(|meta| {
                    meta.get(&account_key).and_then(|m| m.last_state.clone())
                        != Some(provider.connection_state.clone())
                })
                .unwrap_or(true);
            if is_new {
                let should_send = state
                    .storage
                    .lock()
                    .ok()
                    .and_then(|mut storage| {
                        storage
                            .mark_notification_delivered(
                                &provider.account_id,
                                &format!("connection:{state_code}"),
                                1,
                                "current",
                            )
                            .ok()
                    })
                    .unwrap_or(false);
                if should_send {
                    let _ = app
                        .notification()
                        .builder()
                        .title(format!(
                            "{} · {}",
                            provider.provider_name, provider.product_name
                        ))
                        .body(format!(
                            "{} — откройте исходный клиент или проверьте подключение",
                            state_label_for_notify(&provider.connection_state)
                        ))
                        .show();
                }
            }
        }
        if let Ok(mut meta) = state.meta.lock() {
            if let Some(entry) = meta.get_mut(&account_key) {
                entry.last_state = Some(provider.connection_state.clone());
            }
        }
        if provider.freshness != Freshness::Fresh {
            continue;
        }
        for quota in &provider.quotas {
            let Some(remaining) = quota.remaining_percent else {
                continue;
            };
            let Some(window) = quota.window_id.as_deref() else {
                continue;
            };
            let pool_key = format!("{}:{}", account_key, quota.pool_id);
            // Confirmed reset: window id changed AND remaining jumped up.
            // Countdown expiry alone never produces this notification.
            let reset_confirmed = state
                .meta
                .lock()
                .ok()
                .map(|meta| {
                    meta.get(&account_key)
                        .and_then(|m| m.quota_windows.get(&pool_key))
                        .map(|(prev_window, prev_remaining)| {
                            prev_window.as_deref() != Some(window)
                                && prev_remaining.is_some_and(|prev| remaining > prev)
                        })
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if reset_confirmed {
                let should_send = state
                    .storage
                    .lock()
                    .ok()
                    .and_then(|mut storage| {
                        storage
                            .mark_notification_delivered(
                                &provider.account_id,
                                &format!("quota-reset:{}", quota.pool_id),
                                0,
                                window,
                            )
                            .ok()
                    })
                    .unwrap_or(false);
                if should_send {
                    let _ = app
                        .notification()
                        .builder()
                        .title(format!("{} · {}", provider.provider_name, quota.name))
                        .body("Лимит восстановлен — подтверждено новым наблюдением".to_string())
                        .show();
                }
            }
            if let Ok(mut meta) = state.meta.lock() {
                if let Some(entry) = meta.get_mut(&account_key) {
                    entry.quota_windows.insert(
                        pool_key.clone(),
                        (Some(window.to_string()), Some(remaining)),
                    );
                    let obs = entry
                        .quota_observations
                        .entry(pool_key.clone())
                        .or_default();
                    obs.push((Utc::now(), 100.0 - remaining));
                    if obs.len() > 2 {
                        obs.remove(0);
                    }
                }
            }
            // Linear pace projection, only for fixed windows with two fresh
            // observations and a known reset. Rolling windows never project.
            if quota.window_kind == "fixed" {
                if let (Some(resets_at), Some(start)) =
                    (quota.resets_at.as_deref(), quota.window_start.as_deref())
                {
                    let projection = state.meta.lock().ok().and_then(|meta| {
                        let obs = meta.get(&account_key)?.quota_observations.get(&pool_key)?;
                        if obs.len() < 2 {
                            return None;
                        }
                        let (t1, u1) = obs[obs.len() - 2];
                        let (t2, u2) = obs[obs.len() - 1];
                        let dt = (t2 - t1).num_seconds();
                        if dt <= 0 || u2 <= u1 {
                            return None;
                        }
                        let rate = (u2 - u1) / dt as f64;
                        let remaining_use = 100.0 - u2;
                        let secs_left = (remaining_use / rate) as i64;
                        Some(t2 + Duration::seconds(secs_left))
                    });
                    if let Some(exhaust_at) = projection {
                        let reset_ok = resets_at.parse::<DateTime<Utc>>().ok();
                        let start_ok = start.parse::<DateTime<Utc>>().ok().is_some();
                        if start_ok && reset_ok.is_some_and(|reset| exhaust_at < reset) {
                            let should_send = state
                                .storage
                                .lock()
                                .ok()
                                .and_then(|mut storage| {
                                    storage
                                        .mark_notification_delivered(
                                            &provider.account_id,
                                            &format!("pace:{}", quota.pool_id),
                                            0,
                                            window,
                                        )
                                        .ok()
                                })
                                .unwrap_or(false);
                            if should_send {
                                let _ = app
                                    .notification()
                                    .builder()
                                    .title(format!("{} · {}", provider.provider_name, quota.name))
                                    .body(
                                        "Темп выше лимита: может исчерпаться до сброса".to_string(),
                                    )
                                    .show();
                            }
                        }
                    }
                }
            }
            for used_threshold in [100u8, 90, 80] {
                if remaining > f64::from(100 - used_threshold) {
                    continue;
                }
                let key_metric = format!("quota:{}", quota.pool_id);
                let should_send = {
                    let Ok(mut storage) = state.storage.lock() else {
                        return;
                    };
                    storage
                        .mark_notification_delivered(
                            &provider.account_id,
                            &key_metric,
                            used_threshold,
                            window,
                        )
                        .unwrap_or(false)
                };
                if should_send {
                    let _ = app
                        .notification()
                        .builder()
                        .title(format!("{} · {}", provider.provider_name, quota.name))
                        .body(format!(
                            "Использовано {used_threshold}% · осталось {:.0}%",
                            remaining
                        ))
                        .show();
                }
                break;
            }
        }
    }
    // Spend over budget: reported month-to-date cost vs user budgets.
    // Different currencies are compared separately, never converted.
    let month_start = local_midnight(
        Local::now()
            .date_naive()
            .pred_opt()
            .map(|d| d.with_day(1).unwrap_or(d))
            .unwrap_or_else(|| Local::now().date_naive()),
    );
    let month_key = Local::now().format("%Y-%m").to_string();
    let budgets: Vec<Budget> = state
        .storage
        .lock()
        .ok()
        .and_then(|s| s.list_budgets().ok())
        .unwrap_or_default();
    for budget in budgets {
        let spent: Option<rust_decimal::Decimal> = state.storage.lock().ok().and_then(|s| {
            s.cost_records_between(month_start.with_timezone(&Utc), Utc::now())
                .ok()
                .map(|records| {
                    records
                        .iter()
                        .filter(|r| {
                            r.account_id == budget.account_id
                                && r.product_id == budget.product_id
                                && r.currency == budget.currency
                                && r.kind == CostKind::Reported
                        })
                        .map(|r| r.amount_decimal)
                        .sum()
                })
        });
        if spent.is_some_and(|total| total >= budget.amount_decimal) {
            let should_send = state
                .storage
                .lock()
                .ok()
                .and_then(|mut storage| {
                    storage
                        .mark_notification_delivered(
                            &budget.account_id.to_string(),
                            &format!("budget:{}:{}", budget.product_id, budget.currency),
                            100,
                            &month_key,
                        )
                        .ok()
                })
                .unwrap_or(false);
            if should_send {
                let _ = app
                    .notification()
                    .builder()
                    .title("Бюджет превышен".to_string())
                    .body(format!(
                        "{} · {} {} — фактические расходы за месяц",
                        budget.product_id, budget.amount_decimal, budget.currency
                    ))
                    .show();
            }
        }
    }
}

fn state_label_for_notify(state: &ConnectionState) -> &'static str {
    match state {
        ConnectionState::AuthenticationRequired => "Нужна авторизация",
        ConnectionState::SessionExpired => "Сессия истекла",
        ConnectionState::PermissionDenied => "Нет разрешения",
        ConnectionState::InsufficientScope => "Недостаточно прав",
        _ => "Подключение требует внимания",
    }
}
fn quiet_now(settings: &AppSettings) -> bool {
    let (Some(start), Some(end)) = (
        settings.quiet_hours_start.as_deref(),
        settings.quiet_hours_end.as_deref(),
    ) else {
        return false;
    };
    let (Ok(start), Ok(end)) = (
        NaiveTime::parse_from_str(start, "%H:%M"),
        NaiveTime::parse_from_str(end, "%H:%M"),
    ) else {
        return false;
    };
    let now = Local::now().time();
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

#[tauri::command]
fn export_history(
    format: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let storage = state.storage.lock().map_err(|_| "storage_lock")?;
    let content = match format.as_str() {
        "csv" => storage.export_csv(),
        "json" => storage.export_json(),
        _ => return Err("unsupported_export_format".into()),
    }
    .map_err(|_| "export_failed")?;
    let dir = app
        .path()
        .document_dir()
        .or_else(|_| app.path().download_dir())
        .map_err(|_| "export_directory_unavailable")?;
    let file = dir.join(format!(
        "Usage.ai-history-{}.{}",
        Local::now().format("%Y-%m-%d-%H%M%S"),
        format
    ));
    std::fs::write(&file, content).map_err(|_| "export_write_failed")?;
    Ok(format!("Экспорт сохранён: {}", file.display()))
}

#[tauri::command]
fn get_diagnostics(state: tauri::State<'_, AppState>) -> Vec<DiagnosticsDto> {
    let cached = state.cached.lock().ok().and_then(|c| c.clone());
    let meta = state
        .meta
        .lock()
        .ok()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let mut out = vec![];
    if let Some(snapshot) = cached {
        for provider in &snapshot.providers {
            let key = format!(
                "{}/{}/{}",
                provider.provider_id, provider.product_id, provider.account_id
            );
            let entry = meta.get(&key).cloned().unwrap_or_default();
            out.push(DiagnosticsDto {
                provider: provider.provider_id.clone(),
                product: provider.product_id.clone(),
                account_alias: provider.alias.clone(),
                connection_state: provider.connection_state.clone(),
                last_refresh_attempt: entry.last_attempt.map(|d| d.to_rfc3339()),
                last_successful_refresh: entry.last_success.map(|d| d.to_rfc3339()),
                last_data_observed_at: provider.observed_at.clone(),
                freshness: provider.freshness.clone(),
                coverage: provider.coverage,
                status_class: status_class(&provider.connection_state),
                connector_version: entry.connector_version,
                parser_version: entry.parser_version,
                schema_fingerprint: Some(
                    provider
                        .quotas
                        .first()
                        .map(|q| q.source.clone())
                        .unwrap_or_else(|| format!("{}-local-v1", provider.product_id)),
                ),
                capabilities_detected: provider.capabilities.clone(),
                cooldown_until: entry.cooldown_until.map(|d| d.to_rfc3339()),
                last_safe_error_code: entry.error_code,
            });
        }
    }
    out
}

fn status_class(state: &ConnectionState) -> Option<String> {
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

#[tauri::command]
fn export_diagnostics(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    // Separate whitelist-only format, never mixed with history export.
    let diagnostics = get_diagnostics(state);
    let content = serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": 1,
        "exportedAt": Utc::now().to_rfc3339(),
        "diagnostics": diagnostics,
    }))
    .map_err(|_| "export_failed")?;
    let dir = app
        .path()
        .document_dir()
        .or_else(|_| app.path().download_dir())
        .map_err(|_| "export_directory_unavailable")?;
    let file = dir.join(format!(
        "Usage.ai-diagnostics-{}.json",
        Local::now().format("%Y-%m-%d-%H%M%S")
    ));
    std::fs::write(&file, content).map_err(|_| "export_write_failed")?;
    Ok(format!("Диагностика сохранена: {}", file.display()))
}

#[tauri::command]
fn list_accounts(state: tauri::State<'_, AppState>) -> Result<Vec<AccountDto>, String> {
    let storage = state.storage.lock().map_err(|_| "storage_lock")?;
    Ok(storage
        .list_managed_accounts()
        .map_err(|_| "accounts_unavailable")?
        .into_iter()
        .map(|a| AccountDto {
            id: a.id.to_string(),
            provider_id: a.provider_id,
            product_id: a.product_id,
            label: a.label,
            lifecycle: format!("{:?}", a.lifecycle),
            connection_ref: a.connection_ref,
            enabled: a.enabled,
            custom_path: a.custom_path,
        })
        .collect())
}

fn managed_account_to_dto(a: &ManagedAccount) -> AccountDto {
    AccountDto {
        id: a.id.to_string(),
        provider_id: a.provider_id.clone(),
        product_id: a.product_id.clone(),
        label: a.label.clone(),
        lifecycle: format!("{:?}", a.lifecycle),
        connection_ref: a.connection_ref.clone(),
        enabled: a.enabled,
        custom_path: a.custom_path.clone(),
    }
}

fn read_secret(account_id: &str) -> Option<String> {
    // Keychain only; the secret lives in memory for the duration of one
    // request and is never written to SQLite, logs, cache or export.
    keyring::Entry::new("com.nurasss.usageai", account_id)
        .ok()?
        .get_password()
        .ok()
        .filter(|s| !s.is_empty())
}

fn supported_products() -> &'static [(&'static str, &'static str, &'static str, &'static str)] {
    &[
        ("openai", "codex", "OpenAI", "Codex"),
        ("anthropic", "claude-code", "Anthropic", "Claude Code"),
        ("openai", "openai-api", "OpenAI", "API"),
        ("google", "antigravity", "Google", "Antigravity"),
        ("zai", "glm-coding", "Z.ai", "GLM Coding Plan"),
        ("opencode", "zen", "OpenCode", "Zen"),
    ]
}

#[tauri::command]
fn add_account(
    provider_id: String,
    product_id: String,
    alias: String,
    secret: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<AccountDto, String> {
    if !supported_products()
        .iter()
        .any(|(pid, prod, _, _)| *pid == provider_id && *prod == product_id)
    {
        return Err("unsupported_product".into());
    }
    let alias = alias.trim();
    if alias.is_empty() || alias.len() > 64 {
        return Err("invalid_alias".into());
    }
    let needs_secret = product_id == "openai-api";
    if needs_secret && secret.as_deref().unwrap_or("").trim().is_empty() {
        return Err("secret_required".into());
    }
    if let Some(secret) = secret.as_deref() {
        if secret.len() > 4096 {
            return Err("invalid_secret_input".into());
        }
    }
    let id = Uuid::new_v4();
    let external_identity = if product_id == "openai-api" {
        Some(OpenAiApiAdapter::default().identity_for(secret.as_deref().unwrap_or_default()))
    } else {
        None
    };
    let connection_ref = if needs_secret {
        Some("keychain:com.nurasss.usageai".into())
    } else if product_id == "codex" {
        Some("codex-local".into())
    } else if product_id == "claude-code" {
        Some("claude-local".into())
    } else {
        None
    };
    let managed = ManagedAccount {
        id,
        provider_id: provider_id.clone(),
        product_id: Some(product_id.clone()),
        external_identity,
        label: alias.into(),
        connection_ref,
        lifecycle: AccountLifecycle::Active,
        enabled: true,
        custom_path: None,
    };
    if needs_secret {
        keyring::Entry::new("com.nurasss.usageai", &id.to_string())
            .map_err(|_| "keychain_unavailable")?
            .set_password(secret.unwrap_or_default().trim())
            .map_err(|_| "keychain_write_denied".to_string())?;
    }
    state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .create_managed_account(&managed)
        .map_err(|_| "account_create_failed")?;
    Ok(managed_account_to_dto(&managed))
}

#[tauri::command]
fn update_account(
    account_id: String,
    alias: Option<String>,
    enabled: Option<bool>,
    custom_path: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<AccountDto, String> {
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    if let Some(alias) = alias.as_deref() {
        if alias.trim().is_empty() || alias.len() > 64 {
            return Err("invalid_alias".into());
        }
    }
    if let Some(path) = custom_path.as_deref() {
        if path.len() > 1024 {
            return Err("invalid_custom_path".into());
        }
    }
    let mut storage = state.storage.lock().map_err(|_| "storage_lock")?;
    let custom = custom_path.map(|p| {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    storage
        .update_managed_account(
            id,
            alias.as_deref(),
            enabled,
            custom.as_ref().map(|o| o.as_deref()),
        )
        .map_err(|_| "account_update_failed")?;
    storage
        .get_account(id)
        .map_err(|_| "accounts_unavailable")?
        .map(|a| managed_account_to_dto(&a))
        .ok_or_else(|| "account_not_found".into())
}

#[tauri::command]
async fn test_connection(
    account_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DiagnosticsDto, String> {
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    let managed = state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .get_account(id)
        .map_err(|_| "accounts_unavailable")?
        .ok_or_else(|| "account_not_found".to_string())?;
    let product = managed.product_id.clone().unwrap_or_default();
    let probe_account = Account {
        id: managed.id,
        provider_id: managed.provider_id.clone(),
        external_identity: managed.external_identity.clone(),
        label: managed.label.clone(),
        connection_ref: managed.connection_ref.clone(),
        lifecycle: managed.lifecycle.clone(),
    };
    let now = Utc::now();
    let (
        connection_state,
        freshness,
        coverage,
        capabilities,
        quotas,
        error_code,
        connector,
        parser,
    ) = if product == "openai-api" {
        let Some(secret) = read_secret(&managed.id.to_string()) else {
            return Err("authentication_required".into());
        };
        match tokio::time::timeout(
            SOURCE_TIMEOUT,
            OpenAiApiAdapter::default().refresh_with_key(&probe_account, &secret, SOURCE_TIMEOUT),
        )
        .await
        {
            Ok(Ok(snapshot)) => (
                ConnectionState::Connected,
                snapshot.freshness,
                snapshot.coverage,
                snapshot
                    .capabilities
                    .into_iter()
                    .map(capability_name)
                    .collect(),
                snapshot.quotas.len(),
                None,
                usage_providers::openai_api::OPENAI_API_CONNECTOR_VERSION.to_string(),
                usage_providers::openai_api::OPENAI_COSTS_PARSER_VERSION.to_string(),
            ),
            Ok(Err(error)) => (
                map_error(&error),
                Freshness::Unknown,
                Coverage::Unknown,
                vec![],
                0,
                Some(safe_error_code(&error)),
                usage_providers::openai_api::OPENAI_API_CONNECTOR_VERSION.to_string(),
                usage_providers::openai_api::OPENAI_COSTS_PARSER_VERSION.to_string(),
            ),
            Err(_) => (
                ConnectionState::NetworkError,
                Freshness::Unknown,
                Coverage::Unknown,
                vec![],
                0,
                Some("source_timeout".to_string()),
                usage_providers::openai_api::OPENAI_API_CONNECTOR_VERSION.to_string(),
                usage_providers::openai_api::OPENAI_COSTS_PARSER_VERSION.to_string(),
            ),
        }
    } else if product == "codex" {
        match CodexLocalAdapter::default().refresh(&probe_account).await {
            Ok(snapshot) => (
                ConnectionState::Connected,
                snapshot.freshness,
                snapshot.coverage,
                snapshot
                    .capabilities
                    .into_iter()
                    .map(capability_name)
                    .collect(),
                snapshot.quotas.len(),
                None,
                "codex-local-connector-v1".to_string(),
                "codex-local-parser-v1".to_string(),
            ),
            Err(error) => (
                map_error(&error),
                Freshness::Unknown,
                Coverage::LocalClientOnly,
                vec![],
                0,
                Some(safe_error_code(&error)),
                "codex-local-connector-v1".to_string(),
                "codex-local-parser-v1".to_string(),
            ),
        }
    } else {
        return Err("unsupported_source".into());
    };
    let _ = (now, quotas);
    Ok(DiagnosticsDto {
        provider: managed.provider_id,
        product,
        account_alias: managed.label,
        connection_state,
        last_refresh_attempt: Some(now.to_rfc3339()),
        last_successful_refresh: None,
        last_data_observed_at: None,
        freshness,
        coverage,
        status_class: None,
        connector_version: connector,
        parser_version: parser,
        schema_fingerprint: None,
        capabilities_detected: capabilities,
        cooldown_until: None,
        last_safe_error_code: error_code,
    })
}

#[tauri::command]
fn list_budgets(state: tauri::State<'_, AppState>) -> Result<Vec<BudgetDto>, String> {
    let storage = state.storage.lock().map_err(|_| "storage_lock")?;
    Ok(storage
        .list_budgets()
        .map_err(|_| "budgets_unavailable")?
        .into_iter()
        .map(|b| BudgetDto {
            account_id: b.account_id.to_string(),
            product_id: b.product_id,
            currency: b.currency,
            amount: b.amount_decimal.to_string(),
            period: b.period,
        })
        .collect())
}

#[tauri::command]
fn save_budget(
    account_id: String,
    product_id: String,
    currency: String,
    amount: String,
    state: tauri::State<'_, AppState>,
) -> Result<BudgetDto, String> {
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    let amount = Decimal::from_str(amount.trim()).map_err(|_| "invalid_amount")?;
    let currency = currency.trim().to_ascii_uppercase();
    let budget = Budget {
        account_id: id,
        product_id: product_id.clone(),
        currency: currency.clone(),
        amount_decimal: amount,
        period: "monthly".into(),
    };
    state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .upsert_budget(&budget)
        .map_err(|_| "invalid_budget")?;
    Ok(BudgetDto {
        account_id: id.to_string(),
        product_id,
        currency,
        amount: amount.to_string(),
        period: "monthly".into(),
    })
}

#[tauri::command]
fn delete_budget(
    account_id: String,
    product_id: String,
    currency: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    Ok(state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .delete_budget(id, &product_id, &currency.to_ascii_uppercase())
        .map_err(|_| "budget_delete_failed")?)
}

#[tauri::command]
fn get_storage_status(
    state: tauri::State<'_, AppState>,
) -> Result<usage_storage::StorageStatus, String> {
    Ok(state
        .storage
        .lock()
        .map_err(|_| "storage_lock".to_string())?
        .storage_status()
        .map_err(|_| "storage_status_unavailable".to_string())?)
}

#[tauri::command]
fn get_app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        update_channel: None,
        updates_enabled: false,
    }
}

#[tauri::command]
fn remove_account(
    account_id: String,
    delete_history: bool,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    // Own Keychain secret is removed in both cases; the provider-side
    // account and foreign client credentials are never touched.
    let _ = keyring::Entry::new("com.nurasss.usageai", &account_id).map(|e| e.delete_credential());
    let mut storage = state.storage.lock().map_err(|_| "storage_lock")?;
    if delete_history {
        storage
            .delete_account_and_history(id)
            .map_err(|_| "account_delete_failed")?;
        Ok("Подключение и история удалены".into())
    } else {
        storage
            .archive_account(id)
            .map_err(|_| "account_archive_failed")?;
        Ok("Подключение отключено, история сохранена как архив".into())
    }
}

#[tauri::command]
fn store_secret(account_id: String, secret: String) -> Result<(), String> {
    if account_id.len() > 128 || secret.is_empty() {
        return Err("invalid_secret_input".into());
    }
    keyring::Entry::new("com.nurasss.usageai", &account_id)
        .map_err(|_| "keychain_unavailable")?
        .set_password(&secret)
        .map_err(|_| "keychain_write_denied".into())
}

#[tauri::command]
fn remove_secret(account_id: String) -> Result<(), String> {
    keyring::Entry::new("com.nurasss.usageai", &account_id)
        .map_err(|_| "keychain_unavailable")?
        .delete_credential()
        .map_err(|_| "keychain_delete_failed".into())
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> AppSettings {
    read_settings(state.inner())
}

fn read_settings(state: &AppState) -> AppSettings {
    std::fs::read_to_string(&state.settings_path)
        .ok()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn save_settings(
    settings: AppSettings,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if !matches!(
        settings.refresh_interval_minutes,
        None | Some(1 | 5 | 10 | 15 | 30 | 60)
    ) || settings.retention_days < 30
        || settings.quota_warning_percent > 100
    {
        return Err("invalid_settings".into());
    }
    app.global_shortcut()
        .unregister_all()
        .map_err(|_| "shortcut_update_failed")?;
    let shortcut = settings.global_shortcut.clone();
    if let Err(error) = app
        .global_shortcut()
        .on_shortcut(shortcut.as_str(), |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_panel(app, None);
            }
        })
    {
        let _ = app
            .global_shortcut()
            .on_shortcut("Ctrl+Alt+U", |app, _, event| {
                if event.state() == ShortcutState::Pressed {
                    toggle_panel(app, None);
                }
            });
        tracing::warn!(error=%error,"shortcut_conflict");
        return Err("shortcut_unavailable".into());
    }
    if settings.launch_at_login {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|_| "autostart_update_failed")?;
    let json = serde_json::to_vec_pretty(&settings).map_err(|_| "settings_encode_failed")?;
    let temporary = state.settings_path.with_extension("json.tmp");
    std::fs::write(&temporary, json).map_err(|_| "settings_write_failed")?;
    std::fs::rename(&temporary, &state.settings_path).map_err(|_| "settings_commit_failed")?;
    Ok(())
}

async fn build_live_snapshot(state: &AppState) -> AppSnapshot {
    let account = Account {
        id: stable_local_account("codex"),
        provider_id: "openai".into(),
        external_identity: None,
        label: "Аккаунт 1".into(),
        connection_ref: Some("codex-local".into()),
        lifecycle: AccountLifecycle::Active,
    };
    let codex = CodexLocalAdapter::default();
    if let Ok(mut storage) = state.storage.lock() {
        let _ = storage.upsert_account(&account);
    }
    let mut providers = vec![];
    providers
        .push(refresh_local_adapter(state, &codex, &account, "OpenAI", "Codex", "Аккаунт 1").await);
    let claude_account = Account {
        id: stable_local_account("claude-code"),
        provider_id: "anthropic".into(),
        external_identity: None,
        label: "Аккаунт 1".into(),
        connection_ref: Some("claude-local".into()),
        lifecycle: AccountLifecycle::Active,
    };
    if let Ok(mut storage) = state.storage.lock() {
        let _ = storage.upsert_account(&claude_account);
    }
    providers.push(
        refresh_local_adapter(
            state,
            &ClaudeLocalAdapter,
            &claude_account,
            "Anthropic",
            "Claude Code",
            "Аккаунт 1",
        )
        .await,
    );
    // Blocked products are wired through explicit Discovery-only adapters so
    // capabilities and network allow-lists stay declared even while no metric
    // is fabricated. Refresh always fails with a safe "unavailable" state.
    let blocked: &[(&str, &str, &str, &str, &[&str])] = &[
        ("google", "antigravity", "Google", "Antigravity", &[]),
        ("zai", "glm-coding", "Z.ai", "GLM Coding Plan", &[]),
        ("opencode", "zen", "OpenCode", "Zen", &[]),
        ("openai", "chatgpt", "OpenAI", "ChatGPT", &[]),
        ("anthropic", "claude-ai", "Anthropic", "claude.ai", &[]),
        ("google", "gemini-api", "Google", "Gemini API", &[]),
    ];
    for (pid, product, pname, name, hosts) in blocked {
        let adapter = DiscoveryOnlyAdapter {
            provider: pid,
            product,
            declared: BTreeSet::new(),
            hosts,
        };
        let ghost = Account {
            id: stable_local_account(product),
            provider_id: (*pid).into(),
            external_identity: None,
            label: "Не подключено".into(),
            connection_ref: None,
            lifecycle: AccountLifecycle::Disconnected,
        };
        let key = format!("{pid}/{product}/{}", ghost.id);
        let now = Utc::now();
        if let Ok(mut meta) = state.meta.lock() {
            let entry = meta.entry(key).or_default();
            entry.last_attempt = Some(now);
            entry.error_code = Some("integration_requires_verified_source".into());
        }
        let _ = adapter.provider_id();
        providers.push(unavailable(
            pid,
            product,
            pname,
            name,
            ConnectionState::UnsupportedSource,
        ));
    }
    for provider in &mut providers {
        attach_usage_and_cost(state, provider);
    }
    providers.extend(refresh_managed_api_accounts(state).await);
    for provider in &mut providers {
        attach_balances(state, provider);
    }
    let (overview, model_breakdown, account_breakdown, project_breakdown) = build_overview(state);
    // Local-only sources work without internet; the flag stays false here and
    // the frontend ORs it with navigator.onLine for the offline banner.
    AppSnapshot {
        mode: "live".into(),
        offline: false,
        providers,
        overview,
        model_breakdown,
        account_breakdown,
        project_breakdown,
        next_refresh_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
    }
}

/// Refresh user-configured API accounts (each isolated: own secret,
/// own cooldown, own failure scope). Unknown products are skipped
/// without fabricating data.
async fn refresh_managed_api_accounts(state: &AppState) -> Vec<ProviderDto> {
    let managed: Vec<ManagedAccount> = state
        .storage
        .lock()
        .ok()
        .and_then(|s| s.list_managed_accounts().ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|a| {
            a.enabled
                && !matches!(a.lifecycle, AccountLifecycle::Archived)
                && a.product_id.as_deref() == Some("openai-api")
        })
        .collect();
    let mut out = vec![];
    for account in managed {
        let key = format!("openai/openai-api/{}", account.id);
        if cooldown_remaining(
            &state
                .meta
                .lock()
                .ok()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
            &key,
        )
        .is_some()
        {
            let mut dto = unavailable(
                "openai",
                "openai-api",
                "OpenAI",
                "API",
                ConnectionState::Cooldown,
            );
            dto.account_id = account.id.to_string();
            dto.alias = account.label.clone();
            out.push(dto);
            continue;
        }
        let probe = Account {
            id: account.id,
            provider_id: account.provider_id.clone(),
            external_identity: account.external_identity.clone(),
            label: account.label.clone(),
            connection_ref: account.connection_ref.clone(),
            lifecycle: account.lifecycle.clone(),
        };
        if let Ok(mut meta) = state.meta.lock() {
            let entry = meta.entry(key.clone()).or_default();
            entry.last_attempt = Some(Utc::now());
            entry.connector_version =
                usage_providers::openai_api::OPENAI_API_CONNECTOR_VERSION.into();
            entry.parser_version = usage_providers::openai_api::OPENAI_COSTS_PARSER_VERSION.into();
        }
        let Some(secret) = read_secret(&account.id.to_string()) else {
            if let Ok(mut meta) = state.meta.lock() {
                if let Some(entry) = meta.get_mut(&key) {
                    entry.error_code = Some("authentication_required".into());
                }
            }
            let mut dto = unavailable(
                "openai",
                "openai-api",
                "OpenAI",
                "API",
                ConnectionState::AuthenticationRequired,
            );
            dto.account_id = account.id.to_string();
            dto.alias = account.label.clone();
            out.push(dto);
            continue;
        };
        let adapter = OpenAiApiAdapter::default();
        let snapshot_result = tokio::time::timeout(
            SOURCE_TIMEOUT,
            adapter.refresh_with_key(&probe, &secret, SOURCE_TIMEOUT),
        )
        .await;
        match snapshot_result {
            Ok(Ok(snapshot)) => {
                // Import reported costs idempotently; billing delay is honest
                // ProviderDelayed coverage from the parser.
                match adapter
                    .import_costs_with_key(&probe, &secret, SOURCE_TIMEOUT)
                    .await
                {
                    Ok((records, _warnings)) => {
                        if let Ok(mut storage) = state.storage.lock() {
                            for record in &records {
                                let _ = storage.upsert_cost(record);
                            }
                        }
                    }
                    Err(error) => {
                        if let Ok(mut meta) = state.meta.lock() {
                            if let Some(entry) = meta.get_mut(&key) {
                                entry.error_code = Some(safe_error_code(&error));
                            }
                        }
                    }
                }
                if let Ok(mut meta) = state.meta.lock() {
                    if let Some(entry) = meta.get_mut(&key) {
                        entry.last_success = Some(Utc::now());
                        entry.last_observed = snapshot.observed_at;
                        entry.cooldown_until = None;
                        entry.error_code = None;
                    }
                }
                let mut dto = from_snapshot(snapshot, "OpenAI", "API", &account.label);
                dto.account_id = account.id.to_string();
                out.push(dto);
            }
            Ok(Err(error)) => {
                if let ProviderError::RateLimited(retry_after) = &error {
                    let delay = retry_after
                        .map(StdDuration::from_secs)
                        .unwrap_or(FALLBACK_COOLDOWN);
                    if let Ok(mut meta) = state.meta.lock() {
                        if let Some(entry) = meta.get_mut(&key) {
                            entry.cooldown_until = Some(
                                Utc::now()
                                    + Duration::from_std(delay).unwrap_or(Duration::minutes(5)),
                            );
                            entry.error_code = Some("rate_limited".into());
                        }
                    }
                } else if let Ok(mut meta) = state.meta.lock() {
                    if let Some(entry) = meta.get_mut(&key) {
                        entry.error_code = Some(safe_error_code(&error));
                    }
                }
                let mut dto =
                    unavailable("openai", "openai-api", "OpenAI", "API", map_error(&error));
                dto.account_id = account.id.to_string();
                dto.alias = account.label.clone();
                out.push(dto);
            }
            Err(_) => {
                if let Ok(mut meta) = state.meta.lock() {
                    if let Some(entry) = meta.get_mut(&key) {
                        entry.error_code = Some("source_timeout".into());
                    }
                }
                let mut dto = unavailable(
                    "openai",
                    "openai-api",
                    "OpenAI",
                    "API",
                    ConnectionState::NetworkError,
                );
                dto.account_id = account.id.to_string();
                dto.alias = account.label.clone();
                out.push(dto);
            }
        }
    }
    out
}

fn attach_balances(state: &AppState, provider: &mut ProviderDto) {
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

async fn refresh_local_adapter<A: ProviderAdapter>(
    state: &AppState,
    adapter: &A,
    account: &Account,
    provider_name: &str,
    product_name: &str,
    alias: &str,
) -> ProviderDto {
    let key = format!(
        "{}/{}/{}",
        adapter.provider_id(),
        adapter.product_id(),
        account.id
    );
    if cooldown_remaining(
        &state
            .meta
            .lock()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default(),
        &key,
    )
    .is_some()
    {
        let mut dto = unavailable(
            adapter.provider_id(),
            adapter.product_id(),
            provider_name,
            product_name,
            ConnectionState::Cooldown,
        );
        dto.account_id = account.id.to_string();
        dto.alias = alias.into();
        return dto;
    }
    let now = Utc::now();
    if let Ok(mut meta) = state.meta.lock() {
        let entry = meta.entry(key.clone()).or_default();
        entry.last_attempt = Some(now);
    }
    // Per-source timeout: one slow source never blocks the rest of the panel.
    let result = tokio::time::timeout(SOURCE_TIMEOUT, adapter.refresh(account)).await;
    match result {
        Ok(Ok(snapshot)) => {
            if let Ok(mut meta) = state.meta.lock() {
                if let Some(entry) = meta.get_mut(&key) {
                    entry.last_success = Some(Utc::now());
                    entry.last_observed = snapshot.observed_at;
                    entry.cooldown_until = None;
                    entry.error_code = None;
                }
            }
            from_snapshot(snapshot, provider_name, product_name, alias)
        }
        Ok(Err(error)) => {
            if let ProviderError::RateLimited(retry_after) = &error {
                let delay = retry_after
                    .map(StdDuration::from_secs)
                    .unwrap_or(FALLBACK_COOLDOWN);
                if let Ok(mut meta) = state.meta.lock() {
                    if let Some(entry) = meta.get_mut(&key) {
                        entry.cooldown_until = Some(
                            Utc::now() + Duration::from_std(delay).unwrap_or(Duration::minutes(5)),
                        );
                        entry.error_code = Some("rate_limited".into());
                    }
                }
            } else if let Ok(mut meta) = state.meta.lock() {
                if let Some(entry) = meta.get_mut(&key) {
                    entry.error_code = Some(safe_error_code(&error));
                }
            }
            let mut dto = unavailable(
                adapter.provider_id(),
                adapter.product_id(),
                provider_name,
                product_name,
                map_error(&error),
            );
            dto.account_id = account.id.to_string();
            dto.alias = alias.into();
            dto
        }
        Err(_) => {
            if let Ok(mut meta) = state.meta.lock() {
                if let Some(entry) = meta.get_mut(&key) {
                    entry.error_code = Some("source_timeout".into());
                }
            }
            let mut dto = unavailable(
                adapter.provider_id(),
                adapter.product_id(),
                provider_name,
                product_name,
                ConnectionState::NetworkError,
            );
            dto.account_id = account.id.to_string();
            dto.alias = alias.into();
            dto
        }
    }
}

fn safe_error_code(error: &ProviderError) -> String {
    match error {
        ProviderError::AuthenticationRequired => "authentication_required".into(),
        ProviderError::PermissionDenied => "permission_denied".into(),
        ProviderError::InsufficientScope => "insufficient_scope".into(),
        ProviderError::RateLimited(_) => "rate_limited".into(),
        ProviderError::Network(_) => "network_error".into(),
        ProviderError::Unavailable(_) => "unavailable".into(),
        ProviderError::Parse(_) => "parse_error".into(),
    }
}

fn attach_usage_and_cost(state: &AppState, provider: &mut ProviderDto) {
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
    if let Ok(storage) = state.storage.lock() {
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
                // Reported cost for the same scope always wins over an estimate;
                // different currencies are never summed (see aggregation rules).
                let target = if record.kind == CostKind::Reported {
                    &mut reported
                } else {
                    &mut estimated
                };
                *target
                    .entry(record.currency.clone())
                    .or_insert(rust_decimal::Decimal::ZERO) += record.amount_decimal;
            }
            // Same-scope dedup: drop estimates where a reported value exists.
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
}

async fn import_known_history(state: &AppState) {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let account = Account {
        id: stable_local_account("codex"),
        provider_id: "openai".into(),
        external_identity: None,
        label: "Аккаунт 1".into(),
        connection_ref: Some("codex-local".into()),
        lifecycle: AccountLifecycle::Active,
    };
    let codex = CodexLocalAdapter::default();
    import_glob(
        state,
        &codex,
        &account,
        &format!("{}/.codex/sessions/**/rollout-*.jsonl", home.display()),
    )
    .await;
    let claude_account = Account {
        id: stable_local_account("claude-code"),
        provider_id: "anthropic".into(),
        external_identity: None,
        label: "Аккаунт 1".into(),
        connection_ref: Some("claude-local".into()),
        lifecycle: AccountLifecycle::Active,
    };
    import_glob(
        state,
        &ClaudeLocalAdapter,
        &claude_account,
        &format!("{}/.claude/projects/**/*.jsonl", home.display()),
    )
    .await;
    if let Ok(mut storage) = state.storage.lock() {
        let _ = storage
            .prune_before(Utc::now() - Duration::days(read_settings(state).retention_days as i64));
    }
}

async fn import_glob<A: ProviderAdapter>(
    state: &AppState,
    adapter: &A,
    account: &Account,
    pattern: &str,
) {
    let Ok(paths) = glob(pattern) else { return };
    for path in paths.filter_map(Result::ok) {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let size = meta.len();
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let path_hash = anonymous_path_identity(&path);
        let offset = {
            let Ok(mut storage) = state.storage.lock() else {
                return;
            };
            storage
                .reset_checkpoint_on_rotation(&path_hash, size, mtime_ms)
                .unwrap_or(0)
        };
        let Ok(batch) = adapter.import_local(account, &path, offset).await else {
            continue;
        };
        if let Ok(mut storage) = state.storage.lock() {
            for record in &batch.records {
                let _ = storage.upsert_usage(record);
            }
            let _ = storage.set_checkpoint(&ImportCheckpoint {
                path_hash,
                mtime_ms,
                size,
                byte_offset: batch.next_offset,
                schema_version: 1,
                last_source_record_id: batch.records.last().map(|r| r.source_record_id.clone()),
            });
        }
    }
}

fn stable_local_account(namespace: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("usage.ai/local/{namespace}").as_bytes(),
    )
}

fn build_overview(
    state: &AppState,
) -> (
    HashMap<String, Vec<OverviewSegment>>,
    HashMap<String, Vec<OverviewSegment>>,
    HashMap<String, Vec<OverviewSegment>>,
    HashMap<String, Vec<OverviewSegment>>,
) {
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
    let ranges = [
        ("today", today, tomorrow),
        ("yesterday", yesterday, today),
        ("7days", week, tomorrow),
        ("30days", month, tomorrow),
    ];
    let mut result = HashMap::new();
    let mut models: HashMap<String, Vec<OverviewSegment>> = HashMap::new();
    let mut accounts: HashMap<String, Vec<OverviewSegment>> = HashMap::new();
    let mut projects: HashMap<String, Vec<OverviewSegment>> = HashMap::new();
    if let Ok(storage) = state.storage.lock() {
        for (key, start, end) in ranges {
            let (start_utc, end_utc) = (start.with_timezone(&Utc), end.with_timezone(&Utc));
            let rows = storage
                .token_totals_between(start_utc, end_utc)
                .unwrap_or_default();
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
            // Unknown model is never hidden and never merged into a fake name.
            let model_rows = storage
                .model_totals_between(start_utc, end_utc)
                .unwrap_or_default();
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
                .account_product_totals_between(start_utc, end_utc)
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
                .project_totals_between(start_utc, end_utc)
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

fn local_midnight(date: chrono::NaiveDate) -> chrono::DateTime<Local> {
    let naive = date.and_hms_opt(0, 0, 0).expect("midnight is valid");
    Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| Local.from_local_datetime(&naive).latest())
        .unwrap_or_else(Local::now)
}

fn from_snapshot(
    s: usage_core::Snapshot,
    provider_name: &str,
    product_name: &str,
    alias: &str,
) -> ProviderDto {
    ProviderDto {
        account_id: s.account_id.to_string(),
        provider_id: s.provider_id,
        product_id: s.product_id,
        provider_name: provider_name.into(),
        product_name: product_name.into(),
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
    }
}

fn capability_name(value: Capability) -> String {
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

fn unavailable(
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
        alias: "Не подключено".into(),
        plan_label: None,
        connection_state: state,
        freshness: Freshness::Unknown,
        coverage: Coverage::Unknown,
        fetched_at: Utc::now().to_rfc3339(),
        observed_at: None,
        capabilities: vec![],
        quotas: vec![],
        tokens_today: None,
        reported_cost_today: None,
        estimated_cost_today: None,
        balances: vec![],
    }
}
fn map_error(e: &usage_core::ProviderError) -> ConnectionState {
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

fn toggle_panel<R: Runtime>(app: &tauri::AppHandle<R>, position: Option<PhysicalPosition<f64>>) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            if let Some(pos) = position {
                let scale = window.scale_factor().unwrap_or(1.0);
                let size = window.outer_size().unwrap_or_default();
                let x = pos.x - size.width as f64 / scale + 18.0;
                let y = pos.y + 8.0;
                let _ = window.set_position(PhysicalPosition::new(x, y));
            }
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;

    #[test]
    fn cooldown_blocks_only_expired_entries() {
        let mut meta = HashMap::new();
        assert!(cooldown_remaining(&meta, "a/b/c").is_none());
        meta.insert(
            "a/b/c".into(),
            RefreshMeta {
                cooldown_until: Some(Utc::now() + Duration::minutes(5)),
                ..RefreshMeta::default()
            },
        );
        assert!(cooldown_remaining(&meta, "a/b/c").is_some());
        meta.insert(
            "a/b/c".into(),
            RefreshMeta {
                cooldown_until: Some(Utc::now() - Duration::minutes(1)),
                ..RefreshMeta::default()
            },
        );
        assert!(cooldown_remaining(&meta, "a/b/c").is_none());
    }

    #[test]
    fn error_codes_stay_safe() {
        assert_eq!(
            safe_error_code(&ProviderError::RateLimited(Some(60))),
            "rate_limited"
        );
        assert_eq!(
            safe_error_code(&ProviderError::Network("tcp fail".into())),
            "network_error"
        );
        assert_eq!(
            status_class(&ConnectionState::ParseError).as_deref(),
            Some("schema_change")
        );
        assert_eq!(
            status_class(&ConnectionState::UnsupportedSource).as_deref(),
            Some("unavailable")
        );
    }
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .without_time()
        .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let storage = Storage::open(dir.join("usage.db"))?;
            app.manage(AppState {
                storage: Mutex::new(storage),
                cached: Mutex::new(None),
                meta: Mutex::new(HashMap::new()),
                settings_path: dir.join("settings.json"),
            });
            // Scheduler performs one current refresh per tick after sleep/wake
            // instead of replaying the missed queue, so no notification storm
            // can be generated from stale data.
            let scheduler_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut first = true;
                loop {
                    let Some(state) = scheduler_app.try_state::<AppState>() else {
                        break;
                    };
                    if !first {
                        let Some(minutes) = read_settings(state.inner()).refresh_interval_minutes
                        else {
                            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                            continue;
                        };
                        tokio::time::sleep(std::time::Duration::from_secs(minutes * 60)).await;
                    }
                    first = false;
                    import_known_history(state.inner()).await;
                    let value = build_live_snapshot(state.inner()).await;
                    cache_snapshot(state.inner(), &value);
                    deliver_notifications(&scheduler_app, state.inner(), &value);
                    if let Ok(mut cached) = state.cached.lock() {
                        *cached = Some(value);
                    };
                }
            });
            if let Err(error) = app
                .global_shortcut()
                .on_shortcut("Ctrl+Alt+U", |app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_panel(app, None);
                    }
                })
            {
                tracing::warn!(error=%error,"global_shortcut_unavailable");
            }
            let refresh = MenuItemBuilder::with_id("refresh", "Обновить")
                .accelerator("CmdOrCtrl+R")
                .build(app)?;
            let settings = MenuItemBuilder::with_id("settings", "Настройки…")
                .accelerator("CmdOrCtrl+,")
                .build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Выйти").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&refresh, &settings, &quit])
                .build()?;
            let mut tray = TrayIconBuilder::with_id("usage-tray")
                .tooltip("Usage.ai")
                .menu(&menu)
                .show_menu_on_left_click(false);
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    position,
                    ..
                } = event
                {
                    toggle_panel(tray.app_handle(), Some(position));
                }
            })
            .on_menu_event(|app, event| match event.id().as_ref() {
                "refresh" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(state) = app.try_state::<AppState>() {
                            import_known_history(state.inner()).await;
                            let value = build_live_snapshot(state.inner()).await;
                            cache_snapshot(state.inner(), &value);
                            deliver_notifications(&app, state.inner(), &value);
                            if let Ok(mut cache) = state.cached.lock() {
                                *cache = Some(value);
                            }
                        }
                    });
                }
                "settings" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = window.emit("open-settings", ());
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(false) = event {
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_all,
            export_history,
            export_diagnostics,
            get_diagnostics,
            list_accounts,
            add_account,
            update_account,
            test_connection,
            remove_account,
            list_budgets,
            save_budget,
            delete_budget,
            get_storage_status,
            get_app_info,
            store_secret,
            remove_secret,
            get_settings,
            save_settings
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Usage.ai");
}
