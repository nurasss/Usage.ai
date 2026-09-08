use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use std::{collections::BTreeSet, str::FromStr, time::Duration};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, MoneyBalance, ProviderAdapter,
    ProviderError, Snapshot,
};
use uuid::Uuid;

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
    /// Official pagination cursor: response `next_page`, requested back
    /// as query `page`. `None` with `truncated == false` means complete.
    pub next_page: Option<String>,
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
    pub api_key_id: Option<String>,
}

/// Decimal money from a JSON number. Providers emit decimal currency
/// values (e.g. `19.99`); `f64`'s shortest round-trip representation
/// reproduces those decimals exactly, so `Decimal` stays exact for all
/// realistic billing amounts. (A workspace-wide `arbitrary_precision`
/// switch was evaluated and rejected: it breaks `rust_decimal`
/// number deserialization used by quota parsing.)
fn decimal_from_number(value: &serde_json::Value) -> Option<Decimal> {
    let number = value.as_number()?;
    Decimal::from_str(&number.to_string()).ok()
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
    // Official contract only: response `next_page` ↔ query `page`
    // (docs + openai-python + openai-node). No other field is ever
    // guessed as a pagination cursor: a wrong cursor would silently
    // skip or duplicate billing rows.
    let mut out = ParsedCosts {
        records: vec![],
        truncated: value
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        next_page: value
            .get("next_page")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
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
            let amount = row
                .get("amount")
                .and_then(|a| a.get("value"))
                .and_then(decimal_from_number);
            let currency = row
                .get("amount")
                .and_then(|a| a.get("currency"))
                .and_then(|v| v.as_str())
                .map(|c| c.to_ascii_uppercase());
            let (Some(amount), Some(currency)) = (amount, currency) else {
                out.warnings
                    .push(format!("bucket_{bucket_idx}_row_{row_idx}_incomplete"));
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
            let api_key_id = row
                .get("api_key_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            out.records.push(CostDraft {
                // Identity includes currency/scope/key: two rows for the
                // same bucket in different currencies or scopes must not
                // overwrite each other. Amount is excluded on purpose:
                // a corrected re-report must update, not duplicate.
                source_record_id: format!(
                    "openai-costs:{start}:{end}:{currency}:{}:{}:{}",
                    project_id.as_deref().unwrap_or("org"),
                    line_item.as_deref().unwrap_or("all"),
                    api_key_id.as_deref().unwrap_or("nokey")
                ),
                period_start: start,
                period_end: end,
                amount,
                currency,
                project_id,
                line_item,
                api_key_id,
            });
        }
    }
    let _ = source;
    Ok(out)
}

impl OpenAiApiAdapter {
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
    /// Official pagination: opaque `next_page` cursor from the response
    /// is sent back as query `page` (verified against the official Costs
    /// reference + openai-python/openai-node SDK contracts).
    fn page_url(&self, start: DateTime<Utc>, end: DateTime<Utc>, page: Option<&str>) -> String {
        let base = self
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com".into());
        let mut url = format!(
            "{base}{COSTS_PATH}?start_time={}&end_time={}&bucket_width=1d&limit=180",
            start.timestamp(),
            end.timestamp()
        );
        if let Some(page) = page {
            use std::fmt::Write;
            let _ = write!(url, "&page={}", percent_encode(page));
        }
        url
    }
}

/// Minimal percent-encoding for opaque cursor tokens (unreserved set
/// passes through untouched).
fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
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
        let mut page: Option<String> = None;
        let mut pages = 0usize;
        loop {
            let url = self.page_url(start, end, page.as_deref());
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
                    cancel: usage_host::CancellationToken::new(),
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
            match parsed.next_page {
                Some(next) if pages < usage_host::policy::MAX_API_PAGES => page = Some(next),
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
        pages: std::sync::Mutex<Vec<ScriptResponse>>,
        calls: std::sync::Mutex<usize>,
    }

    enum ScriptResponse {
        Page(serde_json::Value),
        NetworkError,
    }

    #[async_trait::async_trait]
    impl usage_host::HttpHost for ScriptHttp {
        async fn get_json(
            &self,
            _req: usage_host::HttpJsonRequest<'_>,
        ) -> Result<usage_host::HttpJsonResponse, usage_host::HostError> {
            *self.calls.lock().unwrap() += 1;
            match self.pages.lock().unwrap().remove(0) {
                ScriptResponse::Page(body) => Ok(usage_host::HttpJsonResponse {
                    status: 200,
                    retry_after_secs: None,
                    body: Some(body),
                }),
                ScriptResponse::NetworkError => Err(usage_host::HostError::Unavailable),
            }
        }
    }

    fn test_hosts(pages: Vec<ScriptResponse>) -> (usage_host::Hosts, std::sync::Arc<ScriptHttp>) {
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

    fn page(pages: Vec<serde_json::Value>) -> Vec<ScriptResponse> {
        pages.into_iter().map(ScriptResponse::Page).collect()
    }

    /// Official-shape cost page: `object/data/has_more/next_page`,
    /// bucket `object/start_time/end_time/results`, row
    /// `object/amount{value,currency}/line_item/project_id/api_key_id`.
    /// Amounts are JSON number literals parsed from decimal text.
    fn cost_page(
        bucket_start: i64,
        rows: &[(&str, &str, Option<&str>)],
        has_more: bool,
        next_page: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "object": "page",
            "has_more": has_more,
            "next_page": next_page,
            "data": [{
                "object": "bucket",
                "start_time": bucket_start,
                "end_time": bucket_start + 86_400,
                "results": rows.iter().map(|(item, amount, project)| {
                    serde_json::json!({
                        "object": "organization.costs.result",
                        "amount": {"value": serde_json::from_str::<serde_json::Value>(amount).unwrap(), "currency": "usd"},
                        "line_item": item,
                        "project_id": project,
                        "api_key_id": null
                    })
                }).collect::<Vec<_>>()
            }]
        })
    }

    fn test_account() -> Account {
        Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "API".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        }
    }

    fn test_ctx<'a>(
        account: &'a Account,
        hosts: &'a usage_host::Hosts,
    ) -> crate::strategy::FetchContext<'a> {
        crate::strategy::FetchContext {
            account,
            hosts,
            timeout: Duration::from_secs(5),
            custom_root: None,
            secret: Some(b"sk-test".to_vec()),
            cancel: usage_host::CancellationToken::new(),
        }
    }

    #[test]
    fn decimal_amounts_stay_exact_without_float_roundtrip() {
        let value = cost_page(1_725_000_000, &[("x", "19.99", None)], false, None);
        assert_eq!(
            parse_costs(&value, "test").unwrap().records[0].amount,
            rust_decimal::Decimal::from_str("19.99").unwrap()
        );
        let tiny = cost_page(1_725_000_000, &[("x", "0.06", None)], false, None);
        assert_eq!(
            parse_costs(&tiny, "test").unwrap().records[0].amount,
            rust_decimal::Decimal::from_str("0.06").unwrap()
        );
    }

    #[test]
    fn currencies_scopes_and_keys_get_distinct_identities() {
        let value = serde_json::json!({
            "object": "page", "has_more": false, "next_page": null,
            "data": [{
                "object": "bucket", "start_time": 1_725_000_000, "end_time": 1_725_086_400,
                "results": [
                    {"object": "organization.costs.result",
                     "amount": {"value": 1.0, "currency": "usd"},
                     "line_item": "gpt", "project_id": "p1", "api_key_id": null},
                    {"object": "organization.costs.result",
                     "amount": {"value": 1.0, "currency": "eur"},
                     "line_item": "gpt", "project_id": "p1", "api_key_id": null},
                    {"object": "organization.costs.result",
                     "amount": {"value": 1.0, "currency": "usd"},
                     "line_item": "gpt", "project_id": "p2", "api_key_id": "k1"}
                ]
            }]
        });
        let parsed = parse_costs(&value, "test").unwrap();
        let ids: std::collections::HashSet<_> = parsed
            .records
            .iter()
            .map(|r| r.source_record_id.clone())
            .collect();
        assert_eq!(ids.len(), 3);
    }

    #[tokio::test]
    async fn one_refresh_fetches_each_page_exactly_once() {
        use crate::strategy::FetchStrategy;
        let (hosts, http) = test_hosts(page(vec![
            cost_page(1_725_000_000, &[("a", "1.0", None)], true, Some("p2")),
            cost_page(1_725_086_400, &[("b", "2.0", None)], true, Some("p3")),
            cost_page(1_725_172_800, &[("c", "3.0", None)], false, None),
        ]));
        let account = test_account();
        let ctx = test_ctx(&account, &hosts);
        let payload = OpenAiCostsStrategy { base_url: None }
            .fetch(&ctx)
            .await
            .unwrap();
        // One logical refresh, N page requests, one normalized result.
        assert!(payload.snapshot.is_some());
        assert_eq!(payload.costs.len(), 3);
        assert_eq!(*http.calls.lock().unwrap(), 3);
        assert_eq!(payload.coverage, Coverage::ProviderDelayed);
    }

    #[tokio::test]
    async fn duplicate_pages_collapse_onto_one_storage_key() {
        let same = cost_page(1_725_000_000, &[("a", "1.0", None)], false, None);
        let first = parse_costs(&same, "test").unwrap();
        let second = parse_costs(&same, "test").unwrap();
        assert_eq!(first.records[0], second.records[0]);
        let unique: std::collections::HashSet<_> = first
            .records
            .iter()
            .chain(second.records.iter())
            .map(|r| r.source_record_id.clone())
            .collect();
        assert_eq!(unique.len(), 1);
    }

    #[tokio::test]
    async fn page_two_network_error_fails_without_partial_result() {
        use crate::strategy::FetchStrategy;
        let (hosts, http) = test_hosts(vec![
            ScriptResponse::Page(cost_page(
                1_725_000_000,
                &[("a", "1.0", None)],
                true,
                Some("p2"),
            )),
            ScriptResponse::NetworkError,
        ]);
        let account = test_account();
        let ctx = test_ctx(&account, &hosts);
        let result = OpenAiCostsStrategy { base_url: None }.fetch(&ctx).await;
        assert!(matches!(result, Err(crate::strategy::SourceError::Network)));
        assert_eq!(*http.calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn malformed_page_is_schema_error_not_empty_bill() {
        use crate::strategy::FetchStrategy;
        let (hosts, _) = test_hosts(page(vec![serde_json::json!({
            "object": "page", "has_more": false, "data": "not-an-array"
        })]));
        let account = test_account();
        let ctx = test_ctx(&account, &hosts);
        let result = OpenAiCostsStrategy { base_url: None }.fetch(&ctx).await;
        assert!(matches!(
            result,
            Err(crate::strategy::SourceError::Parse { .. })
        ));
    }

    #[tokio::test]
    async fn legacy_cursor_fields_are_never_followed() {
        // A response advertising only the old guessed cursor shape
        // ends pagination as bounded-partial instead of looping.
        use crate::strategy::FetchStrategy;
        let (hosts, http) = test_hosts(page(vec![serde_json::json!({
            "object": "page", "has_more": true, "starting_after": "xyz",
            "data": [{
                "object": "bucket", "start_time": 1_725_000_000, "end_time": 1_725_086_400,
                "results": [{"object": "organization.costs.result",
                             "amount": {"value": 1.0, "currency": "usd"},
                             "line_item": "a", "project_id": null, "api_key_id": null}]
            }]
        })]));
        let account = test_account();
        let ctx = test_ctx(&account, &hosts);
        let payload = OpenAiCostsStrategy { base_url: None }
            .fetch(&ctx)
            .await
            .unwrap();
        assert_eq!(*http.calls.lock().unwrap(), 1);
        assert_eq!(payload.coverage, Coverage::Partial);
    }

    #[tokio::test]
    async fn strategy_fetches_all_pages_once_per_refresh() {
        use crate::strategy::FetchStrategy;
        let (hosts, http) = test_hosts(page(vec![
            cost_page(1_725_000_000, &[("a", "1.0", None)], true, Some("c1")),
            cost_page(1_725_086_400, &[("b", "2.0", None)], false, None),
        ]));
        let account = test_account();
        let ctx = test_ctx(&account, &hosts);
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
        use crate::strategy::FetchStrategy;
        let endless: Vec<ScriptResponse> = (0..20)
            .map(|_| {
                ScriptResponse::Page(cost_page(
                    1_725_000_000,
                    &[("a", "1.0", None)],
                    true,
                    Some("again"),
                ))
            })
            .collect();
        let (hosts, http) = test_hosts(endless);
        let account = test_account();
        let ctx = test_ctx(&account, &hosts);
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
