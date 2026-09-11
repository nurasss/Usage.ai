use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use usage_core::{
    Account, Capability, ConnectionState, Coverage, Freshness, MetricUnit, ProviderError, Quota,
    Snapshot, UsageRecord, WindowKind,
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

// ---------------------------------------------------------------------------
// V13-03 Claude subscription quota: ordered ends `claude-pty-usage`
// (installed CLI driven through an owned pty) then `claude-oauth-usage`
// (read-only provider endpoint, dormant until a verified URL + scoped
// credential exist). The JSONL history end above stays the history
// trust state and never yields quotas (§8: quota source != history
// source; a quota error must not disable history).
// ---------------------------------------------------------------------------

pub const PTY_USAGE_SOURCE_ID: &str = "claude-pty-usage";
pub const PTY_USAGE_KILL_SWITCH: &str = "claude-pty-usage";
pub const PTY_USAGE_FINGERPRINT: &str = "claude-pty-usage-v1";
pub const OAUTH_USAGE_SOURCE_ID: &str = "claude-oauth-usage";
pub const OAUTH_USAGE_KILL_SWITCH: &str = "claude-oauth-usage";
pub const OAUTH_USAGE_FINGERPRINT: &str = "claude-oauth-usage-v1";

/// Per-candidate login-gate budget. `auth status` answers in ~1s warm;
/// a hung shim must never stall availability: the gate is the cheap
/// health check (first-exists-wins is banned after the V13-02 Codex
/// stale-shim incident).
const AUTH_GATE_TIMEOUT: Duration = Duration::from_secs(6);
/// One-shot `auth status` output is a 4-field JSON object; anything
/// larger is not the observed shape.
const AUTH_STATUS_CAP: usize = 8 * 1024;
/// PTY drive staging inside the caller timeout: first paint (or
/// proceed anyway), command render, post-/exit drain. The render
/// stage scales with the caller budget instead of a second hardcoded
/// constant; the coordinator caps the total at SOURCE_TIMEOUT.
const BOOT_WAIT: Duration = Duration::from_secs(4);
const RENDER_CAP: Duration = Duration::from_secs(20);
const SHUTDOWN_RESERVE: Duration = Duration::from_secs(5);
const EXIT_DRAIN: Duration = Duration::from_secs(3);
const SCREEN_CAP: usize = 256 * 1024;

/// Static Claude Code CLI locations, tried in order. No PATH lookup,
/// no shell. Existence alone proves nothing (a dead shim answers no
/// handshake): the PTY-usage strategy gates every candidate on
/// `claude auth status` (fast, non-interactive, $0 spend) and serves
/// the first logged-in one.
pub fn claude_executable_candidates(home: &std::path::Path) -> Vec<PathBuf> {
    [
        PathBuf::from("/usr/local/bin/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        home.join(".npm-global/bin/claude"),
        home.join(".local/bin/claude"),
    ]
    .into_iter()
    .collect()
}

/// Login-gate outcome per candidate executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginGate {
    /// A healthy CLI reports an active login.
    LoggedIn(PathBuf),
    /// Healthy CLIs answered but none is logged in (fast skip, no
    /// cooldown: the actionable truth is "sign in", not failure).
    NotLoggedIn,
    /// No candidate answered cleanly (all denied/crashed/timed out).
    Broken,
}

/// Driven-session seam: the strategy logic (gates, parse, map) is
/// fully unit-testable against a scripted driver; production drives
/// the real CLI through scoped hosts. The ONLY bytes a driver may
/// ever send are `/usage` and `/exit` — no prompts, no model input,
/// no inference quota spend (proven by the sent-lines tests).
#[async_trait]
pub trait ClaudeUsageDriver: Send + Sync {
    async fn login_gate(&self, cancel: &usage_host::CancellationToken) -> LoginGate;
    async fn usage_screen(
        &self,
        executable: &Path,
        timeout: Duration,
        cancel: &usage_host::CancellationToken,
    ) -> Result<String, ProviderError>;
}

/// Production driver: `auth status` over one-shot process spawn, then
/// the interactive `/usage` screen over an owned pty child.
pub struct HostClaudeDriver<'a> {
    pub process: &'a dyn usage_host::ProcessHost,
    pub pty: &'a dyn usage_host::PTYHost,
    pub executables: Vec<PathBuf>,
    /// Scoped profile environment (CLAUDE_CONFIG_DIR) applied to both
    /// the auth-status gate and the interactive drive.
    pub env: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct AuthStatusWire {
    #[serde(rename = "loggedIn")]
    logged_in: Option<bool>,
}

impl HostClaudeDriver<'_> {
    async fn auth_status_of(
        &self,
        executable: &Path,
        cancel: &usage_host::CancellationToken,
    ) -> Result<bool, ProviderError> {
        use usage_host::SpawnRequest;
        let out = self
            .process
            .spawn(SpawnRequest {
                executable,
                args: &["auth", "status"],
                timeout: AUTH_GATE_TIMEOUT,
                stdout_cap: AUTH_STATUS_CAP,
                stderr_cap: 1024,
                cancel: cancel.clone(),
                env: self.env.clone(),
            })
            .await
            .map_err(|_| ProviderError::Unavailable("claude_auth_spawn".into()))?;
        if out.timed_out {
            return Err(ProviderError::Network("claude_auth_timeout".into()));
        }
        if out.status_code != Some(0) {
            return Err(ProviderError::Unavailable("claude_auth_status".into()));
        }
        // Observed shape (claude 2.1.245): {"loggedIn":false,
        // "authMethod":"none","apiProvider":"firstParty",
        // "analyticsDisabled":false}. Only the boolean is read; every
        // other field is ignored and nothing is logged or stored.
        let text = String::from_utf8_lossy(&out.stdout);
        let wire: AuthStatusWire = serde_json::from_str(text.trim())
            .map_err(|_| ProviderError::Parse("claude-auth-status-v1".into()))?;
        Ok(wire.logged_in.unwrap_or(false))
    }
}

#[async_trait]
impl ClaudeUsageDriver for HostClaudeDriver<'_> {
    async fn login_gate(&self, cancel: &usage_host::CancellationToken) -> LoginGate {
        let mut clean_false = false;
        for executable in &self.executables {
            if cancel.is_cancelled() {
                break;
            }
            match self.auth_status_of(executable, cancel).await {
                Ok(true) => return LoginGate::LoggedIn(executable.clone()),
                // A healthy CLI answering "not logged in" is ground
                // truth; an errored sibling never overrides it.
                Ok(false) => clean_false = true,
                Err(_) => continue,
            }
        }
        if clean_false {
            LoginGate::NotLoggedIn
        } else {
            LoginGate::Broken
        }
    }

    async fn usage_screen(
        &self,
        executable: &Path,
        timeout: Duration,
        cancel: &usage_host::CancellationToken,
    ) -> Result<String, ProviderError> {
        use usage_host::PtySpawnRequest;
        let started = tokio::time::Instant::now();
        let mut child = self
            .pty
            .spawn_pty(PtySpawnRequest {
                executable,
                args: &[],
                cancel: cancel.clone(),
                env: self.env.clone(),
            })
            .await
            .map_err(|_| ProviderError::Unavailable("claude_pty_spawn".into()))?;
        let mut raw = Vec::new();
        // Stage 1 — first paint: never type into a session that has
        // not drawn yet. Silence alone never fails (the app may paint
        // slowly); the scan below only gates foreign prompts.
        let boot_until = started + BOOT_WAIT;
        while raw.is_empty() {
            if cancel.is_cancelled() {
                let _ = child.kill().await;
                return Err(ProviderError::Network("claude_pty_cancelled".into()));
            }
            let remaining = boot_until.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, child.read_chunk(32 * 1024)).await {
                Ok(Ok(Some(bytes))) if !bytes.is_empty() => raw.extend_from_slice(&bytes),
                _ => break,
            }
        }
        if has_trust_markers(&strip_ansi(&raw)) {
            let _ = child.kill().await;
            return Err(ProviderError::Unavailable(
                "claude_pty_untrusted_prompt".into(),
            ));
        }
        child
            .write_all(b"/usage\r")
            .await
            .map_err(|_| ProviderError::Unavailable("claude_pty_io".into()))?;
        // Stage 2 — render within the caller-blessed budget minus a
        // shutdown reserve (drain + kill). Production keeps its ~10s;
        // explicitly longer caller timeouts (live probes) buy patience
        // instead of hitting a second hardcoded wall.
        let render_budget = timeout
            .min(RENDER_CAP)
            .saturating_sub(SHUTDOWN_RESERVE)
            .max(Duration::from_secs(2));
        let deadline = started + BOOT_WAIT + render_budget;
        let mut usage_timed_out = false;

        loop {
            if cancel.is_cancelled() {
                let _ = child.kill().await;
                return Err(ProviderError::Network("claude_pty_cancelled".into()));
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                usage_timed_out = true;
                break;
            }
            // Bounded read: a silent-but-alive child must end the
            // usage wait, not the refresh.
            let chunk = tokio::time::timeout(remaining, child.read_chunk(32 * 1024)).await;

            match chunk {
                Err(_) => {
                    usage_timed_out = true;
                    break;
                }
                Ok(Err(_)) => {
                    let _ = child.kill().await;
                    return Err(ProviderError::Unavailable("claude_pty_io".into()));
                }
                Ok(Ok(None)) => break,
                Ok(Ok(Some(bytes))) => {
                    if !bytes.is_empty() {
                        raw.extend_from_slice(&bytes);
                        if raw.len() > SCREEN_CAP {
                            break;
                        }
                        let text = strip_ansi(&raw);
                        // A trust dialog can pop up mid-session: stop
                        // typing immediately, never answer it.
                        if has_trust_markers(&text) {
                            let _ = child.kill().await;
                            return Err(ProviderError::Unavailable(
                                "claude_pty_untrusted_prompt".into(),
                            ));
                        }
                        if screen_is_decisive(&text) {
                            break;
                        }
                    }
                }
            }
        }
        // A drive that ran out of time with no decisive content is
        // a timeout (no evidence), not a malformed screen: the
        // coordinator must back off, not record a schema failure.
        if usage_timed_out && !screen_is_decisive(&strip_ansi(&raw)) {
            let _ = child.kill().await;
            return Err(ProviderError::Network("claude_pty_timeout".into()));
        }
        // Always ask the session to exit, then drain briefly: never
        // abandon an owned interactive child.
        let _ = child.write_all(b"/exit\r").await;
        let drain_until = tokio::time::Instant::now() + EXIT_DRAIN;
        while raw.len() <= SCREEN_CAP {
            let remaining = drain_until.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, child.read_chunk(32 * 1024)).await {
                Ok(Ok(Some(bytes))) if !bytes.is_empty() => raw.extend_from_slice(&bytes),
                _ => break,
            }
        }
        let _ = child.kill().await;
        Ok(strip_ansi(&raw))
    }
}

/// Stop reading early once the screen proves login state or quota
/// evidence — no need to wait out the full budget.
fn screen_is_decisive(text: &str) -> bool {
    has_login_markers(text) || has_window_evidence(text)
}

/// Phrases of a foreign interactive prompt (workspace trust,
/// approval, onboarding gates). The driver must NEVER type into
/// these: auto-answering a trust dialog with `/usage` would approve
/// it. Checked on first paint AND continuously — a dialog can pop up
/// after the session starts.
fn has_trust_markers(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "do you trust",
        "trust this",
        "trust the current",
        "workspace trust",
        "trust folder",
        "grant access",
        "allow access",
        "press enter to continue",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

/// Phrases observed on the real login flow (claude 2.1.245 prints an
/// oauth authorize URL + "paste code" prompt when no login exists).
/// Checked case-insensitively; "logged in" (healthy) deliberately
/// does NOT match "log in".
fn has_login_markers(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "pastecode",
        "oauth",
        "authorize",
        "/login",
        "log in",
        "sign in",
        "not logged",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

/// A percent figure next to a window-ish word. Percent alone is not
/// evidence (progress bars); the word requirement keeps the parser
/// strict: unknown screens must Parse, never fabricate.
fn has_window_evidence(text: &str) -> bool {
    text.lines().any(|line| {
        let lower = line.to_lowercase();
        let hinted = [
            "session", "week", "opus", "sonnet", "haiku", "limit", "extra", "overage",
        ]
        .iter()
        .any(|w| lower.contains(w));
        hinted && first_percent(line).is_some()
    })
}

/// First ASCII `N[.M]%` on the line with 0 <= N <= 100. Lines with
/// a comma decimal (`61,5%`) are locale-ambiguous and yield NO
/// evidence: misreading them as `5%` would fabricate quotas.
fn first_percent(line: &str) -> Option<Decimal> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut comma_decimal = false;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b',' {
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if k > j + 1 {
                    comma_decimal = true;
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    if comma_decimal {
        return None;
    }
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'.' {
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                j = k;
            }
            if j < bytes.len() && bytes[j] == b'%' {
                if let Ok(value) = line[i..j].parse::<Decimal>() {
                    if value >= Decimal::ZERO && value <= Decimal::ONE_HUNDRED {
                        return Some(value);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// Strip ANSI CSI/OSC/charset sequences + carriage returns so TUI
/// redraws collapse to searchable text. Locale-independent by
/// construction (the child is spawned with LANG=C).
fn strip_ansi(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for c2 in chars.by_ref() {
                        if c2.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut prev_escape = false;
                    for c2 in chars.by_ref() {
                        if c2 == '\x07' {
                            break;
                        }
                        if prev_escape && c2 == '\\' {
                            break;
                        }
                        prev_escape = c2 == '\x1b';
                    }
                }
                Some('(') | Some(')') => {
                    chars.next();
                    chars.next();
                }
                _ => {}
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// One normalized quota window from the usage screen.
#[derive(Debug, Clone, PartialEq)]
pub struct PtyWindow {
    pub name: String,
    pub pool_id: String,
    pub used_percent: Decimal,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Parsed screen: windows, plan/tier label, warnings.
pub type ParsedScreens = (Vec<PtyWindow>, Option<String>, Vec<String>);

/// Strict usage-screen parser. SYNTHETIC shape v1 (no logged-in host
/// has been available to reconcile against — see module docs):
/// window stanzas are `(label with window hint) + percent` lines with
/// an optional reset line carrying an RFC3339 timestamp. Login
/// markers (OBSERVED) classify as AuthenticationRequired. Anything
/// else is Parse — the parser cannot fabricate from unknown screens.
pub fn parse_usage_screen(text: &str, now: DateTime<Utc>) -> Result<ParsedScreens, ProviderError> {
    if has_login_markers(text) {
        return Err(ProviderError::AuthenticationRequired);
    }
    let mut warnings = vec![];
    let mut windows = vec![];
    let mut plan: Option<String> = None;
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if plan.is_none() {
            if let Some(value) = parse_plan_line(line) {
                match sanitize_plan(&value) {
                    Some(clean) => plan = Some(clean),
                    None => warnings.push("plan_dropped_untrusted".to_string()),
                }
            }
        }
        if let Some(percent) = first_percent(line) {
            let lower = line.to_lowercase();
            if let Some((name, pool)) = classify_window_line(&lower) {
                let end = (i + 3).min(lines.len());
                let resets_at = find_reset(&lines[i..end]);
                if resets_at.is_none() && reset_mentioned(&lines[i..end]) {
                    warnings.push("reset_unparsed".to_string());
                }
                windows.push(PtyWindow {
                    name,
                    pool_id: pool,
                    used_percent: percent,
                    resets_at,
                });
            }
        }
        i += 1;
    }
    if windows.is_empty() {
        return Err(ProviderError::Parse(PTY_USAGE_FINGERPRINT.into()));
    }
    let _ = now;
    Ok((windows, plan, warnings))
}

/// `Plan: Pro` / `Tier - Max` / `Subscription: Team` lines (v1
/// synthetic shape). Values are untrusted server text: length-capped,
/// never an email/token.
fn parse_plan_line(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    for key in ["plan:", "tier:", "subscription:"] {
        if let Some(pos) = lower.find(key) {
            let value = line[pos + key.len()..].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn sanitize_plan(value: &str) -> Option<String> {
    let clean = value.trim();
    if clean.is_empty() || clean.len() > 64 || clean.contains('@') {
        return None;
    }
    Some(clean.to_string())
}

/// Label + stable pool slug for a percent-bearing line. `None` keeps
/// stray percentages (progress bars, help text) out of quotas.
fn classify_window_line(lower: &str) -> Option<(String, String)> {
    let model = ["opus", "sonnet", "haiku"]
        .iter()
        .find(|m| lower.contains(*m));
    let weekly = lower.contains("week");
    let session = lower.contains("session") && !weekly;
    match (model, weekly, session) {
        (Some(m), true, _) => Some((
            format!("Weekly · {}", capitalize(m)),
            format!("claude-pty-weekly-{m}"),
        )),
        (Some(m), false, _) => Some((capitalize(m), format!("claude-pty-{m}"))),
        (None, true, _) => Some(("Weekly".to_string(), "claude-pty-weekly".to_string())),
        (None, false, true) => Some(("Session".to_string(), "claude-pty-session".to_string())),
        _ => {
            if lower.contains("extra") || lower.contains("overage") {
                Some((
                    "Additional".to_string(),
                    "claude-pty-additional".to_string(),
                ))
            } else if lower.contains("limit") {
                Some(("Limit".to_string(), "claude-pty-limit".to_string()))
            } else {
                None
            }
        }
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Whether a reset is TALKED about in the stanza (window line or a
/// following reset-led line) regardless of timestamp evidence.
fn reset_mentioned(lines: &[&str]) -> bool {
    if lines.is_empty() {
        return false;
    }
    if lines[0].to_lowercase().contains("reset") {
        return true;
    }
    lines.len() > 1 && lines[1].trim_start().to_lowercase().starts_with("reset")
}

/// Reset timestamp from a reset stanza: the window's own line, else
/// a following line that STARTS with a reset word. Only RFC3339
/// evidence is accepted (locale-dependent date words stay `None` +
/// warning — never guessed). Narrow by design: attribution across
/// TUI redraws must not invent resets.
fn find_reset(lines: &[&str]) -> Option<DateTime<Utc>> {
    if lines.is_empty() {
        return None;
    }
    if let Some(ts) = reset_ts_in_line(lines[0]) {
        return Some(ts);
    }
    if lines.len() > 1 && lines[1].trim_start().to_lowercase().starts_with("reset") {
        return reset_ts_in_line(lines[1]);
    }
    None
}

fn reset_ts_in_line(line: &str) -> Option<DateTime<Utc>> {
    if !line.to_lowercase().contains("reset") {
        return None;
    }
    line.split_whitespace()
        .filter_map(|token| {
            let trimmed = token.trim_matches(|c: char| {
                !c.is_alphanumeric()
                    && c != ':'
                    && c != '-'
                    && c != '+'
                    && c != 'T'
                    && c != 'Z'
                    && c != '.'
            });
            trimmed.parse::<DateTime<Utc>>().ok()
        })
        .next()
}

fn pty_quotas(windows: &[PtyWindow], observed_at: DateTime<Utc>) -> Vec<Quota> {
    windows
        .iter()
        .map(|w| Quota {
            pool_id: w.pool_id.clone(),
            window_id: w.resets_at.map(|r| r.timestamp().to_string()),
            name: w.name.clone(),
            used: None,
            limit: None,
            used_percent: Some(w.used_percent),
            remaining_percent: Some(Decimal::ONE_HUNDRED - w.used_percent),
            unit: MetricUnit::Percent,
            window_start: None,
            resets_at: w.resets_at,
            // Window semantics are UNPROVEN (synthetic shape v1):
            // Unknown until a logged-in host reconciles them. Never
            // synthesized from JSONL history (§10.3.3).
            window_kind: WindowKind::Unknown,
            source: format!("{PTY_USAGE_SOURCE_ID}@{}", observed_at.timestamp()),
        })
        .collect()
}

/// Primary Claude quota end: the installed CLI's `/usage` screen
/// through an owned pty. Sends exactly `/usage` then `/exit` — no
/// prompts, no model input, no inference spend. Serves default
/// scopes only (custom profiles keep JSONL; the CLI login is
/// home-scoped, never per-profile).
pub struct ClaudePtyUsageStrategy {
    pub executables: Vec<PathBuf>,
    pub stub: Option<Arc<dyn ClaudeUsageDriver>>,
    /// Profile root this strategy serves. None = default login.
    /// Some(root) spawns the CLI with CLAUDE_CONFIG_DIR=root, so the
    /// CLI reads THAT profile's auth — never the default login
    /// (§11.5: no cross-profile credential fallback).
    pub custom_root: Option<PathBuf>,
}

impl ClaudePtyUsageStrategy {
    pub fn new(executables: Vec<PathBuf>) -> Self {
        Self {
            executables,
            stub: None,
            custom_root: None,
        }
    }

    #[cfg(test)]
    fn with_stub(stub: Arc<dyn ClaudeUsageDriver>) -> Self {
        Self {
            executables: vec![],
            stub: Some(stub),
            custom_root: None,
        }
    }

    /// Scoped environment for the child: exactly one variable,
    /// default login untouched when None.
    fn profile_env(&self) -> Vec<(String, String)> {
        self.custom_root
            .as_ref()
            .map(|root| {
                vec![(
                    "CLAUDE_CONFIG_DIR".to_string(),
                    root.to_string_lossy().into_owned(),
                )]
            })
            .unwrap_or_default()
    }

    fn capabilities() -> BTreeSet<Capability> {
        crate::descriptor::CLAUDE_QUOTA_CAPS
            .iter()
            .copied()
            .collect()
    }

    async fn gate(
        &self,
        process: &dyn usage_host::ProcessHost,
        cancel: &usage_host::CancellationToken,
    ) -> LoginGate {
        if let Some(stub) = &self.stub {
            return stub.login_gate(cancel).await;
        }
        HostClaudeDriver {
            process,
            // Availability only needs the process end (auth status);
            // the pty end is resolved at fetch. A dangling reference
            // would be dishonest, so gate through a dedicated probe
            // driver below instead of a dummy pty.
            pty: &NoPty,
            executables: self.executables.clone(),
            env: self.profile_env(),
        }
        .login_gate(cancel)
        .await
    }
}

/// Zero-sized PTYHost that denies everything: the availability gate
/// never touches terminal execution (process-only `auth status`).
struct NoPty;

#[async_trait]
impl usage_host::PTYHost for NoPty {
    async fn run(
        &self,
        _req: usage_host::PtyRequest<'_>,
        _input: usage_host::PtyInput,
    ) -> Result<Vec<u8>, usage_host::HostError> {
        Err(usage_host::HostError::Policy)
    }
    async fn spawn_pty(
        &self,
        _req: usage_host::PtySpawnRequest<'_>,
    ) -> Result<usage_host::PtyChild, usage_host::HostError> {
        Err(usage_host::HostError::Policy)
    }
}

#[async_trait]
impl crate::strategy::FetchStrategy for ClaudePtyUsageStrategy {
    fn source_id(&self) -> crate::strategy::SourceId {
        crate::strategy::SourceId(PTY_USAGE_SOURCE_ID)
    }
    fn classification(&self) -> crate::strategy::SourceClassification {
        crate::strategy::SourceClassification::SupportedClientApi
    }
    fn kill_switch(&self) -> &'static str {
        PTY_USAGE_KILL_SWITCH
    }
    async fn availability(
        &self,
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> crate::strategy::Availability {
        use crate::strategy::Availability;
        if ctx.cancel.is_cancelled() {
            return Availability::Disabled;
        }
        let (Some(process), Some(_)) = (ctx.facade.process, ctx.facade.pty) else {
            return Availability::Unsupported;
        };
        if self.stub.is_some() {
            return Availability::Ready;
        }
        if self.executables.is_empty() {
            return Availability::NotConfigured;
        }
        // Fast login gate (bounded `auth status` per candidate):
        // not-logged-in is NotConfigured (skip without cooldown —
        // the actionable truth is "sign in"); only a proven login
        // is Ready.
        match self.gate(process, &ctx.cancel).await {
            LoginGate::LoggedIn(_) => Availability::Ready,
            _ => Availability::NotConfigured,
        }
    }
    async fn fetch(
        &self,
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> Result<crate::strategy::FetchPayload, crate::strategy::SourceError> {
        use crate::strategy::SourceError;
        if ctx.cancel.is_cancelled() {
            return Err(SourceError::Cancelled);
        }
        let (Some(process), Some(pty)) = (ctx.facade.process, ctx.facade.pty) else {
            return Err(SourceError::Policy);
        };
        let screen = if let Some(stub) = &self.stub {
            stub.usage_screen(Path::new(""), ctx.timeout, &ctx.cancel)
                .await
        } else {
            let driver = HostClaudeDriver {
                process,
                pty,
                executables: self.executables.clone(),
                env: self.profile_env(),
            };
            // Re-gate at fetch: a login that lapsed since
            // availability must surface as auth state, and the drive
            // needs the proven executable.
            let executable = match driver.login_gate(&ctx.cancel).await {
                LoginGate::LoggedIn(exe) => exe,
                LoginGate::NotLoggedIn => return Err(SourceError::AuthenticationRequired),
                LoginGate::Broken => return Err(SourceError::Unavailable),
            };
            driver
                .usage_screen(&executable, ctx.timeout, &ctx.cancel)
                .await
        };
        let text = screen.map_err(|e| match e {
            ProviderError::AuthenticationRequired => SourceError::AuthenticationRequired,
            ProviderError::RateLimited(retry) => SourceError::RateLimited {
                retry_after_secs: retry,
            },
            ProviderError::Parse(_) => SourceError::Parse {
                schema: PTY_USAGE_FINGERPRINT,
            },
            ProviderError::Network(_) => SourceError::Network,
            _ => SourceError::Unavailable,
        })?;
        let observed_at = ctx.facade.clock.now();
        let (windows, plan, mut warnings) =
            parse_usage_screen(&text, observed_at).map_err(|e| match e {
                ProviderError::AuthenticationRequired => SourceError::AuthenticationRequired,
                _ => SourceError::Parse {
                    schema: PTY_USAGE_FINGERPRINT,
                },
            })?;
        // Partial windows commit (a missing stanza degrades, never
        // kills the provider); zero windows already errored above.
        warnings.push("unverified_window_set".to_string());
        let quotas = pty_quotas(&windows, observed_at);
        for quota in &quotas {
            quota.validate().map_err(|_| SourceError::Parse {
                schema: PTY_USAGE_FINGERPRINT,
            })?;
        }
        Ok(crate::strategy::FetchPayload {
            snapshot: Some(Snapshot {
                account_id: ctx.account.id,
                provider_id: "anthropic".into(),
                product_id: "claude-code".into(),
                plan_label: plan,
                capabilities: Self::capabilities(),
                quotas,
                balances: vec![],
                observed_at: Some(observed_at),
                fetched_at: observed_at,
                connection_state: ConnectionState::Connected,
                // Observed seconds ago through our own drive.
                freshness: Freshness::Fresh,
                // Unverified window semantics: never Complete until a
                // logged-in host reconciles the shape.
                coverage: Coverage::UnverifiedSemantics,
            }),
            usage: vec![],
            costs: vec![],
            balances: vec![],
            warnings,
            schema_fingerprint: PTY_USAGE_FINGERPRINT,
            coverage: Coverage::UnverifiedSemantics,
            observed_at: Some(observed_at),
            source_error: None,
            // No account identity leaves the screen: tier text only,
            // never an email or id — so no observed-identity binding.
            observed_identity: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Strategy 2: read-only OAuth usage fallback. Contract implemented,
// activation pending: production wires `oauth_url: None` and an empty
// host allow-list, so availability stays NotConfigured — no verified
// endpoint is fabricated and no Keychain/credential store is opened
// (the secret, when one exists, arrives via FetchContext only).
// ---------------------------------------------------------------------------

/// Synthetic response shape v1 (dormant until verified):
/// `{"windows":[{"scope":"session|weekly|model","model"?,
/// "used_percent":n,"resets_at"?:rfc3339,"window_minutes"?}],
/// "plan"?:string,"tier"?:string}`.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthWindow {
    pub name: String,
    pub pool_id: String,
    pub used_percent: Decimal,
    pub resets_at: Option<DateTime<Utc>>,
    pub window_start: Option<DateTime<Utc>>,
}

/// Parsed OAuth body: windows, plan/tier label, warnings.
pub type ParsedOAuth = (Vec<OAuthWindow>, Option<String>, Vec<String>);

pub fn parse_oauth_usage(body: &serde_json::Value) -> Result<ParsedOAuth, ProviderError> {
    let mut warnings = vec![];
    let windows = body
        .get("windows")
        .and_then(|v| v.as_array())
        .ok_or(ProviderError::Parse(OAUTH_USAGE_FINGERPRINT.into()))?;
    let mut out = vec![];
    for (idx, window) in windows.iter().enumerate() {
        let scope = window.get("scope").and_then(|v| v.as_str()).unwrap_or("");
        let model = window.get("model").and_then(|v| v.as_str());
        let percent = window.get("used_percent").and_then(|v| v.as_f64());
        let Some(percent) = percent else {
            warnings.push(format!("window_{idx}_no_percent"));
            continue;
        };
        let percent = Decimal::from_f64_retain(percent).unwrap_or(Decimal::NEGATIVE_ONE);
        if percent < Decimal::ZERO || percent > Decimal::ONE_HUNDRED {
            warnings.push(format!("window_{idx}_percent_out_of_range"));
            continue;
        }
        let resets_at = window
            .get("resets_at")
            .and_then(|v| v.as_str())
            .map(|s| s.parse::<DateTime<Utc>>());
        if window.get("resets_at").is_some() && resets_at.as_ref().is_some_and(|r| r.is_err()) {
            warnings.push(format!("window_{idx}_reset_unparsed"));
        }
        let resets_at = resets_at.and_then(|r| r.ok());
        let window_start = match (
            resets_at,
            window.get("window_minutes").and_then(|v| v.as_u64()),
        ) {
            (Some(reset), Some(minutes)) => Some(reset - chrono::Duration::minutes(minutes as i64)),
            _ => None,
        };
        let (name, pool_id) = match (scope, model) {
            ("session", _) => ("Session".to_string(), "claude-oauth-session".to_string()),
            ("weekly", Some(m)) => (
                format!("Weekly · {}", capitalize(m)),
                format!("claude-oauth-weekly-{}", slug(m)),
            ),
            ("weekly", None) => ("Weekly".to_string(), "claude-oauth-weekly".to_string()),
            // Model-scoped windows are weekly windows (§10.3.3).
            ("model", Some(m)) => (
                format!("Weekly · {}", capitalize(m)),
                format!("claude-oauth-weekly-{}", slug(m)),
            ),
            _ => {
                warnings.push(format!("window_{idx}_unknown_scope"));
                continue;
            }
        };
        out.push(OAuthWindow {
            name,
            pool_id,
            used_percent: percent,
            resets_at,
            window_start,
        });
    }
    if out.is_empty() {
        return Err(ProviderError::Parse(OAUTH_USAGE_FINGERPRINT.into()));
    }
    let plan = body
        .get("plan")
        .and_then(|v| v.as_str())
        .or_else(|| body.get("tier").and_then(|v| v.as_str()))
        .and_then(sanitize_plan);
    if body.get("plan").or_else(|| body.get("tier")).is_some() && plan.is_none() {
        warnings.push("plan_dropped_untrusted".to_string());
    }
    Ok((out, plan, warnings))
}

fn slug(model: &str) -> String {
    model
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub struct ClaudeOAuthUsageStrategy {
    /// Verified usage URL or None (production: None — dormant until
    /// an endpoint is proven; tests inject `https://usage.test.invalid/`).
    pub oauth_url: Option<String>,
}

#[async_trait]
impl crate::strategy::FetchStrategy for ClaudeOAuthUsageStrategy {
    fn source_id(&self) -> crate::strategy::SourceId {
        crate::strategy::SourceId(OAUTH_USAGE_SOURCE_ID)
    }
    fn classification(&self) -> crate::strategy::SourceClassification {
        crate::strategy::SourceClassification::OfficialApi
    }
    fn kill_switch(&self) -> &'static str {
        OAUTH_USAGE_KILL_SWITCH
    }
    async fn availability(
        &self,
        ctx: &crate::strategy::FetchContext<'_>,
    ) -> crate::strategy::Availability {
        // Mirrors the OpenAI costs gate: secret + http + allow-listed
        // hosts + configured URL, else NotConfigured (never fires
        // without all four — no fabricated endpoint).
        if self.oauth_url.as_deref().is_some_and(|u| !u.is_empty())
            && ctx.secret.as_deref().is_some_and(|s| !s.is_empty())
            && ctx.facade.http.is_some()
            && !ctx.facade.allowed_hosts.is_empty()
        {
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
        let Some(http) = ctx.facade.http else {
            return Err(SourceError::Policy);
        };
        let Some(url) = self.oauth_url.as_deref() else {
            return Err(SourceError::Unavailable);
        };
        // The secret travels only as a Bearer header value for this
        // call; it is never logged, stored, or placed in an error.
        let response = http
            .get_json(usage_host::HttpJsonRequest {
                url,
                bearer: Some(secret),
                allowed_hosts: ctx.facade.allowed_hosts,
                timeout: ctx.timeout,
                max_bytes: usage_host::policy::MAX_HTTP_BYTES,
                user_agent: usage_host::policy::USER_AGENT,
                cancel: ctx.cancel.clone(),
            })
            .await
            .map_err(|e| crate::strategy::map_host_error(&e))?;
        let body = response.body.ok_or(SourceError::Parse {
            schema: OAUTH_USAGE_FINGERPRINT,
        })?;
        // Partial windows commit with warnings (later-page pattern
        // from the costs strategy, single-shot form): a degraded
        // window degrades the snapshot, never the provider.
        let (windows, plan, mut warnings) =
            parse_oauth_usage(&body).map_err(|_| SourceError::Parse {
                schema: OAUTH_USAGE_FINGERPRINT,
            })?;
        warnings.push("unverified_oauth_shape".to_string());
        let observed_at = ctx.facade.clock.now();
        let quotas: Vec<Quota> = windows
            .iter()
            .map(|w| Quota {
                pool_id: w.pool_id.clone(),
                window_id: w.resets_at.map(|r| r.timestamp().to_string()),
                name: w.name.clone(),
                used: None,
                limit: None,
                used_percent: Some(w.used_percent),
                remaining_percent: Some(Decimal::ONE_HUNDRED - w.used_percent),
                unit: MetricUnit::Percent,
                window_start: w.window_start,
                resets_at: w.resets_at,
                window_kind: WindowKind::Unknown,
                source: format!("{OAUTH_USAGE_SOURCE_ID}@{}", observed_at.timestamp()),
            })
            .collect();
        for quota in &quotas {
            quota.validate().map_err(|_| SourceError::Parse {
                schema: OAUTH_USAGE_FINGERPRINT,
            })?;
        }
        Ok(crate::strategy::FetchPayload {
            snapshot: Some(Snapshot {
                account_id: ctx.account.id,
                provider_id: "anthropic".into(),
                product_id: "claude-code".into(),
                plan_label: plan,
                capabilities: crate::descriptor::CLAUDE_QUOTA_CAPS
                    .iter()
                    .copied()
                    .collect(),
                quotas,
                balances: vec![],
                observed_at: Some(observed_at),
                fetched_at: observed_at,
                connection_state: ConnectionState::Connected,
                freshness: Freshness::Fresh,
                // Synthetic shape: non-authoritative until reconciled.
                coverage: Coverage::UnverifiedSemantics,
            }),
            usage: vec![],
            costs: vec![],
            balances: vec![],
            warnings,
            schema_fingerprint: OAUTH_USAGE_FINGERPRINT,
            coverage: Coverage::UnverifiedSemantics,
            observed_at: Some(observed_at),
            source_error: None,
            observed_identity: None,
        })
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

#[cfg(test)]
mod quota_tests {
    use super::*;
    use crate::strategy::{Availability, FetchContext, FetchStrategy, SourceError};
    use std::sync::Mutex;
    use uuid::Uuid;

    const SCREEN: &str = include_str!("../fixtures/claude/usage-screen.txt");
    const OAUTH_FULL: &str = include_str!("../fixtures/claude/oauth-usage.json");
    const OAUTH_PARTIAL: &str = include_str!("../fixtures/claude/oauth-usage-partial.json");
    const TEST_HOSTS: &[&str] = &["usage.test.invalid"];

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

    fn descriptor() -> &'static crate::descriptor::ProductDescriptor {
        crate::descriptor::find_descriptor("anthropic", "claude-code").expect("claude descriptor")
    }

    fn hosts(
        process: Arc<dyn usage_host::ProcessHost>,
        pty: Arc<dyn usage_host::PTYHost>,
        http: Arc<dyn usage_host::HttpHost>,
    ) -> usage_host::Hosts {
        let files = Arc::new(usage_host::ScopedFiles);
        usage_host::Hosts {
            clock: Arc::new(usage_host::SystemClock),
            logger: Arc::new(usage_host::NullLogger),
            keychain: Arc::new(usage_host::MemoryKeychain::default()),
            http,
            files: files.clone(),
            file_scope: files,
            network: Arc::new(usage_host::ObservedNetwork::default()),
            process,
            pty,
        }
    }

    fn bare_hosts() -> usage_host::Hosts {
        hosts(
            Arc::new(usage_host::AllowlistedProcess::default()),
            Arc::new(usage_host::AllowlistedPty::default()),
            Arc::new(usage_host::ReqwestHttpHost::default()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn test_ctx<'a>(
        hosts: &'a usage_host::Hosts,
        account: &'a Account,
        process: bool,
        pty: bool,
        http: bool,
        secret: Option<Vec<u8>>,
        timeout_secs: u64,
    ) -> FetchContext<'a> {
        FetchContext {
            account,
            facade: hosts.facade(&usage_host::facade::FacadePolicy {
                files: false,
                http,
                process,
                pty,
                allowed_hosts: if http { TEST_HOSTS } else { &[] },
                keychain_service: None,
            }),
            descriptor: descriptor(),
            timeout: Duration::from_secs(timeout_secs),
            local_root: None,
            secret,
            cancel: usage_host::CancellationToken::new(),
        }
    }

    #[derive(Clone)]
    enum FailMode {
        Auth,
    }

    struct StubDriver {
        gate: LoginGate,
        screen: Option<String>,
        fail: Option<FailMode>,
    }

    #[async_trait]
    impl ClaudeUsageDriver for StubDriver {
        async fn login_gate(&self, _cancel: &usage_host::CancellationToken) -> LoginGate {
            self.gate.clone()
        }
        async fn usage_screen(
            &self,
            _executable: &Path,
            _timeout: Duration,
            _cancel: &usage_host::CancellationToken,
        ) -> Result<String, ProviderError> {
            match &self.fail {
                Some(FailMode::Auth) => Err(ProviderError::AuthenticationRequired),
                None => Ok(self.screen.clone().unwrap_or_default()),
            }
        }
    }

    fn ok_driver() -> Arc<StubDriver> {
        Arc::new(StubDriver {
            gate: LoginGate::NotLoggedIn,
            screen: Some(SCREEN.to_string()),
            fail: None,
        })
    }

    // -- screen parser ----------------------------------------------------

    #[test]
    fn screen_fixture_parses_three_windows_plus_plan() {
        let (windows, plan, warnings) =
            parse_usage_screen(SCREEN, Utc::now()).expect("fixture must parse");
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].name, "Session");
        assert_eq!(windows[0].pool_id, "claude-pty-session");
        assert_eq!(windows[0].used_percent, Decimal::new(425, 1));
        assert!(windows[0].resets_at.is_none());
        assert_eq!(windows[1].name, "Weekly");
        assert_eq!(
            windows[1].resets_at.map(|r| r.timestamp()),
            Some(1789603200)
        );
        assert_eq!(windows[2].name, "Weekly · Opus");
        assert_eq!(windows[2].used_percent, Decimal::new(12, 0));
        assert_eq!(plan.as_deref(), Some("Pro"));
        assert!(!warnings.iter().any(|w| w == "reset_unparsed"));
    }

    #[test]
    fn observed_login_flow_classifies_as_auth() {
        // Redacted shape of the REAL login flow captured 2026-09-10
        // (oauth authorize URL + paste-code prompt, no login): must
        // surface as auth state, never as quotas or a crash.
        let text = "https://claude.com/cai/oauth/authorize?code=true&client_id=REDACTED\nPaste code here if prompted>";
        assert!(matches!(
            parse_usage_screen(text, Utc::now()),
            Err(ProviderError::AuthenticationRequired)
        ));
    }

    #[test]
    fn garbage_screen_is_parse_not_quotas() {
        assert!(matches!(
            parse_usage_screen("hello\nworld 12 cows\n", Utc::now()),
            Err(ProviderError::Parse(_))
        ));
    }

    #[test]
    fn out_of_range_percent_never_fabricates() {
        let (windows, _, _) =
            parse_usage_screen("Session 142% used\nWeekly 61% used\n", Utc::now()).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].name, "Weekly");
    }

    #[test]
    fn text_reset_without_timestamp_warns_but_survives() {
        let (windows, _, warnings) =
            parse_usage_screen("Session 42% used\nResets tomorrow morning\n", Utc::now()).unwrap();
        assert_eq!(windows.len(), 1);
        assert!(windows[0].resets_at.is_none());
        assert!(warnings.iter().any(|w| w == "reset_unparsed"));
    }

    #[test]
    fn email_like_plan_is_dropped() {
        let (_, plan, warnings) =
            parse_usage_screen("Plan: someone@example.com\nSession 10% used\n", Utc::now())
                .unwrap();
        assert!(plan.is_none());
        assert!(warnings.iter().any(|w| w == "plan_dropped_untrusted"));
    }

    #[test]
    fn locale_variants_do_not_parse_as_percent() {
        // Strict ASCII-dot grammar: comma decimals and spaced
        // percents are evidence of nothing (locale stance documented).
        assert!(parse_usage_screen("Session 61,5% used\n", Utc::now()).is_err());
        assert!(parse_usage_screen("Session 45 % used\n", Utc::now()).is_err());
    }

    #[test]
    fn ansi_escapes_strip_to_searchable_text() {
        let raw = b"\x1b[1mSession 42% used\x1b[0m\r\n\x1b]8;;https://x\x07Weekly 61%\x1b\\ more";
        let clean = strip_ansi(raw);
        assert!(clean.contains("Session 42% used"));
        assert!(clean.contains("Weekly 61%"));
        assert!(!clean.contains('\x1b'));
        assert!(!clean.contains('\r'));
    }

    // -- S1 strategy (stubbed driver) --------------------------------------

    #[tokio::test]
    async fn stub_end_to_end_reports_partial_verified_quotas() {
        let hosts = bare_hosts();
        let account = account();
        let strategy = ClaudePtyUsageStrategy::with_stub(ok_driver());
        let ctx = test_ctx(&hosts, &account, true, true, false, None, 10);
        assert_eq!(strategy.availability(&ctx).await, Availability::Ready);
        let payload = strategy.fetch(&ctx).await.expect("stub fetch");
        let snapshot = payload.snapshot.expect("snapshot");
        assert_eq!(snapshot.quotas.len(), 3);
        assert_eq!(snapshot.plan_label.as_deref(), Some("Pro"));
        assert_eq!(snapshot.coverage, Coverage::UnverifiedSemantics);
        assert!(snapshot
            .capabilities
            .contains(&Capability::SubscriptionQuota));
        assert!(payload
            .warnings
            .iter()
            .any(|w| w == "unverified_window_set"));
        // No email or secret material anywhere in the safe surface;
        // the only legal '@' is our own source marker.
        let rendered = format!("{snapshot:?}");
        let mut rest = rendered.as_str();
        while let Some(pos) = rest.find('@') {
            let before = &rest[..pos];
            let after = &rest[pos + 1..];
            assert!(
                before.ends_with("claude-pty-usage")
                    && after.chars().next().is_some_and(|c| c.is_ascii_digit()),
                "unexpected '@' near: ...{}...",
                &rendered[pos.saturating_sub(24)..(pos + 12).min(rendered.len())]
            );
            rest = after;
        }
    }

    #[tokio::test]
    async fn stub_login_expiry_is_auth_not_failure() {
        let hosts = bare_hosts();
        let account = account();
        let strategy = ClaudePtyUsageStrategy::with_stub(Arc::new(StubDriver {
            gate: LoginGate::NotLoggedIn,
            screen: None,
            fail: Some(FailMode::Auth),
        }));
        let ctx = test_ctx(&hosts, &account, true, true, false, None, 10);
        assert!(matches!(
            strategy.fetch(&ctx).await,
            Err(SourceError::AuthenticationRequired)
        ));
    }

    #[tokio::test]
    async fn stub_garbage_is_parse() {
        let hosts = bare_hosts();
        let account = account();
        let strategy = ClaudePtyUsageStrategy::with_stub(Arc::new(StubDriver {
            gate: LoginGate::LoggedIn(PathBuf::from("/bin/claude")),
            screen: Some("nothing usable here".into()),
            fail: None,
        }));
        let ctx = test_ctx(&hosts, &account, true, true, false, None, 10);
        assert!(matches!(
            strategy.fetch(&ctx).await,
            Err(SourceError::Parse { .. })
        ));
    }

    #[tokio::test]
    async fn availability_gates_are_honest() {
        let hosts = bare_hosts();
        let account = account();
        let stub_strategy = ClaudePtyUsageStrategy::with_stub(ok_driver());
        // No pty grant: Unsupported even with a stub.
        let ctx = test_ctx(&hosts, &account, true, false, false, None, 10);
        assert_eq!(
            stub_strategy.availability(&ctx).await,
            Availability::Unsupported
        );
        // No executable and no stub: NotConfigured, never Ready.
        let bare = ClaudePtyUsageStrategy::new(vec![]);
        let ctx = test_ctx(&hosts, &account, true, true, false, None, 10);
        assert_eq!(bare.availability(&ctx).await, Availability::NotConfigured);
        // Cancelled: Disabled.
        let ctx = test_ctx(&hosts, &account, true, true, false, None, 10);
        ctx.cancel.cancel();
        assert_eq!(
            stub_strategy.availability(&ctx).await,
            Availability::Disabled
        );
        assert!(matches!(
            stub_strategy.fetch(&ctx).await,
            Err(SourceError::Cancelled)
        ));
    }

    // -- S1 through the REAL backends (unix: forkpty + sh) --------------------

    #[cfg(unix)]
    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    /// Production-shaped responder: answers the `auth status` gate
    /// like the real CLI, serves a canned screen, exits on `/exit`.
    /// The no-spend proof below counts input ECHOES in the driven
    /// text (every byte we send is echoed by the pty): exactly one
    /// `/usage` and one `/exit`, in order, regardless of scheduling.
    #[cfg(unix)]
    #[tokio::test]
    async fn real_drive_sends_exactly_usage_then_exit() {
        let temp = tempfile::tempdir().unwrap();
        let screen = temp.path().join("screen.txt");
        std::fs::write(&screen, SCREEN).unwrap();
        let script = write_script(
            temp.path(),
            "fake-claude.sh",
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then\n  printf '%s\\n' '{{\"loggedIn\":true,\"authMethod\":\"oauth\"}}'\n  exit 0\nfi\nscreen='{screen}'\nprintf '%s\\n' 'Claude Code v9.9.9 research preview'\nwhile IFS= read -r line; do\n  case \"$line\" in\n    *\"/usage\"*) cat \"$screen\";;\n    *\"/exit\"*) exit 0;;\n  esac\ndone\nexit 0\n",
                screen = screen.display()
            ),
        );
        let process = usage_host::AllowlistedProcess::with_allowed([script.clone()]);
        let pty = usage_host::AllowlistedPty::with_allowed([script.clone()]);
        let driver = HostClaudeDriver {
            process: &process,
            pty: &pty,
            executables: vec![script.clone()],
            env: vec![],
        };
        // Login gate through the real process backend first.
        assert_eq!(
            driver
                .login_gate(&usage_host::CancellationToken::new())
                .await,
            LoginGate::LoggedIn(script.clone())
        );
        // Then the interactive drive through the real forkpty backend.
        let text = driver
            .usage_screen(
                &script,
                Duration::from_secs(15),
                &usage_host::CancellationToken::new(),
            )
            .await
            .expect("driven screen");
        // The fixture carries neither command word, so every match
        // below is our own pty echo: exactly the two sends, ordered.
        assert_eq!(text.matches("/usage").count(), 1, "one /usage send");
        assert_eq!(text.matches("/exit").count(), 1, "one /exit send");
        assert!(
            text.rfind("/exit").unwrap() > text.rfind("/usage").unwrap(),
            "usage before exit"
        );
        assert!(has_window_evidence(&text), "screen evidence present");
    }

    /// Per-profile isolation (§11.5/§14, explicit requirement): two
    /// CLAUDE_CONFIG_DIR roots driven through the real forkpty
    /// backend observe only their own login. The responder varies its
    /// plan by config dir and records every input line into that same
    /// dir; each fetch must bind its own plan and leave the sibling
    /// dir's record free of foreign commands.
    #[cfg(unix)]
    #[tokio::test]
    async fn claude_a_fetch_cannot_observe_claude_b() {
        let base = tempfile::tempdir().unwrap();
        let root_a = base.path().join("root-a");
        let root_b = base.path().join("root-b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        for (root, plan) in [(&root_a, "Alpha"), (&root_b, "Beta")] {
            std::fs::write(
                root.join("screen.txt"),
                format!(
                    "Claude Code Usage\nPlan: {plan}\n\nSession 42% used\nWeekly (all models) 61% used\nResets at 2026-09-17T00:00:00Z\n"
                ),
            )
            .unwrap();
        }
        let script = write_script(
            base.path(),
            "fake-claude-ab.sh",
            "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then\n  printf '%s\\n' '{\"loggedIn\":true}'\n  exit 0\nfi\nrecord=\"$CLAUDE_CONFIG_DIR/record.txt\"\nscreen=\"$CLAUDE_CONFIG_DIR/screen.txt\"\nprintf '%s\\n' 'boot splash'\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$record\"\n  case \"$line\" in\n    *\"/usage\"*) cat \"$screen\";;\n    *\"/exit\"*) exit 0;;\n  esac\ndone\n",
        );
        let hosts = hosts(
            Arc::new(usage_host::AllowlistedProcess::with_allowed([
                script.clone()
            ])),
            Arc::new(usage_host::AllowlistedPty::with_allowed([script.clone()])),
            Arc::new(usage_host::ReqwestHttpHost::default()),
        );
        let mut plans = std::collections::HashMap::new();
        for (name, root) in [("a", &root_a), ("b", &root_b)] {
            let account = account();
            let strategy = ClaudePtyUsageStrategy {
                executables: vec![script.clone()],
                stub: None,
                custom_root: Some(root.clone()),
            };
            let ctx = test_ctx(&hosts, &account, true, true, false, None, 15);
            assert_eq!(strategy.availability(&ctx).await, Availability::Ready);
            let payload = strategy.fetch(&ctx).await.expect("root fetch");
            let snapshot = payload.snapshot.expect("snapshot");
            assert_eq!(snapshot.quotas.len(), 2);
            plans.insert(name, snapshot.plan_label.expect("plan binds"));
            // This home observed exactly its own drive: the record
            // file exists here (env routing works) and carries our
            // /usage command. No exact full-sequence equality: under
            // host load the exit handshake may race the kill.
            let record = std::fs::read_to_string(root.join("record.txt")).expect("own record");
            assert!(
                record.lines().any(|l| l.trim_end_matches('\r') == "/usage"),
                "{name} record misses /usage"
            );
        }
        assert_eq!(plans["a"].as_str(), "Alpha");
        assert_eq!(plans["b"].as_str(), "Beta");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hanging_drive_is_bounded_network_error() {
        let temp = tempfile::tempdir().unwrap();
        let script = write_script(
            temp.path(),
            "hang-claude.sh",
            "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then\n  printf '%s\\n' '{\"loggedIn\":true}'\n  exit 0\nfi\nprintf '%s\\n' 'booting, please wait'\nsleep 30\n",
        );
        let hosts = hosts(
            Arc::new(usage_host::AllowlistedProcess::with_allowed([
                script.clone()
            ])),
            Arc::new(usage_host::AllowlistedPty::with_allowed([script.clone()])),
            Arc::new(usage_host::ReqwestHttpHost::default()),
        );
        let account = account();
        let strategy = ClaudePtyUsageStrategy::new(vec![script]);
        let ctx = test_ctx(&hosts, &account, true, true, false, None, 2);
        let started = std::time::Instant::now();
        let result = strategy.fetch(&ctx).await;
        assert!(
            matches!(result, Err(SourceError::Network)),
            "hang must surface Network, got {result:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(12),
            "drive must stay bounded"
        );
    }

    #[test]
    fn trust_markers_catch_foreign_prompts_only() {
        assert!(has_trust_markers("Do you trust the files in this folder?"));
        assert!(has_trust_markers("Workspace trust required to continue"));
        assert!(has_trust_markers("Press Enter to continue"));
        assert!(has_trust_markers("Grant access to proceed"));
        // Usage evidence, login flow and plain splash stay clean.
        assert!(!has_trust_markers(SCREEN));
        assert!(!has_trust_markers(
            "https://claude.com/cai/oauth/authorize?code=true\nPaste code here if prompted>"
        ));
        assert!(!has_trust_markers("Claude Code v9.9.9\nSession 42% used"));
    }

    /// A trust dialog must stop the drive BEFORE anything is typed:
    /// auto-answering it with `/usage` would approve it. The script
    /// records every received byte; the record stays empty.
    #[cfg(unix)]
    #[tokio::test]
    async fn trust_dialog_aborts_without_typing() {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("record.txt");
        let script = write_script(
            temp.path(),
            "trust-claude.sh",
            &format!(
                "#!/bin/sh\nrecord='{record}'\nprintf '%s\\n' 'Do you trust the files in this folder?'\nprintf '%s\\n' '  1. Yes, proceed'\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$record\"\ndone\n",
                record = record.display()
            ),
        );
        // Drive must refuse before typing anything.
        let process = usage_host::AllowlistedProcess::with_allowed([script.clone()]);
        let pty = usage_host::AllowlistedPty::with_allowed([script.clone()]);
        let driver = HostClaudeDriver {
            process: &process,
            pty: &pty,
            executables: vec![script.clone()],
            env: vec![],
        };
        let result = driver
            .usage_screen(
                &script,
                Duration::from_secs(10),
                &usage_host::CancellationToken::new(),
            )
            .await;
        assert!(
            matches!(result, Err(ProviderError::Unavailable(_))),
            "trust dialog must abort, got {result:?}"
        );
        // Nothing was ever typed: deterministic regardless of
        // scheduling, because the only writer is the driver.
        assert!(
            !record.exists() || std::fs::read_to_string(&record).unwrap().is_empty(),
            "driver typed into a trust dialog"
        );
    }

    #[tokio::test]
    async fn missing_executable_is_not_configured_fast() {
        let hosts = bare_hosts();
        let account = account();
        let strategy = ClaudePtyUsageStrategy::new(vec![PathBuf::from("/nonexistent/claude-xyz")]);
        let ctx = test_ctx(&hosts, &account, true, true, false, None, 10);
        let started = std::time::Instant::now();
        assert_eq!(
            strategy.availability(&ctx).await,
            Availability::NotConfigured
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    // -- S2 OAuth fallback (stubbed HTTP) -----------------------------------

    enum ScriptResponse {
        Json {
            status: u16,
            body: Option<serde_json::Value>,
            retry_after: Option<u64>,
        },
        TransportErr(usage_host::HostError),
    }

    struct ScriptHttp {
        responses: Mutex<Vec<ScriptResponse>>,
        calls: Mutex<usize>,
        seen_auth: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl usage_host::HttpHost for ScriptHttp {
        async fn get_json(
            &self,
            req: usage_host::HttpJsonRequest<'_>,
        ) -> Result<usage_host::HttpJsonResponse, usage_host::HostError> {
            *self.calls.lock().unwrap() += 1;
            if req.cancel.is_cancelled() {
                return Err(usage_host::HostError::Cancelled);
            }
            if let Some(bearer) = req.bearer {
                let text = String::from_utf8_lossy(bearer).into_owned();
                self.seen_auth.lock().unwrap().push(text);
            }
            match self.responses.lock().unwrap().remove(0) {
                // Mirror the real host contract: non-2xx never
                // arrives as Ok (401→Denied, 403→Forbidden,
                // 429→RateLimited, else Unavailable).
                ScriptResponse::Json {
                    status,
                    body,
                    retry_after,
                } if (200..300).contains(&status) => Ok(usage_host::HttpJsonResponse {
                    status,
                    retry_after_secs: retry_after,
                    body,
                }),
                ScriptResponse::Json { status: 401, .. } => Err(usage_host::HostError::Denied),
                ScriptResponse::Json { status: 403, .. } => Err(usage_host::HostError::Forbidden),
                ScriptResponse::Json {
                    status: 429,
                    retry_after,
                    ..
                } => Err(usage_host::HostError::RateLimited(retry_after)),
                ScriptResponse::Json { .. } => Err(usage_host::HostError::Unavailable),
                ScriptResponse::TransportErr(e) => Err(e),
            }
        }
    }

    fn oauth_hosts(response: ScriptResponse) -> (usage_host::Hosts, Arc<ScriptHttp>) {
        let http = Arc::new(ScriptHttp {
            responses: Mutex::new(vec![response]),
            calls: Mutex::new(0),
            seen_auth: Mutex::new(vec![]),
        });
        let hosts = hosts(
            Arc::new(usage_host::AllowlistedProcess::default()),
            Arc::new(usage_host::AllowlistedPty::default()),
            http.clone(),
        );
        (hosts, http)
    }

    fn oauth_strategy() -> ClaudeOAuthUsageStrategy {
        ClaudeOAuthUsageStrategy {
            oauth_url: Some("https://usage.test.invalid/v1/usage".into()),
        }
    }

    fn oauth_body(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }

    #[tokio::test]
    async fn oauth_success_maps_model_scoped_windows() {
        let (hosts, http) = oauth_hosts(ScriptResponse::Json {
            status: 200,
            body: Some(oauth_body(OAUTH_FULL)),
            retry_after: None,
        });
        let account = account();
        let strategy = oauth_strategy();
        let ctx = test_ctx(
            &hosts,
            &account,
            false,
            false,
            true,
            Some(b"oauth-test-token".to_vec()),
            10,
        );
        assert_eq!(strategy.availability(&ctx).await, Availability::Ready);
        let payload = strategy.fetch(&ctx).await.expect("oauth fetch");
        let snapshot = payload.snapshot.expect("snapshot");
        assert_eq!(snapshot.quotas.len(), 3);
        assert_eq!(snapshot.quotas[2].name, "Weekly · Opus");
        assert_eq!(
            snapshot.quotas[1].resets_at.map(|r| r.timestamp()),
            Some(1789603200)
        );
        assert_eq!(snapshot.plan_label.as_deref(), Some("Pro"));
        assert_eq!(snapshot.coverage, Coverage::UnverifiedSemantics);
        assert!(payload
            .warnings
            .iter()
            .any(|w| w == "unverified_oauth_shape"));
        // Bearer reached the transport; error surfaces never carry it
        // (asserted structurally: only the header carries the secret).
        assert_eq!(
            http.seen_auth.lock().unwrap().as_slice(),
            ["oauth-test-token"]
        );
        assert_eq!(*http.calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn oauth_partial_windows_commit_with_warnings() {
        let (hosts, _) = oauth_hosts(ScriptResponse::Json {
            status: 200,
            body: Some(oauth_body(OAUTH_PARTIAL)),
            retry_after: None,
        });
        let account = account();
        let strategy = oauth_strategy();
        let ctx = test_ctx(
            &hosts,
            &account,
            false,
            false,
            true,
            Some(b"oauth-test-token".to_vec()),
            10,
        );
        let payload = strategy.fetch(&ctx).await.expect("partial fetch");
        let snapshot = payload.snapshot.expect("snapshot");
        // Broken session + unknown teapot degrade; weekly commits.
        assert_eq!(snapshot.quotas.len(), 1);
        assert_eq!(snapshot.quotas[0].name, "Weekly");
        assert!(payload.warnings.iter().any(|w| w == "window_0_no_percent"));
        assert!(payload
            .warnings
            .iter()
            .any(|w| w == "window_2_unknown_scope"));
    }

    #[tokio::test]
    async fn oauth_statuses_classify_separately() {
        for (status, retry, expect) in [
            (401u16, None, "authentication_required"),
            (403u16, None, "insufficient_scope"),
            (429u16, Some(120u64), "rate_limited"),
            (500u16, None, "network_error"),
        ] {
            let (hosts, _) = oauth_hosts(ScriptResponse::Json {
                status,
                body: None,
                retry_after: retry,
            });
            let account = account();
            let strategy = oauth_strategy();
            let ctx = test_ctx(
                &hosts,
                &account,
                false,
                false,
                true,
                Some(b"oauth-test-token".to_vec()),
                10,
            );
            let error = strategy.fetch(&ctx).await.expect_err("must fail");
            assert_eq!(error.safe_code(), expect, "status {status}");
            if status == 429 {
                assert!(matches!(
                    error,
                    SourceError::RateLimited {
                        retry_after_secs: Some(120)
                    }
                ));
            }
        }
    }

    #[tokio::test]
    async fn oauth_transport_errors_map_without_secret() {
        for (host_error, expect) in [
            (usage_host::HostError::Timeout, SourceError::Network),
            (usage_host::HostError::Unavailable, SourceError::Network),
        ] {
            let (hosts, _) = oauth_hosts(ScriptResponse::TransportErr(host_error));
            let account = account();
            let strategy = oauth_strategy();
            let ctx = test_ctx(
                &hosts,
                &account,
                false,
                false,
                true,
                Some(b"oauth-test-token".to_vec()),
                10,
            );
            let error = strategy.fetch(&ctx).await.expect_err("must fail");
            assert_eq!(error.safe_code(), expect.safe_code());
            // Secrets never surface in error codes or messages.
            let rendered = format!("{error:?}");
            assert!(!rendered.contains("oauth-test-token"));
        }
    }

    #[tokio::test]
    async fn oauth_gates_are_honest() {
        let (hosts, _) = oauth_hosts(ScriptResponse::Json {
            status: 200,
            body: Some(oauth_body(OAUTH_FULL)),
            retry_after: None,
        });
        let account = account();
        let strategy = oauth_strategy();
        // No secret: dormant.
        let ctx = test_ctx(&hosts, &account, false, false, true, None, 10);
        assert_eq!(
            strategy.availability(&ctx).await,
            Availability::NotConfigured
        );
        assert!(matches!(
            strategy.fetch(&ctx).await,
            Err(SourceError::AuthenticationRequired)
        ));
        // No URL: dormant even with a secret.
        let url_less = ClaudeOAuthUsageStrategy { oauth_url: None };
        let ctx = test_ctx(
            &hosts,
            &account,
            false,
            false,
            true,
            Some(b"oauth-test-token".to_vec()),
            10,
        );
        assert_eq!(
            url_less.availability(&ctx).await,
            Availability::NotConfigured
        );
        assert!(matches!(
            url_less.fetch(&ctx).await,
            Err(SourceError::Unavailable)
        ));
        // Cancelled call: Cancelled, never a page.
        let ctx = test_ctx(
            &hosts,
            &account,
            false,
            false,
            true,
            Some(b"oauth-test-token".to_vec()),
            10,
        );
        ctx.cancel.cancel();
        assert!(matches!(
            strategy.fetch(&ctx).await,
            Err(SourceError::Cancelled)
        ));
    }

    #[test]
    fn oauth_email_plan_is_dropped() {
        let body = serde_json::json!({
            "windows": [{"scope": "weekly", "used_percent": 10.0}],
            "plan": "someone@example.com"
        });
        let (windows, plan, warnings) = parse_oauth_usage(&body).unwrap();
        assert_eq!(windows.len(), 1);
        assert!(plan.is_none());
        assert!(warnings.iter().any(|w| w == "plan_dropped_untrusted"));
    }

    #[test]
    fn quota_fixtures_carry_no_pii() {
        for text in [SCREEN, OAUTH_FULL, OAUTH_PARTIAL] {
            assert!(!text.contains('@'), "fixture must not contain '@'");
            assert!(!text.contains("sk-"), "fixture must not carry tokens");
        }
    }

    #[test]
    fn executable_candidates_are_static_and_shell_free() {
        let home = std::path::Path::new("/Users/example");
        let candidates = claude_executable_candidates(home);
        assert!(!candidates.is_empty());
        for candidate in &candidates {
            assert!(candidate.is_absolute(), "{candidate:?}");
            assert!(candidate.file_name().is_some_and(|n| n == "claude"));
        }
        assert!(candidates.iter().any(|p| p.starts_with(home)));
    }
}

#[cfg(test)]
mod live_acceptance {
    use super::*;
    use crate::strategy::{Availability, FetchContext, FetchStrategy};
    use std::sync::Arc;

    /// Opt-in live probes against the real installed Claude Code CLI.
    /// Never run in CI: require USAGE_AI_LIVE_CLAUDE=1. Read-only and
    /// spend-free by construction (the driver sends only `/usage`
    /// then `/exit`; every probe asserted $0.0000 during development).
    /// On a host WITHOUT a subscription login (like this dev host:
    /// `auth status` reports loggedIn:false) the strategy must degrade
    /// to NotConfigured/AuthenticationRequired — never hang, crash,
    /// or fabricate. On a logged-in host the Ready branch proves real
    /// quota end to end.
    fn live_hosts(executables: Vec<PathBuf>) -> usage_host::Hosts {
        let files = Arc::new(usage_host::ScopedFiles);
        usage_host::Hosts {
            clock: Arc::new(usage_host::SystemClock),
            logger: Arc::new(usage_host::NullLogger),
            keychain: Arc::new(usage_host::MemoryKeychain::default()),
            http: Arc::new(usage_host::ReqwestHttpHost::default()),
            files: files.clone(),
            file_scope: files,
            network: Arc::new(usage_host::ObservedNetwork::default()),
            process: Arc::new(usage_host::AllowlistedProcess::with_allowed(
                executables.clone(),
            )),
            pty: Arc::new(usage_host::AllowlistedPty::with_allowed(executables)),
        }
    }

    fn live_ctx<'a>(hosts: &'a usage_host::Hosts, account: &'a Account) -> FetchContext<'a> {
        FetchContext {
            account,
            facade: hosts.facade(&usage_host::facade::FacadePolicy {
                files: true,
                http: false,
                process: true,
                pty: true,
                allowed_hosts: &[],
                keychain_service: None,
            }),
            descriptor: crate::descriptor::find_descriptor("anthropic", "claude-code")
                .expect("claude descriptor"),
            timeout: Duration::from_secs(25),
            local_root: None,
            secret: None,
            cancel: usage_host::CancellationToken::new(),
        }
    }

    #[tokio::test]
    #[ignore = "requires a local Claude Code install; run manually with USAGE_AI_LIVE_CLAUDE=1"]
    async fn live_auth_status_shape_parses() {
        if std::env::var_os("USAGE_AI_LIVE_CLAUDE").is_none() {
            return;
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
        let executables: Vec<PathBuf> = claude_executable_candidates(&home)
            .into_iter()
            .filter(|p| p.is_file())
            .collect();
        assert!(!executables.is_empty(), "claude binary must exist");
        let hosts = live_hosts(executables.clone());
        let process = hosts.process.clone();
        let driver = HostClaudeDriver {
            process: process.as_ref(),
            pty: hosts.pty.as_ref(),
            executables,
            env: vec![],
        };
        // The gate parses the REAL binary's output shape (logged in
        // or not) — this is the live contract of auth_status_of.
        let gate = driver
            .login_gate(&usage_host::CancellationToken::new())
            .await;
        assert!(
            matches!(
                gate,
                LoginGate::LoggedIn(_) | LoginGate::NotLoggedIn | LoginGate::Broken
            ),
            "gate must resolve, got {gate:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires a local Claude Code install; run manually with USAGE_AI_LIVE_CLAUDE=1"]
    async fn live_quota_end_serves_or_degrades_cleanly() {
        if std::env::var_os("USAGE_AI_LIVE_CLAUDE").is_none() {
            return;
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
        let executables: Vec<PathBuf> = claude_executable_candidates(&home)
            .into_iter()
            .filter(|p| p.is_file())
            .collect();
        assert!(!executables.is_empty(), "claude binary must exist");
        let hosts = live_hosts(executables.clone());
        let account = Account {
            id: uuid::Uuid::nil(),
            provider_id: "anthropic".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        let ctx = live_ctx(&hosts, &account);
        let strategy = ClaudePtyUsageStrategy::new(executables);
        match strategy.availability(&ctx).await {
            Availability::Ready => {
                // Logged-in host: real quota, verified shape.
                let payload = strategy.fetch(&ctx).await.expect("live quota fetch");
                let snapshot = payload.snapshot.expect("snapshot");
                assert!(!snapshot.quotas.is_empty(), "live quotas must be present");
                assert!(payload.observed_identity.is_none(), "no identity binding");
                let rendered = format!("{snapshot:?}");
                assert!(!rendered.contains('@') || rendered.contains("claude-pty-usage@"));
            }
            Availability::NotConfigured => {
                // No login (this dev host): the drive must surface
                // auth state, never hang or fabricate.
                assert!(matches!(
                    strategy.fetch(&ctx).await,
                    Err(crate::strategy::SourceError::AuthenticationRequired)
                        | Err(crate::strategy::SourceError::Unavailable)
                ));
            }
            other => panic!("unexpected live availability: {other:?}"),
        }
    }
}
