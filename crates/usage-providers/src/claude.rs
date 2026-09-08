use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, ImportBatch, ProviderAdapter,
    ProviderError, Snapshot, UsageRecord,
};

pub const SCHEMA_FINGERPRINT: &str = "claude-jsonl-assistant-v1";

#[derive(Default)]
pub struct ClaudeLocalAdapter {
    pub projects_root: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Envelope {
    timestamp: Option<DateTime<Utc>>,
    uuid: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(rename = "id")]
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

/// Stable source identity priority: `message.id + requestId` where the
/// schema confirms them, else envelope `uuid`, else `path:offset`.
/// Repeated streaming chunks of one logical response therefore share
/// one identity and never inflate totals.
pub fn stable_record_id(
    message_id: Option<&str>,
    request_id: Option<&str>,
    uuid: Option<&str>,
    path_hash: &str,
    offset: u64,
) -> String {
    match (message_id, request_id) {
        (Some(mid), Some(rid)) if !mid.is_empty() && !rid.is_empty() => {
            format!("msg:{mid}:{rid}")
        }
        _ => uuid
            .filter(|u| !u.is_empty())
            .map(|u| format!("uuid:{u}"))
            .unwrap_or_else(|| format!("{path_hash}:{offset}")),
    }
}

fn resolve_projects_root(custom_root: Option<&Path>) -> PathBuf {
    if let Some(root) = custom_root {
        return root.to_path_buf();
    }
    dirs_home().join(".claude").join("projects")
}

fn has_history_files(root: &Path) -> bool {
    glob::glob(&format!("{}/**/*.jsonl", root.display()))
        .ok()
        .and_then(|mut it| it.find_map(Result::ok))
        .is_some()
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeUsageEvent {
    pub at: DateTime<Utc>,
    pub record_id: String,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub total_tokens: u64,
}

pub fn parse_usage_line(
    line: &str,
    path_hash: &str,
    offset: u64,
    now: DateTime<Utc>,
) -> Option<ClaudeUsageEvent> {
    let envelope: Envelope = serde_json::from_str(line).ok()?;
    let message = envelope.message?;
    let usage = message.usage?;
    if usage.input_tokens.is_none() && usage.output_tokens.is_none() {
        return None;
    }
    let at = envelope.timestamp.unwrap_or(now);
    let cached = usage
        .cache_read_input_tokens
        .unwrap_or(0)
        .saturating_add(usage.cache_creation_input_tokens.unwrap_or(0));
    let total = usage
        .input_tokens
        .unwrap_or(0)
        .saturating_add(usage.output_tokens.unwrap_or(0));
    Some(ClaudeUsageEvent {
        at,
        record_id: stable_record_id(
            message.id.as_deref(),
            envelope.request_id.as_deref(),
            envelope.uuid.as_deref(),
            path_hash,
            offset,
        ),
        model: message.model,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_tokens: if cached > 0 { Some(cached) } else { None },
        total_tokens: total,
    })
}

impl ClaudeLocalAdapter {
    fn root(&self) -> PathBuf {
        resolve_projects_root(self.projects_root.as_deref())
    }
}

#[async_trait]
impl ProviderAdapter for ClaudeLocalAdapter {
    fn provider_id(&self) -> &'static str {
        "anthropic"
    }
    fn product_id(&self) -> &'static str {
        "claude-code"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        crate::descriptor::CLAUDE_CODE_CAPS
            .iter()
            .copied()
            .collect()
    }
    fn allowed_hosts(&self) -> &'static [&'static str] {
        &[]
    }
    async fn refresh(&self, account: &Account) -> Result<Snapshot, ProviderError> {
        // Honesty split: this source proves local history availability,
        // never subscription quota. Connected therefore means the history
        // source is reachable; quotas stay empty and the capability set
        // (no subscriptionQuota) tells the UI to render history, not meters.
        if !has_history_files(&self.root()) {
            return Err(ProviderError::Unavailable(
                "claude_history_not_found".into(),
            ));
        }
        Ok(Snapshot {
            account_id: account.id,
            provider_id: "anthropic".into(),
            product_id: "claude-code".into(),
            plan_label: None,
            capabilities: self.capabilities(),
            quotas: vec![],
            balances: vec![],
            observed_at: None,
            fetched_at: Utc::now(),
            connection_state: ConnectionState::Connected,
            freshness: Freshness::Unknown,
            coverage: Coverage::LocalClientOnly,
        })
    }
    async fn import_local(
        &self,
        account: &Account,
        path: &Path,
        offset: u64,
    ) -> Result<ImportBatch, ProviderError> {
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
        let mut records = vec![];
        let mut warnings = vec![];
        let mut partial = false;
        let mut line = String::new();
        let path_hash = crate::anonymous_path_identity(path);
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
                partial = true;
                break;
            }
            let record_offset = cursor;
            cursor += bytes as u64;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match parse_usage_line(trimmed, &path_hash, record_offset, Utc::now()) {
                Some(event) => records.push(UsageRecord {
                    account_id: account.id,
                    product_id: "claude-code".into(),
                    billing_scope_id: None,
                    source_record_id: event.record_id,
                    period_start: event.at,
                    period_end: event.at,
                    model: event.model,
                    input_tokens: event.input_tokens,
                    output_tokens: event.output_tokens,
                    cached_tokens: event.cached_tokens,
                    reasoning_tokens: None,
                    total_tokens: Some(event.total_tokens),
                    requests: Some(1),
                    source: "claude-code-local-session-v1".into(),
                    coverage: Coverage::LocalClientOnly,
                }),
                None => {
                    if looks_like_broken_json(trimmed) {
                        warnings.push(format!("invalid_json_at_relative_line_{line_no}"));
                    }
                }
            }
        }
        Ok(ImportBatch {
            records,
            next_offset: cursor,
            partial_line: partial,
            warnings,
        })
    }
}

fn looks_like_broken_json(trimmed: &str) -> bool {
    trimmed.starts_with('{') && serde_json::from_str::<serde_json::Value>(trimmed).is_err()
}

/// Strategy wrapper: Claude Code local JSONL over scoped HostServices.
pub struct ClaudeJsonlStrategy {
    pub adapter: ClaudeLocalAdapter,
}

impl ClaudeJsonlStrategy {
    pub fn new(custom_root: Option<PathBuf>) -> Self {
        Self {
            adapter: ClaudeLocalAdapter {
                projects_root: custom_root,
            },
        }
    }
}

#[async_trait]
impl crate::strategy::FetchStrategy for ClaudeJsonlStrategy {
    fn source_id(&self) -> crate::strategy::SourceId {
        crate::strategy::SourceId("claude-local-jsonl")
    }
    fn classification(&self) -> crate::strategy::SourceClassification {
        crate::strategy::SourceClassification::LocalStructuredData
    }
    async fn availability(
        &self,
        _ctx: &crate::strategy::FetchContext<'_>,
    ) -> crate::strategy::Availability {
        if has_history_files(&self.adapter.root()) {
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
            .map_err(|_| crate::strategy::SourceError::Unavailable)?;
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
    #[tokio::test]
    async fn parses_usage_without_double_counting_cache() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            f,
            "{}",
            include_str!("../fixtures/claude-usage.jsonl").trim()
        )
        .unwrap();
        let a = Account {
            id: Uuid::nil(),
            provider_id: "anthropic".into(),
            external_identity: None,
            label: "Аккаунт 1".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        let b = ClaudeLocalAdapter::default()
            .import_local(&a, f.path(), 0)
            .await
            .unwrap();
        assert_eq!(b.records[0].total_tokens, Some(30));
        assert_eq!(b.records[0].cached_tokens, Some(5));
    }
    #[test]
    fn cache_creation_adds_to_cached_not_total() {
        let line = r#"{"timestamp":"2026-09-08T12:00:00Z","uuid":"u1","message":{"model":"m","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}}}"#;
        let event = parse_usage_line(line, "h", 0, Utc::now()).unwrap();
        assert_eq!(event.cached_tokens, Some(7));
        assert_eq!(event.total_tokens, 30);
    }
    #[test]
    fn streaming_chunks_share_one_stable_identity() {
        let chunk_a = r#"{"timestamp":"2026-09-08T12:00:00Z","uuid":"different-1","requestId":"r1","message":{"id":"m1","model":"x","usage":{"input_tokens":10,"output_tokens":20}}}"#;
        let chunk_b = r#"{"timestamp":"2026-09-08T12:00:01Z","uuid":"different-2","requestId":"r1","message":{"id":"m1","model":"x","usage":{"input_tokens":10,"output_tokens":20}}}"#;
        let now = Utc::now();
        let a = parse_usage_line(chunk_a, "h", 0, now).unwrap();
        let b = parse_usage_line(chunk_b, "h", 99, now).unwrap();
        assert_eq!(a.record_id, b.record_id);
        assert!(a.record_id.starts_with("msg:m1:r1"));
    }
    #[test]
    fn uuid_fallback_then_offset_fallback() {
        let now = Utc::now();
        let with_uuid = parse_usage_line(
            r#"{"uuid":"u-9","message":{"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            "h",
            5,
            now,
        )
        .unwrap();
        assert_eq!(with_uuid.record_id, "uuid:u-9");
        let bare = parse_usage_line(
            r#"{"message":{"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            "h",
            5,
            now,
        )
        .unwrap();
        assert_eq!(bare.record_id, "h:5");
    }
    #[test]
    fn malformed_lines_warn_but_do_not_stop_import() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut f = tempfile::NamedTempFile::new().unwrap();
            writeln!(f, "{{\"broken\": ").unwrap();
            writeln!(
                f,
                "{}",
                include_str!("../fixtures/claude-usage.jsonl").trim()
            )
            .unwrap();
            let a = Account {
                id: Uuid::nil(),
                provider_id: "anthropic".into(),
                external_identity: None,
                label: "T".into(),
                connection_ref: None,
                lifecycle: usage_core::AccountLifecycle::Active,
            };
            let batch = ClaudeLocalAdapter::default()
                .import_local(&a, f.path(), 0)
                .await
                .unwrap();
            assert_eq!(batch.records.len(), 1);
            assert_eq!(batch.warnings.len(), 1);
        });
    }
}
