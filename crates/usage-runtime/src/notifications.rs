use chrono::{DateTime, Utc};
use std::collections::HashMap;
use usage_core::ConnectionState;

/// Notification business rules live here; the platform layer only
/// delivers and persists dedup keys. Every rule fires on committed
/// validated data, never on countdown expiry alone.
#[derive(Debug, Clone)]
pub struct QuotaNoticeView {
    pub pool_id: String,
    pub window_id: Option<String>,
    pub name: String,
    pub remaining: Option<f64>,
    pub resets_at: Option<DateTime<Utc>>,
    pub window_start: Option<DateTime<Utc>>,
    pub window_kind: String,
}

#[derive(Debug, Clone)]
pub struct ProviderNoticeView {
    pub account_id: String,
    pub provider_name: String,
    pub product_name: String,
    pub connection_state: ConnectionState,
    pub fresh: bool,
    pub quotas: Vec<QuotaNoticeView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedNotice {
    pub account_id: String,
    pub title: String,
    pub body: String,
    pub metric: String,
    pub threshold: u8,
    pub window: String,
}

#[derive(Debug, Default)]
pub struct Planner {
    quota_windows: HashMap<String, (Option<String>, Option<f64>)>,
    observations: HashMap<String, Vec<(DateTime<Utc>, f64)>>,
    last_state: HashMap<String, ConnectionState>,
}

fn auth_like(state: &ConnectionState) -> bool {
    matches!(
        state,
        ConnectionState::AuthenticationRequired
            | ConnectionState::SessionExpired
            | ConnectionState::PermissionDenied
            | ConnectionState::InsufficientScope
    )
}

fn state_label(state: &ConnectionState) -> &'static str {
    match state {
        ConnectionState::AuthenticationRequired => "Нужна авторизация",
        ConnectionState::SessionExpired => "Сессия истекла",
        ConnectionState::PermissionDenied => "Нет разрешения",
        ConnectionState::InsufficientScope => "Недостаточно прав",
        _ => "Подключение требует внимания",
    }
}

impl Planner {
    pub fn plan_provider(
        &mut self,
        view: &ProviderNoticeView,
        now: DateTime<Utc>,
    ) -> Vec<PlannedNotice> {
        let mut out = vec![];
        let account_key = view.account_id.clone();
        // Disconnect/auth transitions fire even on stale snapshots.
        if auth_like(&view.connection_state)
            && self.last_state.get(&account_key) != Some(&view.connection_state)
        {
            out.push(PlannedNotice {
                account_id: view.account_id.clone(),
                title: format!("{} · {}", view.provider_name, view.product_name),
                body: format!(
                    "{} — откройте исходный клиент или проверьте подключение",
                    state_label(&view.connection_state)
                ),
                metric: format!("connection:{:?}", view.connection_state),
                threshold: 1,
                window: "current".into(),
            });
        }
        self.last_state
            .insert(account_key.clone(), view.connection_state.clone());
        if !view.fresh {
            return out;
        }
        for quota in &view.quotas {
            let Some(remaining) = quota.remaining else {
                continue;
            };
            let Some(window) = quota.window_id.as_deref() else {
                continue;
            };
            let pool_key = format!("{account_key}:{}", quota.pool_id);
            // Confirmed reset: window changed AND remaining jumped up.
            let reset_confirmed = self
                .quota_windows
                .get(&pool_key)
                .map(|(prev_window, prev_remaining)| {
                    prev_window.as_deref() != Some(window)
                        && prev_remaining.is_some_and(|prev| remaining > prev)
                })
                .unwrap_or(false);
            if reset_confirmed {
                out.push(PlannedNotice {
                    account_id: view.account_id.clone(),
                    title: format!("{} · {}", view.provider_name, quota.name),
                    body: "Лимит восстановлен — подтверждено новым наблюдением".into(),
                    metric: format!("quota-reset:{}", quota.pool_id),
                    threshold: 0,
                    window: window.into(),
                });
            }
            self.quota_windows.insert(
                pool_key.clone(),
                (Some(window.to_string()), Some(remaining)),
            );
            let obs = self.observations.entry(pool_key.clone()).or_default();
            obs.push((now, 100.0 - remaining));
            if obs.len() > 2 {
                obs.remove(0);
            }
            // Pace projection: fixed windows with two fresh observations.
            if quota.window_kind == "fixed" {
                if let (Some(reset_at), Some(_)) = (quota.resets_at, quota.window_start) {
                    if obs.len() >= 2 {
                        let (t1, u1) = obs[obs.len() - 2];
                        let (t2, u2) = obs[obs.len() - 1];
                        let dt = (t2 - t1).num_seconds();
                        if dt > 0 && u2 > u1 {
                            let rate = (u2 - u1) / dt as f64;
                            let exhaust_at =
                                t2 + chrono::Duration::seconds(((100.0 - u2) / rate) as i64);
                            if exhaust_at < reset_at {
                                out.push(PlannedNotice {
                                    account_id: view.account_id.clone(),
                                    title: format!("{} · {}", view.provider_name, quota.name),
                                    body: "Темп выше лимита: может исчерпаться до сброса".into(),
                                    metric: format!("pace:{}", quota.pool_id),
                                    threshold: 0,
                                    window: window.into(),
                                });
                            }
                        }
                    }
                }
            }
            for threshold in [100u8, 90, 80] {
                if remaining > f64::from(100 - threshold) {
                    continue;
                }
                out.push(PlannedNotice {
                    account_id: view.account_id.clone(),
                    title: format!("{} · {}", view.provider_name, quota.name),
                    body: format!("Использовано {threshold}% · осталось {:.0}%", remaining),
                    metric: format!("quota:{}", quota.pool_id),
                    threshold,
                    window: window.into(),
                });
                break;
            }
        }
        out
    }

    /// Monthly reported spend vs a user budget. Currencies are never
    /// converted; `None` means no alert.
    pub fn plan_budget(
        account_id: &str,
        product_id: &str,
        currency: &str,
        budget_amount: rust_decimal::Decimal,
        reported_spent: rust_decimal::Decimal,
        month_key: &str,
    ) -> Option<PlannedNotice> {
        if reported_spent < budget_amount {
            return None;
        }
        Some(PlannedNotice {
            account_id: account_id.into(),
            title: "Бюджет превышен".into(),
            body: format!(
                "{product_id} · {budget_amount} {currency} — фактические расходы за месяц"
            ),
            metric: format!("budget:{product_id}:{currency}"),
            threshold: 100,
            window: month_key.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn view(
        state: ConnectionState,
        fresh: bool,
        quotas: Vec<QuotaNoticeView>,
    ) -> ProviderNoticeView {
        ProviderNoticeView {
            account_id: "a1".into(),
            provider_name: "OpenAI".into(),
            product_name: "Codex".into(),
            connection_state: state,
            fresh,
            quotas,
        }
    }

    fn quota(pool: &str, remaining: f64, window: &str) -> QuotaNoticeView {
        QuotaNoticeView {
            pool_id: pool.into(),
            window_id: Some(window.into()),
            name: "Лимит".into(),
            remaining: Some(remaining),
            resets_at: None,
            window_start: None,
            window_kind: "rolling".into(),
        }
    }

    #[test]
    fn thresholds_fire_once_per_level() {
        let mut planner = Planner::default();
        let notices = planner.plan_provider(
            &view(
                ConnectionState::Connected,
                true,
                vec![quota("p", 0.0, "w1")],
            ),
            Utc::now(),
        );
        assert!(notices.iter().any(|n| n.threshold == 100));
        assert!(!notices.iter().any(|n| n.threshold == 90));
    }

    #[test]
    fn stale_data_suppresses_quota_but_not_disconnect() {
        let mut planner = Planner::default();
        let notices = planner.plan_provider(
            &view(
                ConnectionState::AuthenticationRequired,
                false,
                vec![quota("p", 1.0, "w1")],
            ),
            Utc::now(),
        );
        assert_eq!(notices.len(), 1);
        assert!(notices[0].metric.starts_with("connection:"));
    }

    #[test]
    fn reset_requires_window_change_and_jump() {
        let mut planner = Planner::default();
        let now = Utc::now();
        let _ = planner.plan_provider(
            &view(
                ConnectionState::Connected,
                true,
                vec![quota("p", 5.0, "w1")],
            ),
            now,
        );
        // Same window, higher remaining: no reset notice (could be
        // re-observation), only thresholds.
        let second = planner.plan_provider(
            &view(
                ConnectionState::Connected,
                true,
                vec![quota("p", 60.0, "w1")],
            ),
            now,
        );
        assert!(!second.iter().any(|n| n.metric.starts_with("quota-reset")));
        // New window with jump: confirmed reset.
        let third = planner.plan_provider(
            &view(
                ConnectionState::Connected,
                true,
                vec![quota("p", 95.0, "w2")],
            ),
            now,
        );
        assert!(third.iter().any(|n| n.metric == "quota-reset:p"));
    }

    #[test]
    fn budget_alert_needs_reported_basis_only() {
        assert!(Planner::plan_budget(
            "a1",
            "openai-api",
            "USD",
            Decimal::new(20, 0),
            Decimal::new(25, 0),
            "2026-09"
        )
        .is_some());
        assert!(Planner::plan_budget(
            "a1",
            "openai-api",
            "USD",
            Decimal::new(20, 0),
            Decimal::new(19, 0),
            "2026-09"
        )
        .is_none());
    }
}
