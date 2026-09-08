use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use std::{collections::BTreeSet, str::FromStr, time::Duration};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, MoneyBalance, ProviderAdapter,
    ProviderError, Snapshot,
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
    /// Opaque pagination cursor when the API offers one and more
    /// pages remain; `None` means the result is complete.
    pub next_cursor: Option<String>,
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
    // Only an explicit cursor field is followed. Bucket/item ids are
    // never guessed as pagination cursors: a wrong `starting_after`
    // would silently skip or duplicate billing rows.
    let mut out = ParsedCosts {
        records: vec![],
        truncated: value
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        next_cursor: ["next_cursor", "starting_after"]
            .iter()
            .filter_map(|key| value.get(*key)?.as_str())
            .map(|s| s.to_string())
            .next(),
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
                // Identity includes currency: two rows for the same
                // bucket/scope in different currencies must not
                // overwrite each other. Amount is excluded on purpose:
                // a corrected re-report must update, not duplicate.
                source_record_id: format!(
                    "openai-costs:{start}:{end}:{currency}:{}:{}",
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

    fn page_url(&self, start: DateTime<Utc>, end: DateTime<Utc>, cursor: Option<&str>) -> String {
        let mut url = format!(
            "{}{COSTS_PATH}?start_time={}&end_time={}&bucket_width=1d&limit=180",
            self.base(),
            start.timestamp(),
            end.timestamp()
        );
        if let Some(cursor) = cursor {
            use std::fmt::Write;
            let _ = write!(url, "&starting_after={cursor}");
        }
        url
    }

    /// One logical fetch set: all pages share a single parsed payload
    /// that produces the snapshot and the cost records together (P0-1).
    /// Pagination is bounded; an unfinished tail degrades coverage to
    /// `Partial` instead of silently truncating the bill.
    pub async fn fetch_report(
        &self,
        account: &Account,
        api_key: &str,
        timeout: Duration,
    ) -> Result<CostsReport, ProviderError> {
        if api_key.trim().is_empty() {
            return Err(ProviderError::AuthenticationRequired);
        }
        let end = Utc::now();
        let start = end - chrono::Duration::days(30);
        let mut drafts = vec![];
        let mut warnings = vec![];
        let mut cursor: Option<String> = None;
        let mut truncated = false;
        let mut pages = 0usize;
        loop {
            let url = self.page_url(start, end, cursor.as_deref());
            let (_status, value) = fetch_json(&url, api_key, OPENAI_API_HOSTS, timeout).await?;
            let parsed = parse_costs(&value, "openai-api-costs-v1")?;
            warnings.extend(parsed.warnings);
            drafts.extend(parsed.records);
            pages += 1;
            if !parsed.truncated {
                break;
            }
            match parsed.next_cursor {
                Some(next) if pages < usage_host::policy::MAX_API_PAGES => cursor = Some(next),
                _ => {
                    truncated = true;
                    warnings.push("pagination_bounded_partial".into());
                    break;
                }
            }
        }
        Ok(build_report(
            account,
            &self.capabilities(),
            drafts,
            warnings,
            truncated,
            start,
        ))
    }

    pub async fn refresh_with_key(
        &self,
        account: &Account,
        api_key: &str,
        timeout: Duration,
    ) -> Result<Snapshot, ProviderError> {
        Ok(self.fetch_report(account, api_key, timeout).await?.snapshot)
    }

    pub async fn import_costs_with_key(
        &self,
        account: &Account,
        api_key: &str,
        timeout: Duration,
    ) -> Result<(Vec<usage_core::CostRecord>, Vec<String>), ProviderError> {
        let report = self.fetch_report(account, api_key, timeout).await?;
        Ok((report.costs, report.warnings))
    }

    pub fn balances_from(&self, _account_id: Uuid) -> Vec<MoneyBalance> {
        // No documented stable balance endpoint is verified for this adapter;
        // absence is reported as unknown, never as zero.
        vec![]
    }
}

pub struct CostsReport {
    pub snapshot: Snapshot,
    pub costs: Vec<usage_core::CostRecord>,
    pub warnings: Vec<String>,
    pub truncated: bool,
}

fn build_report(
    account: &Account,
    capabilities: &BTreeSet<Capability>,
    drafts: Vec<CostDraft>,
    warnings: Vec<String>,
    truncated: bool,
    start: DateTime<Utc>,
) -> CostsReport {
    let observed_at = drafts.iter().map(|r| r.period_end).max().unwrap_or(start);
    let fetched_at = Utc::now();
    let age = (fetched_at - observed_at).num_seconds().max(0) as u64;
    let costs = drafts
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
            coverage: if truncated {
                Coverage::Partial
            } else {
                Coverage::ProviderDelayed
            },
        })
        .collect();
    CostsReport {
        snapshot: Snapshot {
            account_id: account.id,
            provider_id: "openai".into(),
            product_id: "openai-api".into(),
            plan_label: None,
            capabilities: capabilities.clone(),
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
            coverage: if truncated {
                Coverage::Partial
            } else {
                Coverage::ProviderDelayed
            },
        },
        costs,
        warnings,
        truncated,
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
        crate::descriptor::OPENAI_API_CAPS.iter().copied().collect()
    }
    fn allowed_hosts(&self) -> &'static [&'static str] {
        OPENAI_API_HOSTS
    }
    async fn refresh(&self, _account: &Account) -> Result<Snapshot, ProviderError> {
        Err(ProviderError::AuthenticationRequired)
    }
}

/// Strategy wrapper: OpenAI organization costs over scoped HostServices.
/// The secret arrives from the runtime (Keychain) per call and is never
/// stored, logged or forwarded outside the allow-listed host.
pub struct OpenAiCostsStrategy {
    pub base_url: Option<String>,
}

impl OpenAiCostsStrategy {
    fn page_url(&self, start: DateTime<Utc>, end: DateTime<Utc>, cursor: Option<&str>) -> String {
        let base = self
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com".into());
        let mut url = format!(
            "{base}{COSTS_PATH}?start_time={}&end_time={}&bucket_width=1d&limit=180",
            start.timestamp(),
            end.timestamp()
        );
        if let Some(cursor) = cursor {
            use std::fmt::Write;
            let _ = write!(url, "&starting_after={cursor}");
        }
        url
    }
}

#[async_trait]
impl crate::strategy::FetchStrategy for OpenAiCostsStrategy {
    fn source_id(&self) -> crate::strategy::SourceId {
        crate::strategy::SourceId("openai-api-costs")
    }
    fn classification(&self) -> crate::strategy::SourceClassification {
        crate::strategy::SourceClassification::OfficialApi
    }
    async fn availability(
        &self,
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> crate::strategy::Availability {
        if ctx.secret.as_deref().is_some_and(|s| !s.is_empty()) {
            crate::strategy::Availability::Ready
        } else {
            crate::strategy::Availability::NotConfigured
        }
    }
    async fn fetch(
        &self,
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> Result<crate::strategy::FetchPayload, crate::strategy::SourceError> {
        use crate::strategy::SourceError;
        let Some(secret) = ctx.secret.as_deref() else {
            return Err(SourceError::AuthenticationRequired);
        };
        if secret.is_empty() {
            return Err(SourceError::AuthenticationRequired);
        }
        let end = ctx.hosts.clock.now();
        let start = end - chrono::Duration::days(30);
        let mut drafts = vec![];
        let mut warnings = vec![];
        let mut cursor: Option<String> = None;
        let mut pages = 0usize;
        loop {
            let url = self.page_url(start, end, cursor.as_deref());
            let response = ctx
                .hosts
                .http
                .get_json(usage_host::HttpJsonRequest {
                    url: &url,
                    bearer: Some(secret),
                    allowed_hosts: OPENAI_API_HOSTS,
                    timeout: ctx.timeout,
                    max_bytes: usage_host::policy::MAX_HTTP_BYTES,
                    user_agent: usage_host::policy::USER_AGENT,
                })
                .await
                .map_err(|e| match &e {
                    usage_host::HostError::RateLimited(retry_after_secs) => {
                        SourceError::RateLimited {
                            retry_after_secs: *retry_after_secs,
                        }
                    }
                    other => crate::strategy::map_host_error(other),
                })?;
            let body = response.body.ok_or(SourceError::Parse {
                schema: openai_api_schema(),
            })?;
            let parsed =
                parse_costs(&body, "openai-api-costs-v1").map_err(|_| SourceError::Parse {
                    schema: openai_api_schema(),
                })?;
            warnings.extend(parsed.warnings);
            drafts.extend(parsed.records);
            pages += 1;
            if !parsed.truncated {
                break;
            }
            match parsed.next_cursor {
                Some(next) if pages < usage_host::policy::MAX_API_PAGES => cursor = Some(next),
                _ => {
                    warnings.push("pagination_bounded_partial".into());
                    break;
                }
            }
        }
        let truncated = warnings.iter().any(|w| w == "pagination_bounded_partial");
        let account = ctx.account;
        let caps: BTreeSet<Capability> =
            crate::descriptor::OPENAI_API_CAPS.iter().copied().collect();
        let report = build_report(account, &caps, drafts, warnings, truncated, start);
        let observed_at = report.snapshot.observed_at;
        Ok(crate::strategy::FetchPayload {
            snapshot: Some(report.snapshot),
            usage: vec![],
            costs: report.costs,
            balances: vec![],
            warnings: report.warnings,
            schema_fingerprint: openai_api_schema(),
            coverage: if truncated {
                Coverage::Partial
            } else {
                Coverage::ProviderDelayed
            },
            observed_at,
        })
    }
}

pub fn openai_api_schema() -> &'static str {
    "openai-costs-page-v1"
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

    struct ScriptHttp {
        pages: std::sync::Mutex<Vec<serde_json::Value>>,
        calls: std::sync::Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl usage_host::HttpHost for ScriptHttp {
        async fn get_json(
            &self,
            _req: usage_host::HttpJsonRequest<'_>,
        ) -> Result<usage_host::HttpJsonResponse, usage_host::HostError> {
            *self.calls.lock().unwrap() += 1;
            let body = self.pages.lock().unwrap().remove(0);
            Ok(usage_host::HttpJsonResponse {
                status: 200,
                retry_after_secs: None,
                body: Some(body),
            })
        }
    }

    fn test_hosts(
        pages: Vec<serde_json::Value>,
    ) -> (usage_host::Hosts, std::sync::Arc<ScriptHttp>) {
        use std::sync::Arc;
        let http = Arc::new(ScriptHttp {
            pages: std::sync::Mutex::new(pages),
            calls: std::sync::Mutex::new(0),
        });
        let hosts = usage_host::Hosts {
            clock: Arc::new(usage_host::SystemClock),
            logger: Arc::new(usage_host::NullLogger),
            keychain: Arc::new(usage_host::MemoryKeychain::default()),
            http: http.clone(),
            files: Arc::new(usage_host::ScopedFiles),
            network: Arc::new(usage_host::ObservedNetwork::default()),
        };
        (hosts, http)
    }

    fn cost_page(rows: &[(&str, f64)], has_more: bool, cursor: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "object": "page",
            "has_more": has_more,
            "next_cursor": cursor,
            "data": [{
                "start_time": 1_725_000_000,
                "end_time": 1_725_086_400,
                "results": rows.iter().map(|(item, amount)| {
                    serde_json::json!({"amount": {"value": amount, "currency": "usd"}, "line_item": item})
                }).collect::<Vec<_>>()
            }]
        })
    }

    #[tokio::test]
    async fn strategy_fetches_all_pages_once_per_refresh() {
        use crate::strategy::{FetchContext, FetchStrategy};
        let (hosts, http) = test_hosts(vec![
            cost_page(&[("a", 1.0)], true, Some("c1")),
            cost_page(&[("b", 2.0)], false, None),
        ]);
        let account = Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "API".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        let ctx = FetchContext {
            account: &account,
            hosts: &hosts,
            timeout: Duration::from_secs(5),
            custom_root: None,
            secret: Some(b"sk-test".to_vec()),
        };
        let payload = OpenAiCostsStrategy { base_url: None }
            .fetch(&ctx)
            .await
            .unwrap();
        // One logical refresh produced both snapshot and cost rows
        // from a single shared parsed payload (P0-1).
        assert!(payload.snapshot.is_some());
        assert_eq!(payload.costs.len(), 2);
        assert_eq!(*http.calls.lock().unwrap(), 2);
        assert_eq!(payload.coverage, Coverage::ProviderDelayed);
    }

    #[tokio::test]
    async fn endless_pagination_is_bounded_and_partial() {
        use crate::strategy::{FetchContext, FetchStrategy};
        let endless: Vec<serde_json::Value> = (0..20)
            .map(|_| cost_page(&[("a", 1.0)], true, Some("again")))
            .collect();
        let (hosts, http) = test_hosts(endless);
        let account = Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "API".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        let ctx = FetchContext {
            account: &account,
            hosts: &hosts,
            timeout: Duration::from_secs(5),
            custom_root: None,
            secret: Some(b"sk-test".to_vec()),
        };
        let payload = OpenAiCostsStrategy { base_url: None }
            .fetch(&ctx)
            .await
            .unwrap();
        assert_eq!(
            *http.calls.lock().unwrap(),
            usage_host::policy::MAX_API_PAGES
        );
        assert_eq!(payload.coverage, Coverage::Partial);
        assert!(payload
            .warnings
            .contains(&"pagination_bounded_partial".to_string()));
    }
}
