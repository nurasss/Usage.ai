use chrono::{Local, NaiveTime, Utc};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use usage_core::CostKind;
use usage_runtime::{PlannedNotice, ProviderNoticeView, QuotaNoticeView};

use crate::appstate::AppState;
use crate::commands::settings::read_settings;
use crate::dto::AppSnapshot;

/// Delivery adapter: runtime plans notices on committed data, storage
/// deduplicates, the platform sends. Stale/backfilled data never
/// produces a notification storm.
pub fn deliver(app: &AppHandle, state: &AppState, snapshot: &AppSnapshot) {
    let settings = read_settings(state);
    if !settings.notifications_enabled || quiet_now(&settings) {
        return;
    }
    let views: Vec<ProviderNoticeView> = snapshot
        .providers
        .iter()
        .map(|p| ProviderNoticeView {
            account_id: p.account_id.clone(),
            provider_name: p.provider_name.clone(),
            product_name: p.product_name.clone(),
            connection_state: p.connection_state.clone(),
            fresh: p.freshness == usage_core::Freshness::Fresh,
            quotas: p
                .quotas
                .iter()
                .map(|q| QuotaNoticeView {
                    pool_id: q.pool_id.clone(),
                    window_id: q.window_id.clone(),
                    name: q.name.clone(),
                    remaining: q.remaining_percent,
                    resets_at: q.resets_at.as_deref().and_then(|s| s.parse().ok()),
                    window_start: q.window_start.as_deref().and_then(|s| s.parse().ok()),
                    window_kind: q.window_kind.clone(),
                })
                .collect(),
        })
        .collect();
    let planned: Vec<PlannedNotice> = match state.planner.lock() {
        Ok(mut planner) => {
            let now = Utc::now();
            views
                .iter()
                .flat_map(|v| planner.plan_provider(v, now))
                .collect()
        }
        Err(_) => vec![],
    };
    for notice in planned {
        send_if_new(state, app, &notice);
    }
    deliver_budget_notices(app, state);
}

fn send_if_new(state: &AppState, app: &AppHandle, notice: &PlannedNotice) {
    let should_send = state
        .storage
        .lock()
        .ok()
        .and_then(|mut storage| {
            storage
                .mark_notification_delivered(
                    &notice.account_id,
                    &notice.metric,
                    notice.threshold,
                    &notice.window,
                )
                .ok()
        })
        .unwrap_or(false);
    if should_send {
        let _ = app
            .notification()
            .builder()
            .title(notice.title.clone())
            .body(notice.body.clone())
            .show();
    }
}

/// Budget notices carry their account scope explicitly.
fn deliver_budget_notices(app: &AppHandle, state: &AppState) {
    use chrono::Datelike;
    let month_start = crate::refresh_flow::local_midnight(
        Local::now()
            .date_naive()
            .pred_opt()
            .map(|d| d.with_day(1).unwrap_or(d))
            .unwrap_or_else(|| Local::now().date_naive()),
    );
    let month_key = Local::now().format("%Y-%m").to_string();
    let budgets = state
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
        let Some(spent) = spent else { continue };
        let Some(notice) = usage_runtime::Planner::plan_budget(
            &budget.account_id.to_string(),
            &budget.product_id,
            &budget.currency,
            budget.amount_decimal,
            spent,
            &month_key,
        ) else {
            continue;
        };
        let should_send = state
            .storage
            .lock()
            .ok()
            .and_then(|mut storage| {
                storage
                    .mark_notification_delivered(
                        &budget.account_id.to_string(),
                        &notice.metric,
                        notice.threshold,
                        &notice.window,
                    )
                    .ok()
            })
            .unwrap_or(false);
        if should_send {
            let _ = app
                .notification()
                .builder()
                .title(notice.title)
                .body(notice.body)
                .show();
        }
    }
}

pub fn quiet_now(settings: &crate::dto::AppSettings) -> bool {
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
