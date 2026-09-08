use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
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
use usage_host::policy;

pub const SCHEMA_FINGERPRINT: &str = "codex-jsonl-token_count-v1";

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
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenInfo {
    last_token_usage: Option<TokenUsage>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenUsage {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct RateLimits {
    limit_id: Option<String>,
    limit_name: Option<String>,
    primary: Option<RateWindow>,
    secondary: Option<RateWindow>,
    plan_type: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct RateWindow {
    used_percent: Option<Decimal>,
    window_minutes: Option<u64>,
    resets_at: Option<i64>,
}

/// One parsed `token_count` event.
///
/// Semantics policy (P0-4): `last_token_usage` names the usage of the
/// last request, so every event is stored as one request delta keyed
/// by the stable `path:offset` identity. Re-imports and repeated
/// stream lines therefore never change totals (proven by fixtures).
#[derive(Debug, Clone, PartialEq)]
pub struct TokenEvent {
    pub observed: DateTime<Utc>,
    pub input_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub model: Option<String>,
    pub rate_limits: Option<RateLimits>,
}

pub fn parse_token_line(line: &str, now: DateTime<Utc>) -> Option<TokenEvent> {
    let envelope: Envelope = serde_json::from_str(line).ok()?;
    let payload = envelope.payload?;
    if payload.kind.as_deref() != Some("token_count") {
        return None;
    }
    let observed = envelope.timestamp.unwrap_or(now);
    let tokens = payload
        .info
        .as_ref()
        .and_then(|i| i.last_token_usage.as_ref());
    let model = payload
        .model
        .clone()
        .or_else(|| payload.info.as_ref().and_then(|i| i.model.clone()));
    // Token-only lines without usage still carry quota snapshots.
    Some(TokenEvent {
        observed,
        input_tokens: tokens.and_then(|t| t.input_tokens),
        cached_tokens: tokens.and_then(|t| t.cached_input_tokens),
        output_tokens: tokens.and_then(|t| t.output_tokens),
        reasoning_tokens: tokens.and_then(|t| t.reasoning_output_tokens),
        total_tokens: tokens.and_then(|t| t.total_tokens),
        model,
        rate_limits: payload.rate_limits,
    })
}

/// Uniform root resolution shared by refresh and history import:
/// explicit custom root > `$CODEX_HOME/sessions` > `~/.codex/sessions`.
pub fn resolve_sessions_root(custom_root: Option<&Path>) -> PathBuf {
    if let Some(root) = custom_root {
        return root.to_path_buf();
    }
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".codex"))
        .join("sessions")
}

/// Scan every session file tail for the newest rate-limit snapshot.
/// Newest-file mtime alone must not hide a valid quota event living
/// in another relevant session file.
pub fn latest_rate_limit_in_dir(root: &Path) -> Option<(DateTime<Utc>, RateLimits)> {
    let pattern = format!("{}/**/rollout-*.jsonl", root.display());
    let mut best: Option<(DateTime<Utc>, RateLimits)> = None;
    let Ok(paths) = glob::glob(&pattern) else {
        return None;
    };
    for path in paths.filter_map(Result::ok) {
        if let Some(candidate) = latest_rate_limit_in_file(&path) {
            let is_newer = best.as_ref().is_none_or(|(at, _)| candidate.0 > *at);
            if is_newer {
                best = Some(candidate);
            }
        }
    }
    best
}

fn latest_rate_limit_in_file(path: &Path) -> Option<(DateTime<Utc>, RateLimits)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let start = size.saturating_sub(policy::QUOTA_TAIL_BYTES as u64);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((size - start) as usize);
    file.read_to_end(&mut buf).ok()?;
    let tail = std::str::from_utf8(&buf).ok()?;
    let mut lines: Vec<&str> = tail.lines().collect();
    if start > 0 {
        // First line may be a chunk-boundary fragment; the tail scan
        // reads newest-first so dropping it is safe.
        lines.remove(0);
    }
    let now = Utc::now();
    lines
        .iter()
        .rev()
        .filter_map(|line| parse_token_line(line, now))
        .find_map(|event| event.rate_limits.map(|rate| (event.observed, rate)))
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
        resolve_sessions_root(self.sessions_root.as_deref())
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
            let Some(event) = parse_token_line(&line, Utc::now()) else {
                if looks_like_json(&line) {
                    parsed
                        .warnings
                        .push(format!("skipped_non_token_line_{line_no}"));
                }
                continue;
            };
            if let Some(rate) = event.rate_limits.clone() {
                parsed.latest_rate_limits = Some((event.observed, rate));
            }
            if event.total_tokens.is_some()
                || event.input_tokens.is_some()
                || event.output_tokens.is_some()
            {
                parsed.records.push(UsageRecord {
                    account_id: account.id,
                    product_id: "codex".into(),
                    billing_scope_id: None,
                    source_record_id: format!(
                        "{}:{record_offset}",
                        crate::anonymous_path_identity(path)
                    ),
                    period_start: event.observed,
                    period_end: event.observed,
                    model: event.model,
                    input_tokens: event.input_tokens,
                    output_tokens: event.output_tokens,
                    cached_tokens: event.cached_tokens,
                    reasoning_tokens: event.reasoning_tokens,
                    total_tokens: event.total_tokens,
                    requests: Some(1),
                    source: "codex-local-session-v1".into(),
                    coverage: Coverage::LocalClientOnly,
                });
            }
        }
        Ok(parsed)
    }
}

fn looks_like_json(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('{') && serde_json::from_str::<serde_json::Value>(line).is_err()
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
        crate::descriptor::CODEX_CAPS.iter().copied().collect()
    }
    fn allowed_hosts(&self) -> &'static [&'static str] {
        &[]
    }
    async fn refresh(&self, account: &Account) -> Result<Snapshot, ProviderError> {
        let root = self.root();
        let (observed_at, rate) = latest_rate_limit_in_dir(&root).ok_or_else(|| {
            ProviderError::Unavailable("codex_rate_limit_snapshot_not_found".into())
        })?;
        let mut quotas = Vec::new();
        if let Some(window) = rate.primary.clone() {
            quotas.push(to_quota(
                "primary",
                rate.limit_name.as_deref().unwrap_or("Основной лимит"),
                window,
                &rate.limit_id,
                observed_at,
            )?);
        }
        if let Some(window) = rate.secondary.clone() {
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
        // Window semantics are unproven for this source: a derived
        // start/end pair alone does not make it a fixed provider window.
        window_kind: WindowKind::Unknown,
        source: format!("codex-local-session@{}", observed.timestamp()),
    })
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Strategy wrapper: Codex local JSONL over scoped HostServices.
pub struct CodexJsonlStrategy {
    pub adapter: CodexLocalAdapter,
}

impl CodexJsonlStrategy {
    pub fn new(custom_root: Option<PathBuf>) -> Self {
        Self {
            adapter: CodexLocalAdapter {
                sessions_root: custom_root,
            },
        }
    }
}

#[async_trait]
impl crate::strategy::FetchStrategy for CodexJsonlStrategy {
    fn source_id(&self) -> crate::strategy::SourceId {
        crate::strategy::SourceId("codex-local-jsonl")
    }
    fn classification(&self) -> crate::strategy::SourceClassification {
        crate::strategy::SourceClassification::LocalStructuredData
    }
    async fn availability(
        &self,
        _ctx: &crate::strategy::FetchContext<'_>,
    ) -> crate::strategy::Availability {
        let root = self.adapter.root();
        if root.is_dir() {
            crate::strategy::Availability::Ready
        } else {
            crate::strategy::Availability::NotConfigured
        }
    }
    async fn fetch(
        &self,
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> Result<crate::strategy::FetchPayload, crate::strategy::SourceError> {
        let snapshot = self
            .adapter
            .refresh(ctx.account)
            .await
            .map_err(|e| match e {
                ProviderError::AuthenticationRequired => {
                    crate::strategy::SourceError::AuthenticationRequired
                }
                ProviderError::PermissionDenied => crate::strategy::SourceError::PermissionDenied,
                ProviderError::InsufficientScope => crate::strategy::SourceError::InsufficientScope,
                ProviderError::RateLimited(_) => crate::strategy::SourceError::RateLimited {
                    retry_after_secs: None,
                },
                ProviderError::Network(_) => crate::strategy::SourceError::Network,
                ProviderError::Parse(_) => crate::strategy::SourceError::Parse {
                    schema: SCHEMA_FINGERPRINT,
                },
                ProviderError::Unavailable(_) => crate::strategy::SourceError::Unavailable,
            })?;
        let observed_at = snapshot.observed_at;
        let coverage = snapshot.coverage;
        Ok(crate::strategy::FetchPayload {
            snapshot: Some(snapshot),
            usage: vec![],
            costs: vec![],
            balances: vec![],
            warnings: vec![],
            schema_fingerprint: SCHEMA_FINGERPRINT,
            coverage,
            observed_at,
        })
    }
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
    #[test]
    fn repeated_import_of_same_events_is_stable() {
        // Cumulative re-delivery of the same lines: stable identities,
        // so storage upserts keep totals unchanged.
        let line = include_str!("../fixtures/codex-token-count.jsonl").trim();
        let now = Utc::now();
        let first = parse_token_line(line, now).unwrap();
        let second = parse_token_line(line, now).unwrap();
        assert_eq!(first, second);
    }
    #[test]
    fn mixed_schema_lines_are_tolerated() {
        let now = Utc::now();
        assert!(
            parse_token_line(r#"{"type":"event_msg","payload":{"type":"other"}}"#, now).is_none()
        );
        assert!(parse_token_line("not json at all", now).is_none());
        let with_new_fields = r#"{"timestamp":"2026-09-08T12:00:00Z","type":"event_msg","future_field":{"nested":true},"payload":{"type":"token_count","unknown_kind":"x","info":{"last_token_usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12,"brand_new_metric":99}}}}"#;
        let event = parse_token_line(with_new_fields, now).unwrap();
        assert_eq!(event.total_tokens, Some(12));
    }
    #[test]
    fn quota_scan_finds_event_beyond_newest_file() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let sub_new = sessions.join("new");
        let sub_old = sessions.join("old");
        std::fs::create_dir_all(&sub_new).unwrap();
        std::fs::create_dir_all(&sub_old).unwrap();
        // Newest file carries no quota snapshot...
        std::fs::write(
            sub_new.join("rollout-new.jsonl"),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"other\"}}\n",
        )
        .unwrap();
        // ...while an older relevant session does.
        let quota_line = include_str!("../fixtures/codex-token-count.jsonl").trim();
        std::fs::write(sub_old.join("rollout-old.jsonl"), format!("{quota_line}\n")).unwrap();
        // Force mtime order: touch the new file last.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        let _ = filetime_set(sub_new.join("rollout-new.jsonl"), later);
        let found = latest_rate_limit_in_dir(&sessions);
        assert!(found.is_some());
        assert_eq!(found.unwrap().1.limit_id.as_deref(), Some("codex-fixture"));
    }
    #[test]
    fn window_kind_stays_unknown_without_proven_semantics() {
        let quota = to_quota(
            "primary",
            "x",
            RateWindow {
                used_percent: Some(Decimal::new(325, 1)),
                window_minutes: Some(300),
                resets_at: Some(1788872400),
            },
            &Some("id".into()),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(quota.window_kind, WindowKind::Unknown);
    }
    #[test]
    fn codex_home_is_shared_by_refresh_and_import() {
        std::env::set_var("CODEX_HOME", "/tmp/usage-ai-test-codex-home");
        let root = resolve_sessions_root(None);
        assert_eq!(
            root,
            PathBuf::from("/tmp/usage-ai-test-codex-home/sessions")
        );
        std::env::remove_var("CODEX_HOME");
        let custom = PathBuf::from("/custom/sessions");
        assert_eq!(resolve_sessions_root(Some(&custom)), custom);
    }

    fn filetime_set(path: PathBuf, time: std::time::SystemTime) -> std::io::Result<()> {
        let file = std::fs::OpenOptions::new().write(true).open(path)?;
        file.set_modified(time)
    }
}
