use chrono::{Local, NaiveTime, Utc};
use rust_decimal::Decimal;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use usage_core::CostKind;
use usage_runtime::{PlannedNotice, ProviderNoticeView, QuotaNoticeView};
use usage_storage::{Budget, Storage};

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
    // Per-profile mute: silenced accounts refresh normally but never
    // alert. Unknown accounts default to notifying (fail-open toward
    // visibility, never toward silence).
    let muted: std::collections::HashSet<String> = state
        .storage
        .lock()
        .ok()
        .map(|storage| {
            storage
                .list_managed_accounts()
                .unwrap_or_default()
                .into_iter()
                .filter(|account| account.notifications_muted)
                .map(|account| account.id.to_string())
                .collect()
        })
        .unwrap_or_default();
    let views: Vec<ProviderNoticeView> = snapshot
        .providers
        .iter()
        .filter(|p| !muted.contains(&p.account_id))
        .map(|p| ProviderNoticeView {
            account_id: p.account_id.clone(),
            alias: p.alias.clone(),
            provider_name: p.provider_name.clone(),
            product_name: p.product_name.clone(),
            connection_state: p.connection_state.clone(),
            fresh: p.freshness == usage_core::Freshness::Fresh,
            coverage: p.coverage,
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
            let thresholds = usage_runtime::warning_thresholds(settings.quota_warning_percent);
            views
                .iter()
                .flat_map(|v| planner.plan_provider_at(v, now, &thresholds))
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
        let spent = state.storage.lock().ok().and_then(|s| {
            reported_spend_for_budget(&s, &budget, month_start.with_timezone(&Utc), Utc::now())
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

/// Read the reported, authoritative spend used by the real budget delivery
/// path. Persisted coverage has already crossed the storage parser boundary;
/// this predicate remains explicit so non-authoritative rows cannot influence
/// notices or remaining-budget calculations.
fn reported_spend_for_budget(
    storage: &Storage,
    budget: &Budget,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
) -> Option<Decimal> {
    storage
        .cost_records_between(start, end)
        .ok()
        .map(|records| {
            records
                .iter()
                .filter(|record| {
                    record.account_id == budget.account_id
                        && record.product_id == budget.product_id
                        && record.currency == budget.currency
                        && record.kind == CostKind::Reported
                        && record.coverage.is_authoritative()
                })
                .map(|record| record.amount_decimal)
                .sum()
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use usage_core::{CostKind, CostRecord, Coverage};
    use usage_runtime::Planner;
    use uuid::Uuid;

    #[test]
    fn budget_spend_path_excludes_non_authoritative_rows() {
        let mut storage = Storage::in_memory().unwrap();
        let account_id = Uuid::new_v4();
        let start = Utc::now();
        let observed_at = start + Duration::hours(1);
        let budget = Budget {
            account_id,
            product_id: "codex".into(),
            currency: "USD".into(),
            amount_decimal: Decimal::new(200, 2),
            period: "monthly".into(),
        };
        let make_cost =
            |source_record_id: &str, amount_decimal: Decimal, coverage: Coverage| -> CostRecord {
                CostRecord {
                    account_id,
                    product_id: "codex".into(),
                    billing_scope_id: None,
                    source_record_id: source_record_id.into(),
                    period_start: observed_at,
                    period_end: observed_at,
                    amount_decimal,
                    currency: "USD".into(),
                    kind: CostKind::Reported,
                    source: "budget-test".into(),
                    coverage,
                }
            };
        for (id, amount, coverage) in [
            ("complete", Decimal::new(100, 2), Coverage::Complete),
            ("partial", Decimal::new(50, 2), Coverage::Partial),
            ("unknown", Decimal::new(1000, 2), Coverage::Unknown),
            (
                "unverified",
                Decimal::new(2000, 2),
                Coverage::UnverifiedSemantics,
            ),
        ] {
            storage
                .upsert_cost(&make_cost(id, amount, coverage))
                .unwrap();
        }

        let spent =
            reported_spend_for_budget(&storage, &budget, start, start + Duration::days(1)).unwrap();
        assert_eq!(spent, Decimal::new(150, 2));
        assert_eq!(budget.amount_decimal - spent, Decimal::new(50, 2));
        assert!(Planner::plan_budget(
            &account_id.to_string(),
            &budget.product_id,
            &budget.currency,
            budget.amount_decimal,
            spent,
            "2026-09",
        )
        .is_none());

        // The same production path still alerts when canonical authoritative
        // rows genuinely exceed the configured budget.
        assert!(Planner::plan_budget(
            &account_id.to_string(),
            &budget.product_id,
            &budget.currency,
            Decimal::new(100, 2),
            spent,
            "2026-09",
        )
        .is_some());
    }
}
