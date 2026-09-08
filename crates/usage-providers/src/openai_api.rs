use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use std::{collections::BTreeSet, str::FromStr, time::Duration};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, MoneyBalance, ProviderAdapter,
    ProviderError, Snapshot, UsageRecord,
};
use uuid::Uuid;

use crate::http::{fetch_json, key_fingerprint};

pub const OPENAI_API_HOSTS: &[&str] = &["api.openai.com"];
pub const OPENAI_API_CONNECTOR_VERSION: &str = "openai-api-connector-v1";
pub const OPENAI_COSTS_PARSER_VERSION: &str = "openai-costs-parser-v1";
const COSTS_PATH: &str = "/v1/organization/costs";

#[derive(Default)]
pub struct OpenAiApiAdapter {
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCosts {
    pub records: Vec<CostDraft>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostDraft {
    pub source_record_id: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub amount: Decimal,
    pub currency: String,
    pub project_id: Option<String>,
    pub line_item: Option<String>,
}

/// Pure defensive parser for the OpenAI organization costs page.
/// Unknown or missing fields never become fabricated money:
/// incomplete rows are skipped with a warning; a missing `data`
/// array is a schema-change error, not an empty bill.
pub fn parse_costs(value: &serde_json::Value, source: &str) -> Result<ParsedCosts, ProviderError> {
    let data = value
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ProviderError::Parse("openai_costs_schema_changed".into()))?;
    let mut out = ParsedCosts {
        records: vec![],
        truncated: value
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        warnings: vec![],
    };
    for (bucket_idx, bucket) in data.iter().enumerate() {
        let start = bucket
            .get("start_time")
            .and_then(|v| v.as_i64())
            .and_then(|t| Utc.timestamp_opt(t, 0).single());
        let end = bucket
            .get("end_time")
            .and_then(|v| v.as_i64())
            .and_then(|t| Utc.timestamp_opt(t, 0).single());
        let (Some(start), Some(end)) = (start, end) else {
            out.warnings
                .push(format!("bucket_{bucket_idx}_missing_window"));
            continue;
        };
        let results = bucket
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for (row_idx, row) in results.iter().enumerate() {
            let amount_value = row
                .get("amount")
                .and_then(|a| a.get("value"))
                .and_then(|v| v.as_f64());
            let currency = row
                .get("amount")
                .and_then(|a| a.get("currency"))
                .and_then(|v| v.as_str())
                .map(|c| c.to_ascii_uppercase());
            let (Some(amount_value), Some(currency)) = (amount_value, currency) else {
                out.warnings
                    .push(format!("bucket_{bucket_idx}_row_{row_idx}_incomplete"));
                continue;
            };
            let Ok(amount) = Decimal::from_str(&amount_value.to_string()) else {
                out.warnings
                    .push(format!("bucket_{bucket_idx}_row_{row_idx}_bad_amount"));
                continue;
            };
            let project_id = row
                .get("project_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let line_item = row
                .get("line_item")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            out.records.push(CostDraft {
                source_record_id: format!(
                    "openai-costs:{start}:{end}:{}:{}",
                    project_id.as_deref().unwrap_or("org"),
                    line_item.as_deref().unwrap_or("all")
                ),
                period_start: start,
                period_end: end,
                amount,
                currency,
                project_id,
                line_item,
            });
        }
    }
    let _ = source;
    Ok(out)
}

impl OpenAiApiAdapter {
    fn base(&self) -> String {
        self.base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com".into())
    }

    pub fn identity_for(&self, api_key: &str) -> String {
        key_fingerprint("openai-api", api_key)
    }

    pub async fn refresh_with_key(
        &self,
        account: &Account,
        api_key: &str,
        timeout: Duration,
    ) -> Result<Snapshot, ProviderError> {
        if api_key.trim().is_empty() {
            return Err(ProviderError::AuthenticationRequired);
        }
        let end = Utc::now();
        let start = end - chrono::Duration::days(30);
        let url = format!(
            "{}{COSTS_PATH}?start_time={}&end_time={}&bucket_width=1d&limit=180",
            self.base(),
            start.timestamp(),
            end.timestamp()
        );
        let (_status, value) = fetch_json(&url, api_key, OPENAI_API_HOSTS, timeout).await?;
        let parsed = parse_costs(&value, "openai-api-costs-v1")?;
        let fetched_at = Utc::now();
        let observed_at = parsed
            .records
            .iter()
            .map(|r| r.period_end)
            .max()
            .unwrap_or(start);
        let age = (fetched_at - observed_at).num_seconds().max(0) as u64;
        let usage_records: Vec<UsageRecord> = vec![];
        let _ = usage_records;
        Ok(Snapshot {
            account_id: account.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
            plan_label: None,
            capabilities: self.capabilities(),
            quotas: vec![],
            balances: vec![],
            observed_at: Some(observed_at),
            fetched_at,
            connection_state: ConnectionState::Connected,
            freshness: if age < 3600 {
                Freshness::Fresh
            } else {
                Freshness::Stale(age)
            },
            coverage: if parsed.truncated {
                Coverage::Partial
            } else {
                Coverage::ProviderDelayed
            },
        })
    }

    pub async fn import_costs_with_key(
        &self,
        account: &Account,
        api_key: &str,
        timeout: Duration,
    ) -> Result<(Vec<usage_core::CostRecord>, Vec<String>), ProviderError> {
        let end = Utc::now();
        let start = end - chrono::Duration::days(30);
        let url = format!(
            "{}{COSTS_PATH}?start_time={}&end_time={}&bucket_width=1d&limit=180",
            self.base(),
            start.timestamp(),
            end.timestamp()
        );
        let (_status, value) = fetch_json(&url, api_key, OPENAI_API_HOSTS, timeout).await?;
        let parsed = parse_costs(&value, "openai-api-costs-v1")?;
        let records = parsed
            .records
            .into_iter()
            .map(|draft| usage_core::CostRecord {
                account_id: account.id,
                product_id: "openai-api".into(),
                billing_scope_id: draft.project_id,
                source_record_id: draft.source_record_id,
                period_start: draft.period_start,
                period_end: draft.period_end,
                amount_decimal: draft.amount,
                currency: draft.currency,
                kind: usage_core::CostKind::Reported,
                source: "openai-api-costs-v1".into(),
                coverage: Coverage::ProviderDelayed,
            })
            .collect();
        Ok((records, parsed.warnings))
    }

    pub fn balances_from(&self, _account_id: Uuid) -> Vec<MoneyBalance> {
        // No documented stable balance endpoint is verified for this adapter;
        // absence is reported as unknown, never as zero.
        vec![]
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiApiAdapter {
    fn provider_id(&self) -> &'static str {
        "openai"
    }
    fn product_id(&self) -> &'static str {
        "openai-api"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        [
            Capability::ApiCostReported,
            Capability::ProjectBreakdown,
            Capability::HistoryRemote,
            Capability::MultiAccount,
        ]
        .into_iter()
        .collect()
    }
    fn allowed_hosts(&self) -> &'static [&'static str] {
        OPENAI_API_HOSTS
    }
    async fn refresh(&self, _account: &Account) -> Result<Snapshot, ProviderError> {
        Err(ProviderError::AuthenticationRequired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_realistic_cost_page() {
        let value = serde_json::json!({
            "object": "page",
            "has_more": false,
            "data": [{
                "object": "bucket",
                "start_time": 1_725_000_000,
                "end_time": 1_725_086_400,
                "results": [
                    {"object": "organization_costs_result",
                     "amount": {"value": 4.82, "currency": "usd"},
                     "line_item": "gpt-4o",
                     "project_id": "proj_1"},
                    {"object": "organization_costs_result",
                     "amount": {"value": 1.5, "currency": "usd"},
                     "line_item": null,
                     "project_id": null},
                    {"object": "organization_costs_result",
                     "amount": null,
                     "line_item": "broken"}
                ]
            }]
        });
        let parsed = parse_costs(&value, "test").unwrap();
        assert_eq!(parsed.records.len(), 2);
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(parsed.records[0].currency, "USD");
        assert_eq!(parsed.records[0].project_id.as_deref(), Some("proj_1"));
        assert!(parsed.records[1].project_id.is_none());
    }
    #[test]
    fn missing_data_is_schema_change_not_empty_bill() {
        let value = serde_json::json!({"object": "page"});
        assert!(matches!(
            parse_costs(&value, "test"),
            Err(ProviderError::Parse(_))
        ));
    }
    #[test]
    fn capabilities_declare_reported_cost_only() {
        let caps = OpenAiApiAdapter::default().capabilities();
        assert!(caps.contains(&Capability::ApiCostReported));
        assert!(!caps.contains(&Capability::ApiCostEstimated));
        assert!(!caps.contains(&Capability::Balance));
    }
}
