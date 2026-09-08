use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, ImportBatch, ProviderAdapter,
    ProviderError, Snapshot, UsageRecord,
};

#[derive(Default)]
pub struct ClaudeLocalAdapter;
#[derive(Deserialize)]
struct Envelope {
    timestamp: Option<DateTime<Utc>>,
    uuid: Option<String>,
    message: Option<Message>,
}
#[derive(Deserialize)]
struct Message {
    model: Option<String>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
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
        [
            Capability::ApiTokens,
            Capability::HistoryLocal,
            Capability::LocalSessions,
            Capability::ModelBreakdown,
        ]
        .into_iter()
        .collect()
    }
    fn allowed_hosts(&self) -> &'static [&'static str] {
        &[]
    }
    async fn refresh(&self, account: &Account) -> Result<Snapshot, ProviderError> {
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
        loop {
            line.clear();
            let bytes = reader
                .read_line(&mut line)
                .map_err(|e| ProviderError::Parse(e.to_string()))?;
            if bytes == 0 {
                break;
            }
            if !line.ends_with('\n') {
                partial = true;
                break;
            }
            let record_offset = cursor;
            cursor += bytes as u64;
            let e: Envelope = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    warnings.push(format!("invalid_json_at_offset_{record_offset}"));
                    continue;
                }
            };
            let Some(message) = e.message else { continue };
            let Some(u) = message.usage else { continue };
            let at = e.timestamp.unwrap_or_else(Utc::now);
            let total = u
                .input_tokens
                .unwrap_or(0)
                .saturating_add(u.output_tokens.unwrap_or(0));
            records.push(UsageRecord {
                account_id: account.id,
                product_id: "claude-code".into(),
                billing_scope_id: None,
                source_record_id: e.uuid.unwrap_or_else(|| {
                    format!("{}:{record_offset}", crate::anonymous_path_identity(path))
                }),
                period_start: at,
                period_end: at,
                model: message.model,
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cached_tokens: u.cache_read_input_tokens,
                reasoning_tokens: None,
                total_tokens: Some(total),
                requests: Some(1),
                source: "claude-code-local-session-v1".into(),
                coverage: Coverage::LocalClientOnly,
            });
        }
        Ok(ImportBatch {
            records,
            next_offset: cursor,
            partial_line: partial,
            warnings,
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
        let b = ClaudeLocalAdapter
            .import_local(&a, f.path(), 0)
            .await
            .unwrap();
        assert_eq!(b.records[0].total_tokens, Some(30));
        assert_eq!(b.records[0].cached_tokens, Some(5));
    }
}
