use crate::{CostKind, CostRecord, UsageRecord};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashSet};

pub fn sum_tokens(records: &[UsageRecord]) -> u64 {
    records
        .iter()
        .map(|r| {
            r.total_tokens.unwrap_or_else(|| {
                r.input_tokens
                    .unwrap_or(0)
                    .saturating_add(r.output_tokens.unwrap_or(0))
            })
        })
        .sum()
}

pub fn costs_by_currency(records: &[CostRecord]) -> BTreeMap<String, Decimal> {
    let reported_scopes: HashSet<_> = records
        .iter()
        .filter(|r| r.kind == CostKind::Reported)
        .map(scope_key)
        .collect();
    let organization_totals: HashSet<_> = records
        .iter()
        .filter(|r| r.kind == CostKind::Reported && r.billing_scope_id.is_none())
        .map(|r| {
            (
                r.account_id,
                r.product_id.as_str(),
                r.period_start,
                r.period_end,
            )
        })
        .collect();
    let mut totals = BTreeMap::new();
    for record in records {
        if record.kind == CostKind::Estimated && reported_scopes.contains(&scope_key(record)) {
            continue;
        }
        if record.billing_scope_id.is_some()
            && organization_totals.contains(&(
                record.account_id,
                record.product_id.as_str(),
                record.period_start,
                record.period_end,
            ))
        {
            continue;
        }
        *totals
            .entry(record.currency.clone())
            .or_insert(Decimal::ZERO) += record.amount_decimal;
    }
    totals
}

fn scope_key(
    record: &CostRecord,
) -> (
    uuid::Uuid,
    &str,
    Option<&str>,
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Utc>,
) {
    (
        record.account_id,
        &record.product_id,
        record.billing_scope_id.as_deref(),
        record.period_start,
        record.period_end,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CostKind, Coverage};
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;
    fn cost(kind: CostKind, currency: &str, amount: i64) -> CostRecord {
        let now = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        CostRecord {
            account_id: Uuid::nil(),
            product_id: "zen".into(),
            billing_scope_id: Some("scope".into()),
            source_record_id: format!("{kind:?}"),
            period_start: now,
            period_end: now,
            amount_decimal: Decimal::new(amount, 0),
            currency: currency.into(),
            kind,
            source: "fixture".into(),
            coverage: Coverage::Complete,
        }
    }
    #[test]
    fn reported_replaces_estimate_for_same_scope() {
        assert_eq!(
            costs_by_currency(&[
                cost(CostKind::Estimated, "USD", 2),
                cost(CostKind::Reported, "USD", 3)
            ])["USD"],
            Decimal::new(3, 0)
        );
    }
    #[test]
    fn currencies_stay_separate() {
        let result = costs_by_currency(&[
            cost(CostKind::Reported, "USD", 3),
            cost(CostKind::Reported, "EUR", 4),
        ]);
        assert_eq!(result.len(), 2);
    }
    #[test]
    fn organization_total_excludes_project_subsets() {
        let mut org = cost(CostKind::Reported, "USD", 10);
        org.billing_scope_id = None;
        let project = cost(CostKind::Reported, "USD", 4);
        assert_eq!(
            costs_by_currency(&[org, project])["USD"],
            Decimal::new(10, 0)
        );
    }
    #[test]
    fn gateway_and_direct_provider_remain_separate() {
        let zen = cost(CostKind::Reported, "USD", 4);
        let mut direct = cost(CostKind::Reported, "USD", 3);
        direct.product_id = "anthropic-api".into();
        assert_eq!(costs_by_currency(&[zen, direct])["USD"], Decimal::new(7, 0));
    }
}
