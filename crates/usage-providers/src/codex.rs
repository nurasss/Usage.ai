use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use glob::glob;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, ImportBatch, MetricUnit,
    ProviderAdapter, ProviderError, Quota, Snapshot, UsageRecord, WindowKind,
};

#[derive(Default)]
pub struct CodexLocalAdapter {
    pub sessions_root: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    timestamp: Option<DateTime<Utc>>,
    payload: Option<Payload>,
}
#[derive(Debug, Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    info: Option<TokenInfo>,
    rate_limits: Option<RateLimits>,
}
#[derive(Debug, Deserialize)]
struct TokenInfo {
    last_token_usage: Option<TokenUsage>,
}
#[derive(Debug, Deserialize)]
struct TokenUsage {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}
#[derive(Debug, Deserialize)]
struct RateLimits {
    limit_id: Option<String>,
    limit_name: Option<String>,
    primary: Option<RateWindow>,
    secondary: Option<RateWindow>,
    plan_type: Option<String>,
}
#[derive(Debug, Deserialize)]
struct RateWindow {
    used_percent: Option<Decimal>,
    window_minutes: Option<u64>,
    resets_at: Option<i64>,
}

#[derive(Debug, Default)]
struct Parsed {
    records: Vec<UsageRecord>,
    latest_rate_limits: Option<(DateTime<Utc>, RateLimits)>,
    next_offset: u64,
    partial_line: bool,
    warnings: Vec<String>,
}

impl CodexLocalAdapter {
    fn root(&self) -> PathBuf {
        self.sessions_root.clone().unwrap_or_else(|| {
            std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| dirs_home().join(".codex"))
                .join("sessions")
        })
    }
    fn latest_file(&self) -> Option<PathBuf> {
        glob(&format!("{}/**/rollout-*.jsonl", self.root().display()))
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|p| std::fs::metadata(&p).ok().map(|m| (p, m.modified().ok())))
            .max_by_key(|(_, m)| *m)
            .map(|(p, _)| p)
    }

    fn parse_file(
        &self,
        account: &Account,
        path: &Path,
        offset: u64,
    ) -> Result<Parsed, ProviderError> {
        let mut file = File::open(path).map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        let size = file
            .metadata()
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?
            .len();
        let start = offset.min(size);
        file.seek(SeekFrom::Start(start))
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        let mut reader = BufReader::new(file);
        let mut cursor = start;
        let mut parsed = Parsed {
            next_offset: start,
            ..Default::default()
        };
        let mut line = String::new();
        let mut line_no = 0u64;
        loop {
            line.clear();
            let bytes = reader
                .read_line(&mut line)
                .map_err(|e| ProviderError::Parse(e.to_string()))?;
            if bytes == 0 {
                break;
            }
            line_no += 1;
            if !line.ends_with('\n') {
                parsed.partial_line = true;
                break;
            }
            let record_offset = cursor;
            cursor += bytes as u64;
            parsed.next_offset = cursor;
            let envelope: Envelope = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    parsed
                        .warnings
                        .push(format!("invalid_json_at_relative_line_{line_no}"));
                    continue;
                }
            };
            let Some(payload) = envelope.payload else {
                continue;
            };
            if payload.kind.as_deref() != Some("token_count") {
                continue;
            }
            let observed = envelope.timestamp.unwrap_or_else(Utc::now);
            if let Some(rate) = payload.rate_limits {
                parsed.latest_rate_limits = Some((observed, rate));
            }
            if let Some(tokens) = payload.info.and_then(|i| i.last_token_usage) {
                parsed.records.push(UsageRecord {
                    account_id: account.id,
                    product_id: "codex".into(),
                    billing_scope_id: None,
                    source_record_id: format!(
                        "{}:{record_offset}",
                        crate::anonymous_path_identity(path)
                    ),
                    period_start: observed,
                    period_end: observed,
                    model: None,
                    input_tokens: tokens.input_tokens,
                    output_tokens: tokens.output_tokens,
                    cached_tokens: tokens.cached_input_tokens,
                    reasoning_tokens: tokens.reasoning_output_tokens,
                    total_tokens: tokens.total_tokens,
                    requests: Some(1),
                    source: "codex-local-session-v1".into(),
                    coverage: Coverage::LocalClientOnly,
                });
            }
        }
        Ok(parsed)
    }
}

#[async_trait]
impl ProviderAdapter for CodexLocalAdapter {
    fn provider_id(&self) -> &'static str {
        "openai"
    }
    fn product_id(&self) -> &'static str {
        "codex"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        [
            Capability::SubscriptionQuota,
            Capability::QuotaResetTime,
            Capability::ApiTokens,
            Capability::HistoryLocal,
            Capability::LocalSessions,
        ]
        .into_iter()
        .collect()
    }
    fn allowed_hosts(&self) -> &'static [&'static str] {
        &[]
    }
    async fn refresh(&self, account: &Account) -> Result<Snapshot, ProviderError> {
        let path = self
            .latest_file()
            .ok_or_else(|| ProviderError::Unavailable("codex_sessions_not_found".into()))?;
        let parsed = self.parse_file(account, &path, 0)?;
        let (observed_at, rate) = parsed.latest_rate_limits.ok_or_else(|| {
            ProviderError::Unavailable("codex_rate_limit_snapshot_not_found".into())
        })?;
        let mut quotas = Vec::new();
        if let Some(window) = rate.primary {
            quotas.push(to_quota(
                "primary",
                rate.limit_name.as_deref().unwrap_or("Основной лимит"),
                window,
                &rate.limit_id,
                observed_at,
            )?);
        }
        if let Some(window) = rate.secondary {
            quotas.push(to_quota(
                "secondary",
                "Дополнительный лимит",
                window,
                &rate.limit_id,
                observed_at,
            )?);
        }
        let fetched = Utc::now();
        let age = (fetched - observed_at).num_seconds().max(0) as u64;
        Ok(Snapshot {
            account_id: account.id,
            provider_id: "openai".into(),
            product_id: "codex".into(),
            plan_label: rate.plan_type,
            capabilities: self.capabilities(),
            quotas,
            balances: vec![],
            observed_at: Some(observed_at),
            fetched_at: fetched,
            connection_state: ConnectionState::Connected,
            freshness: if age < 600 {
                Freshness::Fresh
            } else {
                Freshness::Stale(age)
            },
            coverage: Coverage::LocalClientOnly,
        })
    }
    async fn import_local(
        &self,
        account: &Account,
        path: &Path,
        offset: u64,
    ) -> Result<ImportBatch, ProviderError> {
        let p = self.parse_file(account, path, offset)?;
        Ok(ImportBatch {
            records: p.records,
            next_offset: p.next_offset,
            partial_line: p.partial_line,
            warnings: p.warnings,
        })
    }
}

fn to_quota(
    pool: &str,
    name: &str,
    w: RateWindow,
    limit_id: &Option<String>,
    observed: DateTime<Utc>,
) -> Result<Quota, ProviderError> {
    if w.used_percent
        .is_some_and(|v| v < Decimal::ZERO || v > Decimal::ONE_HUNDRED)
    {
        return Err(ProviderError::Parse("invalid_percentage".into()));
    }
    let reset = w.resets_at.and_then(|v| Utc.timestamp_opt(v, 0).single());
    let window_start = reset
        .zip(w.window_minutes)
        .map(|(r, m)| r - chrono::Duration::minutes(m as i64));
    Ok(Quota {
        pool_id: format!("{}-{pool}", limit_id.as_deref().unwrap_or("codex")),
        window_id: reset.map(|r| r.timestamp().to_string()),
        name: name.into(),
        used: None,
        limit: None,
        used_percent: w.used_percent,
        remaining_percent: w.used_percent.map(|v| Decimal::ONE_HUNDRED - v),
        unit: MetricUnit::Percent,
        window_start,
        resets_at: reset,
        window_kind: if window_start.is_some() {
            WindowKind::Fixed
        } else {
            WindowKind::Unknown
        },
        source: format!("codex-local-session@{}", observed.timestamp()),
    })
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use uuid::Uuid;
    fn account() -> Account {
        Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "Аккаунт 1".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        }
    }
    #[test]
    fn parses_dynamic_rate_pool_and_tokens() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            f,
            "{}",
            include_str!("../fixtures/codex-token-count.jsonl").trim()
        )
        .unwrap();
        let a = CodexLocalAdapter::default();
        let p = a.parse_file(&account(), f.path(), 0).unwrap();
        assert_eq!(p.records.len(), 1);
        assert!(p.latest_rate_limits.is_some());
    }
    #[test]
    fn preserves_partial_line_for_next_import() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{{\"type\":\"event_msg\"").unwrap();
        let p = CodexLocalAdapter::default()
            .parse_file(&account(), f.path(), 0)
            .unwrap();
        assert!(p.partial_line);
        assert_eq!(p.next_offset, 0);
    }
}
