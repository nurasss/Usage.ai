use chrono::{DateTime, Duration, Local, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;
use usage_storage::Storage;
use uuid::Uuid;

/// Runtime-owned segment data. The application shell only maps these values
/// to its transport DTOs; product labels and colors come from descriptors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Segment {
    pub label: String,
    pub value: u64,
    pub color: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoneyTotal {
    pub amount: String,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCostInput {
    pub account_id: Uuid,
    pub product_id: String,
    pub provider_name: String,
    pub alias: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderCostAggregate {
    pub account_id: String,
    pub product_id: String,
    pub label: String,
    pub reported: Vec<MoneyTotal>,
    pub estimated: Vec<MoneyTotal>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Aggregates {
    pub overview: HashMap<String, Vec<Segment>>,
    pub model_breakdown: HashMap<String, Vec<Segment>>,
    pub account_breakdown: HashMap<String, Vec<Segment>>,
    pub project_breakdown: HashMap<String, Vec<Segment>>,
    pub costs_by_period: HashMap<String, Vec<ProviderCostAggregate>>,
    pub unverified_overview: HashMap<String, Vec<Segment>>,
    pub excluded_unverified_count: HashMap<String, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountUsageAndCost {
    pub tokens_today: Option<u64>,
    pub reported_cost_today: Option<MoneyTotal>,
    pub estimated_cost_today: Option<MoneyTotal>,
}

#[derive(Clone, Copy)]
struct PeriodRange {
    key: &'static str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

/// Prepare all storage-backed period aggregates in the runtime layer.
/// Tauri must not own provider/product aggregation policy.
/// UnverifiedSemantics are strictly excluded from authoritative overview/breakdowns
/// and exposed separately via unverified_overview and excluded_unverified_count.
pub fn build_aggregates(storage: &Storage, providers: &[ProviderCostInput]) -> Aggregates {
    let ranges = period_ranges();
    let mut aggregates = Aggregates::default();
    for range in ranges {
        let key = range.key.to_string();
        aggregates.overview.insert(
            key.clone(),
            storage
                .verified_token_totals_between(range.start, range.end)
                .unwrap_or_default()
                .into_iter()
                .map(|(product, value)| Segment {
                    label: product_label(&product),
                    value,
                    color: product_color(&product),
                })
                .collect(),
        );
        aggregates.model_breakdown.insert(
            key.clone(),
            storage
                .verified_model_totals_between(range.start, range.end)
                .unwrap_or_default()
                .into_iter()
                .map(|(model, value)| Segment {
                    label: model
                        .filter(|model| !model.trim().is_empty())
                        .unwrap_or_else(|| "Модель не определена".into()),
                    value,
                    color: "#8ea2ff".into(),
                })
                .collect(),
        );
        aggregates.account_breakdown.insert(
            key.clone(),
            storage
                .verified_account_product_totals_between(range.start, range.end)
                .unwrap_or_default()
                .into_iter()
                .map(|(alias, _id, product, value)| Segment {
                    label: format!("{alias} · {}", product_label(&product)),
                    value,
                    color: product_color(&product),
                })
                .collect(),
        );
        aggregates.project_breakdown.insert(
            key.clone(),
            storage
                .verified_project_totals_between(range.start, range.end)
                .unwrap_or_default()
                .into_iter()
                .map(|(scope, value)| Segment {
                    label: scope
                        .filter(|scope| !scope.trim().is_empty())
                        .unwrap_or_else(|| "Без проекта".into()),
                    value,
                    color: "#5fb3a1".into(),
                })
                .collect(),
        );
        aggregates.costs_by_period.insert(
            key.clone(),
            build_costs_for_range(storage, providers, range.start, range.end),
        );
        aggregates.unverified_overview.insert(
            key.clone(),
            storage
                .unverified_token_totals_between(range.start, range.end)
                .unwrap_or_default()
                .into_iter()
                .map(|(product, value)| Segment {
                    label: format!("{} (неподтверждённые)", product_label(&product)),
                    value,
                    color: "#e0a040".into(),
                })
                .collect(),
        );
        aggregates.excluded_unverified_count.insert(
            key,
            storage
                .excluded_unverified_count_between(range.start, range.end)
                .unwrap_or_default(),
        );
    }
    aggregates
}

pub fn account_usage_and_cost_today(
    storage: &Storage,
    account_id: Uuid,
    product_id: &str,
) -> AccountUsageAndCost {
    let today = Local::now().date_naive();
    let start_local = local_midnight(today);
    let end_local = local_midnight(today.succ_opt().unwrap_or(today));
    let (start, end) = (
        start_local.with_timezone(&Utc),
        end_local.with_timezone(&Utc),
    );

    let mut result = AccountUsageAndCost::default();

    if let Ok(tokens) =
        storage.authoritative_tokens_for_account_between(account_id, product_id, start, end)
    {
        if tokens > 0 {
            result.tokens_today = Some(tokens);
        }
    }

    if let Ok(costs) = storage.cost_records_between(start, end) {
        let mut reported: HashMap<String, Decimal> = HashMap::new();
        let mut estimated: HashMap<String, Decimal> = HashMap::new();
        for record in costs.iter().filter(|r| {
            r.coverage.is_authoritative()
                && r.account_id == account_id
                && r.product_id == product_id
        }) {
            let target = if record.kind == usage_core::CostKind::Reported {
                &mut reported
            } else {
                &mut estimated
            };
            *target
                .entry(record.currency.clone())
                .or_insert(Decimal::ZERO) += record.amount_decimal;
        }
        for currency in reported.keys().cloned().collect::<Vec<_>>() {
            estimated.remove(&currency);
        }
        if reported.len() == 1 {
            let (currency, amount) = reported.into_iter().next().unwrap();
            result.reported_cost_today = Some(MoneyTotal {
                amount: amount.to_string(),
                currency,
            });
        }
        if estimated.len() == 1 {
            let (currency, amount) = estimated.into_iter().next().unwrap();
            result.estimated_cost_today = Some(MoneyTotal {
                amount: amount.to_string(),
                currency,
            });
        }
    }

    result
}

fn build_costs_for_range(
    storage: &Storage,
    providers: &[ProviderCostInput],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<ProviderCostAggregate> {
    let costs = storage.cost_records_between(start, end).unwrap_or_default();
    let mut rows = Vec::new();
    for provider in providers {
        let mut reported: HashMap<String, Decimal> = HashMap::new();
        let mut estimated: HashMap<String, Decimal> = HashMap::new();
        for record in costs.iter().filter(|record| {
            record.coverage.is_authoritative()
                && record.account_id == provider.account_id
                && record.product_id == provider.product_id
        }) {
            let target = if record.kind == usage_core::CostKind::Reported {
                &mut reported
            } else {
                &mut estimated
            };
            *target
                .entry(record.currency.clone())
                .or_insert(Decimal::ZERO) += record.amount_decimal;
        }
        for currency in reported.keys().cloned().collect::<Vec<_>>() {
            estimated.remove(&currency);
        }
        if reported.is_empty() && estimated.is_empty() {
            continue;
        }
        let mut reported_list = money_totals(reported);
        let mut estimated_list = money_totals(estimated);
        reported_list.sort_by(|a, b| a.currency.cmp(&b.currency));
        estimated_list.sort_by(|a, b| a.currency.cmp(&b.currency));
        rows.push(ProviderCostAggregate {
            account_id: provider.account_id.to_string(),
            product_id: provider.product_id.clone(),
            label: format!("{} · {}", provider.provider_name, provider.alias),
            reported: reported_list,
            estimated: estimated_list,
        });
    }
    rows
}

fn money_totals(values: HashMap<String, Decimal>) -> Vec<MoneyTotal> {
    values
        .into_iter()
        .map(|(currency, amount)| MoneyTotal {
            amount: amount.to_string(),
            currency,
        })
        .collect()
}

fn descriptor_for_product(
    product: &str,
) -> Option<&'static usage_providers::descriptor::ProductDescriptor> {
    usage_providers::descriptor::all_descriptors()
        .iter()
        .find(|descriptor| descriptor.product_id == product)
}

fn product_label(product: &str) -> String {
    descriptor_for_product(product)
        .map(|descriptor| descriptor.product_name.to_string())
        .unwrap_or_else(|| product.to_string())
}

fn product_color(product: &str) -> String {
    descriptor_for_product(product)
        .map(|descriptor| descriptor.color.to_string())
        .unwrap_or_else(|| "#46c9a7".into())
}

fn local_midnight(date: chrono::NaiveDate) -> DateTime<Local> {
    use chrono::TimeZone;
    let naive = date.and_hms_opt(0, 0, 0).expect("midnight is valid");
    Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| Local.from_local_datetime(&naive).latest())
        .unwrap_or_else(Local::now)
}

fn period_ranges() -> Vec<PeriodRange> {
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
    [
        ("today", today, tomorrow),
        ("yesterday", yesterday, today),
        ("7days", week, tomorrow),
        ("30days", month, tomorrow),
    ]
    .into_iter()
    .map(|(key, start, end)| PeriodRange {
        key,
        start: start.with_timezone(&Utc),
        end: end.with_timezone(&Utc),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_presentation_is_descriptor_driven() {
        assert_eq!(product_label("codex"), "Codex");
        assert_eq!(product_color("claude-code"), "#b895ff");
        assert_eq!(product_label("future-product"), "future-product");
    }

    #[test]
    fn unverified_records_are_excluded_from_authoritative_overview_and_breakdowns() {
        let mut storage = Storage::in_memory().unwrap();
        let acc_id = Uuid::new_v4();
        storage
            .upsert_account(&usage_core::Account {
                id: acc_id,
                provider_id: "openai".into(),
                external_identity: None,
                label: "Test".into(),
                connection_ref: None,
                lifecycle: usage_core::AccountLifecycle::Active,
            })
            .unwrap();

        let now = Utc::now();
        // Insert verified record
        storage
            .upsert_usage(&usage_core::UsageRecord {
                account_id: acc_id,
                product_id: "openai-api".into(),
                billing_scope_id: Some("scope-verified".into()),
                source_record_id: "rec-verified".into(),
                period_start: now,
                period_end: now,
                model: Some("gpt-4o".into()),
                input_tokens: Some(100),
                output_tokens: Some(50),
                cached_tokens: None,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                reasoning_tokens: None,
                total_tokens: Some(150),
                requests: Some(1),
                source: "api".into(),
                coverage: usage_core::Coverage::Complete,
                connection_id: None,
                file_id: None,
                file_generation: None,
            })
            .unwrap();

        // Insert unverified records. The second one uses a different
        // non-authoritative coverage value to ensure SQL follows the domain
        // predicate instead of checking one enum variant by name.
        let unverified = usage_core::UsageRecord {
            account_id: acc_id,
            product_id: "codex".into(),
            billing_scope_id: Some("scope-unverified".into()),
            source_record_id: "rec-unverified".into(),
            period_start: now,
            period_end: now,
            model: Some("gpt-codex".into()),
            input_tokens: Some(200),
            output_tokens: Some(100),
            cached_tokens: None,
            cache_creation_tokens: None,
            cache_read_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(300),
            requests: Some(1),
            source: "local".into(),
            coverage: usage_core::Coverage::UnverifiedSemantics,
            connection_id: None,
            file_id: None,
            file_generation: None,
        };
        storage.upsert_usage(&unverified).unwrap();
        let mut unknown = unverified.clone();
        unknown.source_record_id = "rec-unknown".into();
        unknown.total_tokens = Some(400);
        unknown.coverage = usage_core::Coverage::Unknown;
        storage.upsert_usage(&unknown).unwrap();

        let aggregates = build_aggregates(&storage, &[]);

        let today_overview = aggregates.overview.get("today").unwrap();
        // Authoritative overview MUST only contain verified product (150), never unverified
        assert_eq!(today_overview.len(), 1);
        assert_eq!(today_overview[0].label, "API");
        assert_eq!(today_overview[0].value, 150);

        // Unverified overview MUST contain unverified product (300)
        let today_unverified = aggregates.unverified_overview.get("today").unwrap();
        assert_eq!(today_unverified.len(), 1);
        assert_eq!(today_unverified[0].value, 700);

        // Excluded unverified count MUST reflect excluded source
        let excluded_count = aggregates.excluded_unverified_count.get("today").unwrap();
        assert_eq!(*excluded_count, 1);
    }

    #[test]
    fn provider_facing_totals_exclude_non_authoritative_usage_and_costs() {
        let mut storage = Storage::in_memory().unwrap();
        let account_id = Uuid::new_v4();
        let other_account_id = Uuid::new_v4();
        let today = Local::now().date_naive();
        let start = local_midnight(today).with_timezone(&Utc);
        let observed_at = start + Duration::hours(1);

        let make_usage = |account_id: Uuid,
                          product_id: &str,
                          source_record_id: &str,
                          total_tokens: u64,
                          coverage: usage_core::Coverage|
         -> usage_core::UsageRecord {
            usage_core::UsageRecord {
                account_id,
                product_id: product_id.into(),
                billing_scope_id: None,
                source_record_id: source_record_id.into(),
                period_start: observed_at,
                period_end: observed_at,
                model: Some("gpt-codex".into()),
                input_tokens: Some(total_tokens),
                output_tokens: None,
                cached_tokens: None,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                reasoning_tokens: None,
                total_tokens: Some(total_tokens),
                requests: Some(1),
                source: "test".into(),
                coverage,
                connection_id: None,
                file_id: None,
                file_generation: None,
            }
        };
        for (source_record_id, total_tokens, coverage) in [
            ("complete", 100, usage_core::Coverage::Complete),
            ("partial", 50, usage_core::Coverage::Partial),
            ("unknown", 1000, usage_core::Coverage::Unknown),
            ("local-only", 2000, usage_core::Coverage::LocalClientOnly),
            (
                "unverified-semantics",
                4000,
                usage_core::Coverage::UnverifiedSemantics,
            ),
        ] {
            storage
                .upsert_usage(&make_usage(
                    account_id,
                    "codex",
                    source_record_id,
                    total_tokens,
                    coverage,
                ))
                .unwrap();
        }
        // A different account and product must never bleed into the target
        // provider-facing DTO totals, while remaining visible to global
        // authoritative analytics.
        storage
            .upsert_usage(&make_usage(
                other_account_id,
                "codex",
                "other-account",
                700,
                usage_core::Coverage::Complete,
            ))
            .unwrap();
        storage
            .upsert_usage(&make_usage(
                account_id,
                "claude-code",
                "other-product",
                800,
                usage_core::Coverage::Complete,
            ))
            .unwrap();

        assert_eq!(
            storage
                .tokens_for_account_between(account_id, "codex", start, start + Duration::days(1))
                .unwrap(),
            7150,
            "raw/local history keeps every coverage class"
        );
        assert_eq!(
            storage
                .authoritative_tokens_for_account_between(
                    account_id,
                    "codex",
                    start,
                    start + Duration::days(1),
                )
                .unwrap(),
            150,
            "provider-facing usage accepts only Complete + Partial"
        );

        let make_cost = |account_id: Uuid,
                         product_id: &str,
                         source_record_id: &str,
                         amount_decimal: Decimal,
                         coverage: usage_core::Coverage|
         -> usage_core::CostRecord {
            usage_core::CostRecord {
                account_id,
                product_id: product_id.into(),
                billing_scope_id: None,
                source_record_id: source_record_id.into(),
                period_start: observed_at,
                period_end: observed_at,
                amount_decimal,
                currency: "USD".into(),
                kind: usage_core::CostKind::Reported,
                source: "test".into(),
                coverage,
            }
        };
        for (source_record_id, amount_decimal, coverage) in [
            (
                "cost-complete",
                Decimal::new(100, 2),
                usage_core::Coverage::Complete,
            ),
            (
                "cost-partial",
                Decimal::new(50, 2),
                usage_core::Coverage::Partial,
            ),
            (
                "cost-unknown",
                Decimal::new(1000, 2),
                usage_core::Coverage::Unknown,
            ),
            (
                "cost-local-only",
                Decimal::new(2000, 2),
                usage_core::Coverage::LocalClientOnly,
            ),
            (
                "cost-unverified-semantics",
                Decimal::new(4000, 2),
                usage_core::Coverage::UnverifiedSemantics,
            ),
        ] {
            storage
                .upsert_cost(&make_cost(
                    account_id,
                    "codex",
                    source_record_id,
                    amount_decimal,
                    coverage,
                ))
                .unwrap();
        }
        storage
            .upsert_cost(&make_cost(
                other_account_id,
                "codex",
                "cost-other-account",
                Decimal::new(9999, 2),
                usage_core::Coverage::Complete,
            ))
            .unwrap();
        storage
            .upsert_cost(&make_cost(
                account_id,
                "claude-code",
                "cost-other-product",
                Decimal::new(7777, 2),
                usage_core::Coverage::Complete,
            ))
            .unwrap();

        let raw_target_cost: Decimal = storage
            .cost_records_between(start, start + Duration::days(1))
            .unwrap()
            .into_iter()
            .filter(|record| record.account_id == account_id && record.product_id == "codex")
            .map(|record| record.amount_decimal)
            .sum();
        assert_eq!(
            raw_target_cost,
            Decimal::new(7150, 2),
            "raw/local cost history keeps non-authoritative amounts exactly"
        );

        let today_totals = account_usage_and_cost_today(&storage, account_id, "codex");
        assert_eq!(today_totals.tokens_today, Some(150));
        assert_eq!(
            today_totals.reported_cost_today.unwrap().amount,
            Decimal::new(150, 2).to_string(),
            "provider-facing cost uses exact Decimal arithmetic"
        );

        let providers = [
            ProviderCostInput {
                account_id,
                product_id: "codex".into(),
                provider_name: "OpenAI".into(),
                alias: "Test".into(),
            },
            ProviderCostInput {
                account_id: other_account_id,
                product_id: "codex".into(),
                provider_name: "OpenAI".into(),
                alias: "Other account".into(),
            },
            ProviderCostInput {
                account_id,
                product_id: "claude-code".into(),
                provider_name: "Anthropic".into(),
                alias: "Other product".into(),
            },
        ];
        let aggregates = build_aggregates(&storage, &providers);
        let today_costs = aggregates.costs_by_period.get("today").unwrap();
        let target_costs = today_costs
            .iter()
            .find(|row| row.account_id == account_id.to_string() && row.product_id == "codex")
            .unwrap();
        assert_eq!(
            target_costs.reported[0].amount,
            Decimal::new(150, 2).to_string()
        );
        assert_eq!(
            today_costs.len(),
            3,
            "each account/product remains isolated"
        );

        let unverified = aggregates.unverified_overview.get("today").unwrap();
        assert!(unverified.iter().any(|segment| {
            segment.label == "Codex (неподтверждённые)" && segment.value == 7000
        }));
    }
}
