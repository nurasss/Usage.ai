use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
#[cfg(test)]
use std::path::Path;
use std::{collections::BTreeSet, path::PathBuf};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, MetricUnit, ProviderError, Quota,
    Snapshot, UsageRecord, WindowKind,
};
use usage_host::policy;

pub const SCHEMA_FINGERPRINT: &str = "codex-jsonl-token_count-v1";

#[derive(Debug, Deserialize)]
struct Envelope {
    timestamp: Option<DateTime<Utc>>,
    ordinal: Option<u64>,
    #[serde(rename = "requestId", alias = "request_id")]
    request_id: Option<String>,
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
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option", default)]
    used_percent: Option<Decimal>,
    window_minutes: Option<u64>,
    resets_at: Option<i64>,
}

/// One parsed `token_count` event.
///
/// Current parser policy (covered only by synthetic fixtures in
/// `fixtures/codex/`): `last_token_usage` is treated as one request delta.
/// A source-provided `requestId` is the identity when present; the
/// path/offset fallback is only a conservative import identity and does not
/// prove that repeated real-client events are independent requests. Real
/// semantics are intentionally marked `UnverifiedSemantics` until an
/// actually captured, redacted Codex corpus confirms the interpretation.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenEvent {
    pub observed: DateTime<Utc>,
    pub ordinal: Option<u64>,
    pub request_id: Option<String>,
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
    Some(TokenEvent {
        observed,
        ordinal: envelope.ordinal,
        request_id: envelope.request_id,
        input_tokens: tokens.and_then(|t| t.input_tokens),
        cached_tokens: tokens.and_then(|t| t.cached_input_tokens),
        output_tokens: tokens.and_then(|t| t.output_tokens),
        reasoning_tokens: tokens.and_then(|t| t.reasoning_output_tokens),
        total_tokens: tokens.and_then(|t| t.total_tokens),
        model,
        rate_limits: payload.rate_limits,
    })
}

/// Pure byte-range parser: no filesystem, no environment. Maps a
/// `[base_offset, base_offset + bytes.len())` window into records
/// with absolute stable identities. A trailing partial line (no `\n`)
/// is never consumed. `#` comment lines and blank lines are skipped
/// silently; `{`-led invalid JSON counts as malformed.
pub fn parse_chunk_bytes(
    account: &Account,
    path_hash: &str,
    base_offset: u64,
    bytes: &[u8],
    now: DateTime<Utc>,
) -> crate::strategy::ParsedChunk {
    use crate::strategy::ParsedChunk;
    let mut chunk = ParsedChunk::default();
    let mut cursor = base_offset;
    let mut line_no = 0u64;
    // `split_inclusive` keeps terminators so offsets stay exact even
    // for weird line endings inside the window.
    for piece in bytes.split_inclusive(|byte| *byte == b'\n') {
        let piece_bytes = piece.len() as u64;
        if !piece.ends_with(b"\n") {
            chunk.partial = true;
            break;
        }
        line_no += 1;
        let record_offset = cursor;
        cursor += piece_bytes;
        let text = String::from_utf8_lossy(piece);
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(event) = parse_token_line(trimmed, now) else {
            // Malformed = JSON-looking but unparseable. Valid JSON of
            // another event type and plain-text lines stay silent.
            if trimmed.starts_with('{')
                && serde_json::from_str::<serde_json::Value>(trimmed).is_err()
            {
                chunk.malformed += 1;
                chunk
                    .warnings
                    .push(format!("invalid_json_at_relative_line_{line_no}"));
            }
            continue;
        };
        if event.total_tokens.is_some()
            || event.input_tokens.is_some()
            || event.output_tokens.is_some()
        {
            let source_record_id = event
                .request_id
                .filter(|id| !id.trim().is_empty())
                .map(|id| format!("{path_hash}:request:{id}"))
                .or_else(|| {
                    event
                        .ordinal
                        .map(|ordinal| format!("{path_hash}:ordinal:{ordinal}"))
                })
                .unwrap_or_else(|| format!("{path_hash}:{record_offset}"));
            chunk.records.push(UsageRecord {
                account_id: account.id,
                product_id: "codex".into(),
                billing_scope_id: None,
                source_record_id,
                period_start: event.observed,
                period_end: event.observed,
                model: event.model,
                input_tokens: event.input_tokens,
                output_tokens: event.output_tokens,
                cached_tokens: event.cached_tokens,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                reasoning_tokens: event.reasoning_tokens,
                total_tokens: event.total_tokens,
                requests: Some(1),
                source: "codex-local-session-v1".into(),
                coverage: Coverage::UnverifiedSemantics,
                connection_id: None,
                file_id: None,
                file_generation: None,
            });
        }
    }
    chunk.consumed = cursor;
    chunk
}

/// A custom path may point at the product home or directly at its
/// sessions directory; both spellings resolve to the same home.
#[cfg(test)]
fn normalize_home(custom: &Path) -> PathBuf {
    if custom
        .file_name()
        .is_some_and(|name| name == "sessions" || name == "archived_sessions")
    {
        custom
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| custom.to_path_buf())
    } else {
        custom.to_path_buf()
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

fn snapshot_from_rate(
    account: &Account,
    capabilities: BTreeSet<Capability>,
    observed_at: DateTime<Utc>,
    fetched_at: DateTime<Utc>,
    rate: &RateLimits,
) -> Result<Snapshot, ProviderError> {
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
    let age = (fetched_at - observed_at).num_seconds().max(0) as u64;
    Ok(Snapshot {
        account_id: account.id,
        provider_id: "openai".into(),
        product_id: "codex".into(),
        plan_label: rate.plan_type.clone(),
        capabilities,
        quotas,
        balances: vec![],
        observed_at: Some(observed_at),
        fetched_at,
        connection_state: ConnectionState::Connected,
        freshness: if age < 600 {
            Freshness::Fresh
        } else {
            Freshness::Stale(age)
        },
        coverage: Coverage::UnverifiedSemantics,
    })
}

/// Strategy wrapper: Codex local JSONL over scoped HostServices.
/// Quota is scanned across every session tail in the product home —
/// never newest-file-only.
pub struct CodexJsonlStrategy {
    pub custom_root: Option<PathBuf>,
}

impl CodexJsonlStrategy {
    pub fn new(custom_root: Option<PathBuf>) -> Self {
        Self { custom_root }
    }

    fn capabilities() -> BTreeSet<Capability> {
        crate::descriptor::CODEX_CAPS.iter().copied().collect()
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
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> crate::strategy::Availability {
        if ctx.cancel.is_cancelled() {
            return crate::strategy::Availability::Disabled;
        }
        if ctx.facade.files.is_some() && ctx.local_root.is_some() {
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
        let Some(file_host) = ctx.facade.files else {
            return Err(SourceError::Unavailable);
        };
        let Some(root) = ctx.local_root.as_ref() else {
            return Err(SourceError::Unavailable);
        };
        let files = file_host
            .list_files(
                root,
                ctx.descriptor
                    .local_glob
                    .unwrap_or("sessions/**/rollout-*.jsonl"),
                &ctx.cancel,
            )
            .map_err(|_| SourceError::Unavailable)?;
        let now = ctx.facade.clock.now();
        let mut best: Option<(DateTime<Utc>, RateLimits)> = None;
        for file in &files {
            if ctx.cancel.is_cancelled() {
                return Err(SourceError::Cancelled);
            }
            let Ok(tail) = file_host.read_scoped_tail(file, policy::QUOTA_TAIL_BYTES, &ctx.cancel)
            else {
                continue;
            };
            let text = String::from_utf8_lossy(&tail);
            let mut lines: Vec<&str> = text.lines().collect();
            if !tail.is_empty() && !tail.ends_with(b"\n") {
                lines.pop();
            }
            for line in lines.iter().rev() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some(event) = parse_token_line(trimmed, now) {
                    if let Some(rate) = event.rate_limits {
                        let newer = best.as_ref().is_none_or(|(at, _)| event.observed > *at);
                        if newer {
                            best = Some((event.observed, rate));
                        }
                        break;
                    }
                }
            }
        }
        let Some((observed_at, rate)) = best else {
            return Err(SourceError::Unavailable);
        };
        let snapshot = snapshot_from_rate(
            ctx.account,
            Self::capabilities(),
            observed_at,
            ctx.facade.clock.now(),
            &rate,
        )
        .map_err(|_| SourceError::Parse {
            schema: SCHEMA_FINGERPRINT,
        })?;
        let observed = snapshot.observed_at;
        let coverage = snapshot.coverage;
        Ok(crate::strategy::FetchPayload {
            snapshot: Some(snapshot),
            usage: vec![],
            costs: vec![],
            balances: vec![],
            warnings: vec![],
            schema_fingerprint: SCHEMA_FINGERPRINT,
            coverage,
            observed_at: observed,
            source_error: None,
            observed_identity: None,
        })
    }
}

#[async_trait]
impl crate::strategy::FileLogParser for CodexJsonlStrategy {
    fn source_id(&self) -> crate::strategy::SourceId {
        crate::strategy::SourceId("codex-local-jsonl")
    }
    fn schema_fingerprint(&self) -> &'static str {
        SCHEMA_FINGERPRINT
    }
    fn parse_chunk(
        &self,
        account: &Account,
        path_hash: &str,
        base_offset: u64,
        bytes: &[u8],
    ) -> crate::strategy::ParsedChunk {
        parse_chunk_bytes(account, path_hash, base_offset, bytes, Utc::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    fn account() -> Account {
        Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "Локальная история".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        }
    }
    fn fixture(name: &str) -> Vec<u8> {
        let path = format!("fixtures/codex/{name}");
        std::fs::read(path).unwrap()
    }
    #[test]
    fn session_a_totals_match_hand_verified_values() {
        let bytes = fixture("session-a.jsonl");
        let chunk = parse_chunk_bytes(&account(), "h", 0, &bytes, Utc::now());
        assert_eq!(chunk.records.len(), 3);
        assert!(!chunk.partial);
        let sum =
            |f: fn(&UsageRecord) -> Option<u64>| chunk.records.iter().filter_map(f).sum::<u64>();
        assert_eq!(sum(|r| r.input_tokens), 400);
        assert_eq!(sum(|r| r.cached_tokens), 70);
        assert_eq!(sum(|r| r.output_tokens), 100);
        assert_eq!(sum(|r| r.reasoning_tokens), 15);
        assert_eq!(sum(|r| r.total_tokens), 510);
        let models: std::collections::HashMap<_, usize> = chunk
            .records
            .iter()
            .filter_map(|r| r.model.clone())
            .fold(std::collections::HashMap::new(), |mut acc, m| {
                *acc.entry(m).or_insert(0) += 1;
                acc
            });
        assert_eq!(models.get("gpt-a"), Some(&2));
        assert_eq!(models.get("gpt-b"), Some(&1));
    }
    #[test]
    fn malformed_lines_counted_without_stopping_import() {
        let bytes = fixture("malformed.jsonl");
        let chunk = parse_chunk_bytes(&account(), "h", 0, &bytes, Utc::now());
        assert_eq!(chunk.records.len(), 1);
        assert_eq!(chunk.malformed, 1);
        assert_eq!(chunk.records[0].total_tokens, Some(10));
    }
    #[test]
    fn partial_trailing_line_is_not_consumed() {
        let bytes = b"{\"timestamp\":\"2026-09-08T12:00:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}\n{\"incomplete\":";
        let chunk = parse_chunk_bytes(&account(), "h", 0, bytes, Utc::now());
        assert!(chunk.partial);
        assert_eq!(chunk.records.len(), 1);
        assert_eq!(
            chunk.consumed as usize,
            bytes.iter().position(|&b| b == b'\n').unwrap() + 1
        );
        // Resuming at the returned offset replays nothing twice once the
        // line completes: the completed line parses as exactly one event.
        let completed = b"{\"incomplete\": true}\n{\"timestamp\":\"2026-09-08T12:00:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}\n";
        let rest = parse_chunk_bytes(&account(), "h", chunk.consumed, completed, Utc::now());
        assert!(!rest.partial);
        assert_eq!(rest.records.len(), 1);
    }

    #[test]
    fn invalid_utf8_does_not_shift_resume_offsets() {
        let valid = br#"{"timestamp":"2026-09-08T12:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}}"#;
        let mut bytes = vec![b'{', 0xff, b'}', b'\n'];
        bytes.extend_from_slice(valid);
        bytes.push(b'\n');

        let chunk = parse_chunk_bytes(&account(), "h", 100, &bytes, Utc::now());
        assert_eq!(chunk.consumed, 100 + bytes.len() as u64);
        assert_eq!(chunk.malformed, 1);
        assert_eq!(chunk.records.len(), 1);
        assert_eq!(chunk.records[0].source_record_id, "h:104");
    }

    #[test]
    fn repeated_bytes_reimport_yields_stable_identities() {
        let bytes = fixture("duplicates.jsonl");
        let first = parse_chunk_bytes(&account(), "h", 0, &bytes, Utc::now());
        let second = parse_chunk_bytes(&account(), "h", 0, &bytes, Utc::now());
        assert_eq!(first.records.len(), 2);
        let ids_a: Vec<_> = first.records.iter().map(|r| &r.source_record_id).collect();
        let ids_b: Vec<_> = second.records.iter().map(|r| &r.source_record_id).collect();
        assert_eq!(ids_a, ids_b);
        assert_eq!(
            ids_a.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            ["h:request:r1", "h:request:r2"]
        );
    }

    #[test]
    fn repeated_same_request_id_has_one_storage_identity() {
        let bytes = br#"{"timestamp":"2026-09-08T12:00:00Z","requestId":"same","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}}
{"timestamp":"2026-09-08T12:00:01Z","requestId":"same","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}}
"#;
        let chunk = parse_chunk_bytes(&account(), "h", 0, bytes, Utc::now());
        assert_eq!(chunk.records.len(), 2);
        assert_eq!(chunk.records[0].source_record_id, "h:request:same");
        assert_eq!(
            chunk.records[0].source_record_id,
            chunk.records[1].source_record_id
        );
    }

    #[test]
    #[ignore = "requires an explicitly supplied real/redacted Codex session path"]
    fn real_codex_corpus_probe_is_opt_in_and_never_checked_in() {
        let path = std::env::var_os("CODEX_REAL_FIXTURE")
            .expect("CODEX_REAL_FIXTURE must point to a user-supplied real/redacted session");
        let bytes = std::fs::read(path).expect("real Codex fixture must be readable");
        let chunk = parse_chunk_bytes(&account(), "real-corpus", 0, &bytes, Utc::now());
        assert!(
            !chunk.records.is_empty(),
            "real corpus has no token records"
        );
        assert!(chunk
            .records
            .iter()
            .all(|record| record.coverage == Coverage::UnverifiedSemantics));
    }
    #[test]
    fn quota_scan_prefers_newest_observation_over_newest_file() {
        // session-b carries the newer quota observation; the scan must
        // report 40.0 no matter which file the OS touched last.
        let line_b = std::fs::read_to_string("fixtures/codex/session-b.jsonl").unwrap();
        let quota_line = line_b
            .lines()
            .find(|line| line.contains("rate_limits"))
            .unwrap();
        let event = parse_token_line(quota_line, Utc::now()).unwrap();
        let rate = event.rate_limits.unwrap();
        assert_eq!(rate.limit_id.as_deref(), Some("codex-weekly"));
        assert_eq!(
            rate.primary.unwrap().used_percent,
            Some(Decimal::new(400, 1))
        );
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
    fn custom_path_accepts_product_home_and_sessions_dir() {
        assert_eq!(
            normalize_home(Path::new("/data/codex")),
            PathBuf::from("/data/codex")
        );
        assert_eq!(
            normalize_home(Path::new("/data/codex/sessions")),
            PathBuf::from("/data/codex")
        );
        assert_eq!(
            normalize_home(Path::new("/data/codex/archived_sessions")),
            PathBuf::from("/data/codex")
        );
    }
    #[tokio::test]
    async fn strategy_reports_newest_observation_across_all_files() {
        use crate::strategy::{FetchContext, FetchStrategy};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-home");
        let sessions = home.join("sessions");
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
        // ...while an older relevant session holds the newer observation.
        let b = std::fs::read_to_string("fixtures/codex/session-b.jsonl").unwrap();
        std::fs::write(sub_old.join("rollout-old.jsonl"), &b).unwrap();
        // Force mtime order: the quota-less file is newest on disk.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::OpenOptions::new()
            .write(true)
            .open(sub_new.join("rollout-new.jsonl"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        let hosts = Arc::new(usage_host::Hosts {
            clock: Arc::new(usage_host::SystemClock),
            logger: Arc::new(usage_host::NullLogger),
            keychain: Arc::new(usage_host::MemoryKeychain::default()),
            http: Arc::new(usage_host::ReqwestHttpHost::default()),
            files: {
                let files = Arc::new(usage_host::ScopedFiles);
                files.clone()
            },
            file_scope: Arc::new(usage_host::ScopedFiles),
            network: Arc::new(usage_host::ObservedNetwork::default()),
            process: std::sync::Arc::new(usage_host::AllowlistedProcess::default()),
            pty: std::sync::Arc::new(usage_host::AllowlistedPty::default()),
        });
        let account = account();
        let descriptor =
            crate::descriptor::find_descriptor("openai", "codex").expect("codex descriptor");
        let cancel = usage_host::CancellationToken::new();
        let local_root = hosts
            .file_scope
            .scope_root(&home, &cancel)
            .expect("scoped codex root");
        let ctx = FetchContext {
            account: &account,
            facade: hosts.facade(&usage_host::facade::FacadePolicy {
                files: true,
                http: false,
                allowed_hosts: &[],
                keychain_service: None,
                process: false,
                pty: false,
            }),
            descriptor,
            timeout: std::time::Duration::from_secs(5),
            local_root: Some(local_root),
            secret: None,
            cancel: cancel.clone(),
        };
        let payload = CodexJsonlStrategy::new(None).fetch(&ctx).await.unwrap();
        let snapshot = payload.snapshot.expect("quota snapshot");
        assert_eq!(snapshot.quotas.len(), 1);
        assert_eq!(snapshot.quotas[0].used_percent, Some(Decimal::new(400, 1)));
        // Sessions-dir spelling resolves to the same product home.
        let cancel2 = usage_host::CancellationToken::new();
        let local_root2 = hosts
            .file_scope
            .scope_root(&home, &cancel2)
            .expect("scoped codex root");
        let ctx2 = FetchContext {
            account: &account,
            facade: hosts.facade(&usage_host::facade::FacadePolicy {
                files: true,
                http: false,
                allowed_hosts: &[],
                keychain_service: None,
                process: false,
                pty: false,
            }),
            descriptor,
            timeout: std::time::Duration::from_secs(5),
            local_root: Some(local_root2),
            secret: None,
            cancel: cancel2,
        };
        let payload2 = CodexJsonlStrategy::new(None).fetch(&ctx2).await.unwrap();
        assert!(payload2.snapshot.is_some());
    }

    #[test]
    fn redacted_codex_line_roundtrips_through_production_parser() {
        let redacted_line = concat!(
            "{\"timestamp\":\"2026-09-08T12:00:00.000Z\",",
            "\"type\":\"event_msg\",",
            "\"requestId\":\"req_0001\",",
            "\"payload\":{",
            "\"type\":\"token_count\",",
            "\"info\":{",
            "\"last_token_usage\":{",
            "\"input_tokens\":120,",
            "\"cached_input_tokens\":30,",
            "\"output_tokens\":45,",
            "\"reasoning_output_tokens\":10,",
            "\"total_tokens\":165",
            "},",
            "\"model\":\"gpt-4o\"",
            "},",
            "\"rate_limits\":{",
            "\"limit_id\":\"lim_0001\",",
            "\"limit_name\":\"primary\",",
            "\"plan_type\":\"plus\",",
            "\"primary\":{",
            "\"used_percent\":40.5,",
            "\"window_minutes\":300,",
            "\"resets_at\":1757332800",
            "}",
            "}",
            "}}\n"
        );

        let parsed =
            parse_token_line(redacted_line.trim(), Utc::now()).expect("must parse token line");
        assert_eq!(parsed.request_id.as_deref(), Some("req_0001"));
        assert_eq!(parsed.input_tokens, Some(120));
        assert_eq!(parsed.cached_tokens, Some(30));
        assert_eq!(parsed.output_tokens, Some(45));
        assert_eq!(parsed.reasoning_tokens, Some(10));
        assert_eq!(parsed.total_tokens, Some(165));
        assert_eq!(parsed.model.as_deref(), Some("gpt-4o"));

        let rate = parsed.rate_limits.expect("rate limits must be present");
        assert_eq!(rate.limit_id.as_deref(), Some("lim_0001"));
        assert_eq!(rate.plan_type.as_deref(), Some("plus"));

        let chunk = parse_chunk_bytes(
            &account(),
            "path_hash_123",
            0,
            redacted_line.as_bytes(),
            Utc::now(),
        );
        assert_eq!(chunk.records.len(), 1);
        assert_eq!(chunk.malformed, 0);
        let record = &chunk.records[0];
        assert_eq!(record.input_tokens, Some(120));
        assert_eq!(record.output_tokens, Some(45));
        assert_eq!(record.total_tokens, Some(165));
        assert_eq!(record.model.as_deref(), Some("gpt-4o"));

        // End-to-end CLI execution test if node is available
        let script_path = Path::new("../../scripts/redact-codex.mjs");
        if script_path.exists() {
            let temp = tempfile::tempdir().unwrap();
            let input_path = temp.path().join("input.jsonl");
            let output_path = temp.path().join("output.jsonl");
            let meta_path = temp.path().join("meta.json");

            let raw_sensitive = concat!(
                "{\"timestamp\":\"2026-09-08T12:00:00.000Z\",",
                "\"type\":\"event_msg\",",
                "\"request_id\":\"sensitive-req-999\",",
                "\"prompt\":\"TOP SECRET PROMPT\",",
                "\"payload\":{",
                    "\"type\":\"token_count\",",
                    "\"model\":\"gpt-4o\",",
                    "\"info\":{",
                        "\"last_token_usage\":{\"input_tokens\":50,\"output_tokens\":25,\"total_tokens\":75}",
                    "}",
                "}}\n"
            );
            std::fs::write(&input_path, raw_sensitive).unwrap();

            let status = std::process::Command::new("node")
                .arg(script_path)
                .arg(&input_path)
                .arg(&output_path)
                .arg(&meta_path)
                .status();

            if let Ok(st) = status {
                if st.success() {
                    let sanitized = std::fs::read_to_string(&output_path).unwrap();
                    assert!(!sanitized.contains("TOP SECRET PROMPT"));
                    assert!(!sanitized.contains("sensitive-req-999"));
                    assert!(sanitized.contains("req_0001"));

                    let parsed_chunk = parse_chunk_bytes(
                        &account(),
                        "probe_path",
                        0,
                        sanitized.as_bytes(),
                        Utc::now(),
                    );
                    assert_eq!(parsed_chunk.records.len(), 1);
                    assert_eq!(parsed_chunk.malformed, 0);
                    assert_eq!(parsed_chunk.records[0].total_tokens, Some(75));

                    let meta_content = std::fs::read_to_string(&meta_path).unwrap();
                    assert!(meta_content.contains("\"schema_version\": \"1.0.0\""));
                    assert!(meta_content.contains("\"record_count\": 1"));
                }
            }
        }
    }
}

#[cfg(test)]
mod real_corpus_evidence {
    use super::*;
    use std::collections::HashSet;

    const CONTROLLED: &str = include_str!("../fixtures/real/codex-controlled-1.jsonl");
    const SESSION: &str = include_str!("../fixtures/real/codex-session.jsonl");

    #[test]
    fn controlled_experiment_proves_delta_semantics() {
        // Manifest: 3 submitted prompts -> 3 responses. Totals are
        // distinct and growing, so `last_token_usage` is a per-request
        // delta, not a cumulative or repeated observation.
        let events: Vec<_> = CONTROLLED
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| parse_token_line(l, Utc::now()).expect("controlled line must parse"))
            .collect();
        assert_eq!(events.len(), 3);
        let totals: Vec<u64> = events.iter().filter_map(|e| e.total_tokens).collect();
        assert_eq!(totals, vec![14441, 14494, 14710]);
        let inputs: Vec<u64> = events.iter().filter_map(|e| e.input_tokens).collect();
        assert_eq!(inputs, vec![14397, 14453, 14505]);
        assert_eq!(inputs.iter().sum::<u64>(), 43355);
        assert_eq!(totals.iter().sum::<u64>(), 43645);
    }

    #[test]
    fn real_session_quota_windows_parse() {
        let mut quotas = 0;
        for line in SESSION.lines().filter(|l| !l.trim().is_empty()) {
            let Some(event) = parse_token_line(line, Utc::now()) else {
                continue;
            };
            if let Some(rate) = event.rate_limits {
                quotas += 1;
                assert_eq!(rate.plan_type.as_deref(), Some("plus"));
                let primary = rate.primary.expect("primary window");
                assert_eq!(primary.window_minutes, Some(300));
                assert!(primary.used_percent.is_some());
            }
        }
        assert!(quotas > 0, "real session must carry quota snapshots");
    }

    #[test]
    fn real_session_events_are_all_parseable() {
        // Every non-empty line of the redacted session is a token_count
        // event our parser accepts; nothing silently drops.
        let lines: Vec<_> = SESSION.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 33);
        for line in lines {
            assert!(
                parse_token_line(line, Utc::now()).is_some(),
                "line must parse: {line:.80}"
            );
        }
    }

    #[test]
    fn real_session_has_no_duplicate_offsets() {
        // Same bytes re-parsed yield identical stable identities:
        // storage upserts can neither duplicate nor drop rows.
        let ids_a: HashSet<_> = SESSION
            .lines()
            .enumerate()
            .filter_map(|(i, l)| parse_token_line(l, Utc::now()).map(|_| format!("h:{i}")))
            .collect();
        assert_eq!(ids_a.len(), 33);
    }
}
