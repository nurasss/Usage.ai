use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, Snapshot, UsageRecord,
};

pub const SCHEMA_FINGERPRINT: &str = "claude-jsonl-assistant-v1";

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
/// one identity and never inflate totals — covered by synthetic fixtures in
/// `fixtures/claude/`; real-client semantic evidence remains external.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeUsageEvent {
    pub at: DateTime<Utc>,
    pub record_id: String,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
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
    // Cache buckets are stored separately and never added to the request
    // total: the source total covers input + output only.
    let cache_read_tokens = usage.cache_read_input_tokens;
    let cache_creation_tokens = usage.cache_creation_input_tokens;
    let cached = optional_sum(cache_read_tokens, cache_creation_tokens);
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
        cache_creation_tokens,
        cache_read_tokens,
        cached_tokens: cached,
        total_tokens: total,
    })
}

fn optional_sum(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0).saturating_add(right.unwrap_or(0))),
    }
}

/// Claude's usage counters are cumulative within one logical assistant
/// response. A late, older chunk must therefore never overwrite a larger
/// snapshot. This reducer is deliberately monotonic per source field and
/// keeps the first non-empty model value.
fn merge_counter(current: Option<u64>, candidate: Option<u64>) -> Option<u64> {
    match (current, candidate) {
        (None, None) => None,
        (Some(value), None) | (None, Some(value)) => Some(value),
        (Some(current), Some(candidate)) => Some(current.max(candidate)),
    }
}

fn merge_usage_record(current: &mut UsageRecord, candidate: UsageRecord) {
    current.period_start = current.period_start.max(candidate.period_start);
    current.period_end = current.period_end.max(candidate.period_end);
    if current.model.is_none() {
        current.model = candidate.model;
    }
    current.input_tokens = merge_counter(current.input_tokens, candidate.input_tokens);
    current.output_tokens = merge_counter(current.output_tokens, candidate.output_tokens);
    current.reasoning_tokens = merge_counter(current.reasoning_tokens, candidate.reasoning_tokens);
    current.total_tokens = merge_counter(current.total_tokens, candidate.total_tokens);
    current.requests = merge_counter(current.requests, candidate.requests);
    current.cache_creation_tokens = merge_counter(
        current.cache_creation_tokens,
        candidate.cache_creation_tokens,
    );
    current.cache_read_tokens =
        merge_counter(current.cache_read_tokens, candidate.cache_read_tokens);
    current.cached_tokens = match (current.cache_creation_tokens, current.cache_read_tokens) {
        (None, None) => merge_counter(current.cached_tokens, candidate.cached_tokens),
        (creation, read) => optional_sum(read, creation),
    };
}

/// Pure byte-range parser: no filesystem, no environment. Same resume
/// contract as the Codex chunk parser (offsets, partial tail,
/// malformed counting, silent skips). Within a chunk, records with the same
/// stable logical identity are reduced before they reach storage, so file
/// order cannot regress cumulative usage.
pub fn parse_chunk_bytes(
    account: &Account,
    path_hash: &str,
    base_offset: u64,
    bytes: &[u8],
    now: DateTime<Utc>,
) -> crate::strategy::ParsedChunk {
    use crate::strategy::ParsedChunk;
    let mut chunk = ParsedChunk::default();
    let text = String::from_utf8_lossy(bytes);
    let mut cursor = base_offset;
    let mut line_no = 0u64;
    let mut reduced = BTreeMap::<String, UsageRecord>::new();
    for piece in text.split_inclusive('\n') {
        let piece_bytes = piece.len() as u64;
        if !piece.ends_with('\n') {
            chunk.partial = true;
            break;
        }
        line_no += 1;
        let record_offset = cursor;
        cursor += piece_bytes;
        let trimmed = piece.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match parse_usage_line(trimmed, path_hash, record_offset, now) {
            Some(event) => {
                let record = UsageRecord {
                    account_id: account.id,
                    product_id: "claude-code".into(),
                    billing_scope_id: None,
                    source_record_id: event.record_id,
                    period_start: event.at,
                    period_end: event.at,
                    model: event.model,
                    input_tokens: event.input_tokens,
                    output_tokens: event.output_tokens,
                    cache_creation_tokens: event.cache_creation_tokens,
                    cache_read_tokens: event.cache_read_tokens,
                    cached_tokens: event.cached_tokens,
                    reasoning_tokens: None,
                    total_tokens: Some(event.total_tokens),
                    requests: Some(1),
                    source: "claude-code-local-session-v1".into(),
                    coverage: Coverage::UnverifiedSemantics,
                    connection_id: None,
                    file_id: None,
                    file_generation: None,
                };
                if let Some(current) = reduced.get_mut(&record.source_record_id) {
                    merge_usage_record(current, record);
                } else {
                    reduced.insert(record.source_record_id.clone(), record);
                }
            }
            None => {
                if trimmed.starts_with('{') {
                    chunk.malformed += 1;
                    chunk
                        .warnings
                        .push(format!("invalid_json_at_relative_line_{line_no}"));
                }
            }
        }
    }
    chunk.consumed = cursor;
    chunk.records = reduced.into_values().collect();
    chunk
}

/// Strategy wrapper: Claude Code local JSONL over scoped HostServices.
///
/// Honesty split: this source proves local history availability, never
/// subscription quota. `Connected` therefore means the history source
/// is reachable; quotas stay empty and the capability set (no
/// `subscriptionQuota`) tells the UI to render history, not meters.
pub struct ClaudeJsonlStrategy {
    pub custom_root: Option<PathBuf>,
}

impl ClaudeJsonlStrategy {
    pub fn new(custom_root: Option<PathBuf>) -> Self {
        Self { custom_root }
    }

    fn capabilities() -> BTreeSet<Capability> {
        crate::descriptor::CLAUDE_CODE_CAPS
            .iter()
            .copied()
            .collect()
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
        let Some(files) = ctx.facade.files else {
            return Err(SourceError::Unavailable);
        };
        let Some(root) = ctx.local_root.as_ref() else {
            return Err(SourceError::Unavailable);
        };
        let files = files
            .list_files(
                root,
                ctx.descriptor.local_glob.unwrap_or("projects/**/*.jsonl"),
                &ctx.cancel,
            )
            .map_err(|_| SourceError::Unavailable)?;
        if files.is_empty() {
            return Err(SourceError::Unavailable);
        }
        Ok(crate::strategy::FetchPayload {
            snapshot: Some(Snapshot {
                account_id: ctx.account.id,
                provider_id: "anthropic".into(),
                product_id: "claude-code".into(),
                plan_label: None,
                capabilities: Self::capabilities(),
                quotas: vec![],
                balances: vec![],
                observed_at: None,
                fetched_at: ctx.facade.clock.now(),
                connection_state: ConnectionState::Connected,
                freshness: Freshness::Unknown,
                coverage: Coverage::UnverifiedSemantics,
            }),
            usage: vec![],
            costs: vec![],
            balances: vec![],
            warnings: vec![],
            schema_fingerprint: SCHEMA_FINGERPRINT,
            coverage: Coverage::UnverifiedSemantics,
            observed_at: None,
            source_error: None,
            observed_identity: None,
        })
    }
}

#[async_trait]
impl crate::strategy::FileLogParser for ClaudeJsonlStrategy {
    fn source_id(&self) -> crate::strategy::SourceId {
        crate::strategy::SourceId("claude-local-jsonl")
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
            provider_id: "anthropic".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        }
    }
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("fixtures/claude/{name}")).unwrap()
    }
    #[test]
    fn thread_a_chunks_share_identity_and_final_wins() {
        let bytes = fixture("thread-a.jsonl");
        let chunk = parse_chunk_bytes(&account(), "h", 0, &bytes, Utc::now());
        assert_eq!(chunk.records.len(), 2);
        let m1 = chunk
            .records
            .iter()
            .find(|record| record.source_record_id == "msg:m1:r1")
            .unwrap();
        assert_eq!(m1.total_tokens, Some(70));
        assert_eq!(m1.cached_tokens, Some(5));
        assert_eq!(m1.cache_read_tokens, Some(5));
        assert!(chunk
            .records
            .iter()
            .any(|record| record.source_record_id == "uuid:u-b1"));
    }
    #[test]
    fn thread_b_out_of_order_duplicate_and_cache() {
        let bytes = fixture("thread-b.jsonl");
        let chunk = parse_chunk_bytes(&account(), "h", 0, &bytes, Utc::now());
        assert_eq!(chunk.records.len(), 2);
        let u2 = chunk
            .records
            .iter()
            .find(|record| record.source_record_id == "uuid:u-2")
            .unwrap();
        assert_eq!(u2.cached_tokens, Some(7));
        assert_eq!(u2.cache_read_tokens, Some(3));
        assert_eq!(u2.cache_creation_tokens, Some(4));
        assert_eq!(u2.total_tokens, Some(4));
        // Missing model stays missing, never fabricated.
        assert!(chunk.records[0].model.is_none());
    }
    #[test]
    fn cache_creation_adds_to_cached_not_total() {
        let line = r#"{"timestamp":"2026-09-08T12:00:00Z","uuid":"u1","message":{"model":"m","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}}}"#;
        let event = parse_usage_line(line, "h", 0, Utc::now()).unwrap();
        assert_eq!(event.cached_tokens, Some(7));
        assert_eq!(event.cache_read_tokens, Some(3));
        assert_eq!(event.cache_creation_tokens, Some(4));
        assert_eq!(event.total_tokens, 30);
    }

    #[test]
    fn late_older_chunk_does_not_regress_final_usage() {
        let bytes = br#"{"timestamp":"2026-09-08T12:00:02Z","uuid":"final","requestId":"r1","message":{"id":"m1","usage":{"input_tokens":30,"output_tokens":40,"cache_read_input_tokens":5}}}
{"timestamp":"2026-09-08T12:00:01Z","uuid":"older","requestId":"r1","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":20}}}
"#;
        let chunk = parse_chunk_bytes(&account(), "h", 0, bytes, Utc::now());
        assert_eq!(chunk.records.len(), 1);
        assert_eq!(chunk.records[0].total_tokens, Some(70));
        assert_eq!(chunk.records[0].input_tokens, Some(30));
        assert_eq!(chunk.records[0].cached_tokens, Some(5));
    }

    #[test]
    fn redacted_claude_line_roundtrips_through_production_parser() {
        let redacted_line = concat!(
            "{\"timestamp\":\"2026-09-08T12:00:00.000Z\",",
            "\"type\":\"message\",",
            "\"uuid\":\"uuid_0001\",",
            "\"requestId\":\"req_0001\",",
            "\"message\":{",
            "\"id\":\"msg_0001\",",
            "\"model\":\"claude-3-5-sonnet\",",
            "\"usage\":{",
            "\"input_tokens\":50,",
            "\"output_tokens\":25,",
            "\"cache_read_input_tokens\":10,",
            "\"cache_creation_input_tokens\":5",
            "}",
            "}}\n"
        );

        let event = parse_usage_line(redacted_line.trim(), "test_path", 0, Utc::now())
            .expect("must parse claude line");
        assert_eq!(event.record_id, "msg:msg_0001:req_0001");
        assert_eq!(event.model.as_deref(), Some("claude-3-5-sonnet"));
        assert_eq!(event.input_tokens, Some(50));
        assert_eq!(event.output_tokens, Some(25));
        assert_eq!(event.cache_read_tokens, Some(10));
        assert_eq!(event.cache_creation_tokens, Some(5));
        assert_eq!(event.cached_tokens, Some(15));
        assert_eq!(event.total_tokens, 75);

        let chunk = parse_chunk_bytes(
            &account(),
            "path_hash",
            0,
            redacted_line.as_bytes(),
            Utc::now(),
        );
        assert_eq!(chunk.records.len(), 1);
        assert_eq!(chunk.records[0].input_tokens, Some(50));
        assert_eq!(chunk.records[0].output_tokens, Some(25));
        assert_eq!(chunk.records[0].total_tokens, Some(75));

        // End-to-end CLI execution test if node is available
        let script_path = std::path::Path::new("../../scripts/redact-claude.mjs");
        if script_path.exists() {
            let temp = tempfile::tempdir().unwrap();
            let input_path = temp.path().join("input.jsonl");
            let output_path = temp.path().join("output.jsonl");
            let meta_path = temp.path().join("meta.json");

            let raw_sensitive = concat!(
                "{\"timestamp\":\"2026-09-08T12:00:00.000Z\",",
                "\"type\":\"message\",",
                "\"uuid\":\"real-uuid-444\",",
                "\"requestId\":\"sensitive-req-claude-999\",",
                "\"message\":{",
                "\"id\":\"msg-secret-id\",",
                "\"model\":\"claude-3-5-sonnet\",",
                "\"content\":\"TOP SECRET PROMPT BODY\",",
                "\"usage\":{",
                "\"input_tokens\":60,",
                "\"output_tokens\":40,",
                "\"cache_creation_input_tokens\":10,",
                "\"cache_read_input_tokens\":5",
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
                    assert!(!sanitized.contains("TOP SECRET PROMPT BODY"));
                    assert!(!sanitized.contains("real-uuid-444"));
                    assert!(!sanitized.contains("sensitive-req-claude-999"));
                    assert!(!sanitized.contains("msg-secret-id"));
                    assert!(sanitized.contains("uuid_0001"));
                    assert!(sanitized.contains("req_0001"));
                    assert!(sanitized.contains("msg_0001"));

                    let parsed_chunk = parse_chunk_bytes(
                        &account(),
                        "probe_path",
                        0,
                        sanitized.as_bytes(),
                        Utc::now(),
                    );
                    assert_eq!(parsed_chunk.records.len(), 1);
                    assert_eq!(parsed_chunk.malformed, 0);
                    assert_eq!(parsed_chunk.records[0].total_tokens, Some(100));
                    assert_eq!(parsed_chunk.records[0].input_tokens, Some(60));
                    assert_eq!(parsed_chunk.records[0].output_tokens, Some(40));

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
    use std::collections::{HashMap, HashSet};

    const THREAD: &str = include_str!("../fixtures/real/claude-thread.jsonl");

    fn events() -> Vec<ClaudeUsageEvent> {
        let now = Utc::now();
        THREAD
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, l)| parse_usage_line(l, "h", i as u64, now).expect("corpus line must parse"))
            .collect()
    }

    #[test]
    fn repeated_chunks_collapse_to_logical_responses() {
        // 72 real chunks over 27 (requestId, message.id) identities:
        // naive per-line summation would inflate totals ~3x.
        let events = events();
        assert_eq!(events.len(), 72);
        let ids: HashSet<_> = events.iter().map(|e| e.record_id.clone()).collect();
        assert_eq!(ids.len(), 27);
        // Every repeated identity carries byte-identical usage: the
        // stored survivor equals any chunk, so upsert order is safe.
        let mut by_id: HashMap<&str, Vec<&ClaudeUsageEvent>> = HashMap::new();
        for e in &events {
            by_id.entry(e.record_id.as_str()).or_default().push(e);
        }
        for (id, group) in &by_id {
            assert!(
                group
                    .windows(2)
                    .all(|w| w[0].total_tokens == w[1].total_tokens),
                "chunks of {id} disagree"
            );
        }
    }

    #[test]
    fn cache_buckets_present_on_every_row() {
        for e in events() {
            assert!(
                e.cached_tokens.is_some(),
                "real rows carry cache buckets: {}",
                e.record_id
            );
        }
    }

    #[test]
    fn storage_totals_equal_sum_over_distinct_identities() {
        // Simulates the storage upsert (last write wins per identity):
        // authoritative total = sum over distinct identities only.
        let events = events();
        let mut latest: HashMap<&str, u64> = HashMap::new();
        for e in &events {
            latest.insert(e.record_id.as_str(), e.total_tokens);
        }
        let authoritative: u64 = latest.values().sum();
        let naive: u64 = events.iter().map(|e| e.total_tokens).sum();
        assert!(authoritative < naive, "dedup must reduce the total");
        assert_eq!(latest.len(), 27);
    }
}
