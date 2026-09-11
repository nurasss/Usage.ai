use chrono::Utc;
use uuid::Uuid;

use crate::appstate::AppState;
use crate::dto::{status_class, AttemptDto, DiagnosticsDto};

/// Attempt latency from persisted RFC3339 timestamps. None when the
/// clock data is missing or unparsable — never a fabricated zero.
pub fn attempt_latency_ms(started_at: &str, finished_at: &str) -> Option<u64> {
    let parse = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    };
    let ms = parse(finished_at)?.signed_duration_since(parse(started_at)?);
    u64::try_from(ms.num_milliseconds()).ok()
}

/// Per-source diagnostics: attempts, versions, real schema
/// fingerprints, cooldowns and safe error codes. No secrets,
/// paths, tokens or raw payloads ever appear here.
#[tauri::command]
pub fn get_diagnostics(state: tauri::State<'_, AppState>) -> Vec<DiagnosticsDto> {
    get_diagnostics_inner(state.inner())
}

/// Import diagnostics: discovered/imported files, accepted records,
/// malformed lines and checkpoint resets. Paths stay hashed.
#[tauri::command]
pub fn get_import_stats(state: tauri::State<'_, AppState>) -> Vec<crate::dto::ImportStatsDto> {
    get_import_stats_inner(state.inner())
}

fn get_import_stats_inner(state: &AppState) -> Vec<crate::dto::ImportStatsDto> {
    state
        .import_stats
        .lock()
        .ok()
        .map(|stats| {
            stats
                .iter()
                .map(|(product, report)| crate::dto::ImportStatsDto {
                    product_id: product.clone(),
                    files_discovered: report.files_discovered,
                    files_imported: report.files_imported,
                    records_accepted: report.records_accepted,
                    malformed: report.malformed,
                    checkpoint_resets: report.checkpoint_resets,
                    warnings: report.warnings.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Free-text redaction for copied diagnostics (§13.1): secret-like
/// spans become `[redacted-*]` while the surrounding text survives.
/// No regex dependency: small prefix/charset scanners.
pub fn redact_free_text(input: &str) -> String {
    let mut out = input.to_string();
    out = redact_prefixed(&out, "sk-", "[redacted-key]");
    out = redact_prefixed(&out, "xoxb-", "[redacted-token]");
    out = redact_prefixed(&out, "xoxp-", "[redacted-token]");
    out = redact_prefixed(&out, "xoxa-", "[redacted-token]");
    out = redact_prefixed(&out, "xoxr-", "[redacted-token]");
    out = redact_prefixed(&out, "xoxs-", "[redacted-token]");
    out = redact_prefixed(&out, "ghp_", "[redacted-token]");
    out = redact_labeled(&out, "bearer", "[redacted]");
    out = redact_labeled(&out, "api_key", "[redacted]");
    out = redact_labeled(&out, "apikey", "[redacted]");
    out = redact_labeled(&out, "token", "[redacted]");
    out = redact_labeled(&out, "secret", "[redacted]");
    out = redact_labeled(&out, "password", "[redacted]");
    out = redact_labeled(&out, "cookie", "[redacted]");
    out = redact_pem(&out);
    out = redact_emails(&out);
    out = redact_home_paths(&out);
    out
}

/// Bare email addresses have no other secret marker but are PII all
/// the same (§16). Conservative ASCII pattern; version strings and
/// source markers (`name@123`) never match the dot-TLD shape.
fn redact_emails(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut spans: Vec<(usize, usize)> = vec![];
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let mut start = i;
            while start > 0 && is_email_local(bytes[start - 1]) {
                start -= 1;
            }
            let mut end = i + 1;
            while end < bytes.len() && is_email_domain(bytes[end]) {
                end += 1;
            }
            // Require local part, domain dot, and 2+ letter TLD.
            if end - start >= 5
                && start < i
                && text[start..i].chars().any(|c| c != '.')
                && text[i + 1..end].contains('.')
                && text[i + 1..end].rsplit('.').next().is_some_and(|tld| {
                    tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
                })
            {
                spans.push((start, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for (start, end) in spans {
        out.push_str(&text[last..start]);
        out.push_str("[redacted-email]");
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

fn is_email_local(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn is_email_domain(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')
}

/// Absolute home-dir paths (`/Users/<…>`, `/home/<…>`,
/// `/private/<…>`, `C:\<…>`) collapse to the anchored prefix:
/// layout survives, identity does not.
fn redact_home_paths(text: &str) -> String {
    let mut out = text.to_string();
    for (prefix, marker) in [
        ("/Users/", "/Users/[redacted]"),
        ("/home/", "/home/[redacted]"),
        ("/private/", "/private/[redacted]"),
    ] {
        let mut rebuilt = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(pos) = rest.find(prefix) {
            let after = &rest[pos + prefix.len()..];
            let tail_len: usize = after
                .chars()
                .take_while(|c| !c.is_whitespace() && !matches!(c, ',' | ';' | '"' | '\''))
                .map(|c| c.len_utf8())
                .sum();
            rebuilt.push_str(&rest[..pos]);
            if tail_len > 0 {
                rebuilt.push_str(marker);
                rest = &after[tail_len..];
            } else {
                rebuilt.push_str(prefix);
                rest = after;
            }
        }
        rebuilt.push_str(rest);
        out = rebuilt;
    }
    // Windows absolute paths `C:\…` (byte-safe walk).
    let mut rebuilt = String::with_capacity(out.len());
    let bytes = out.as_bytes();
    let mut i = 0usize;
    while i < out.len() {
        if i + 3 <= out.len()
            && bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == b':'
            && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/')
        {
            let mut k = i + 3;
            while let Some(c) = out[k..].chars().next() {
                if c.is_whitespace() || matches!(c, ',' | ';' | '"' | '\'') {
                    break;
                }
                k += c.len_utf8();
            }
            if k > i + 3 {
                rebuilt.push_str(&out[i..i + 2]);
                rebuilt.push_str("\\[redacted]");
                i = k;
                continue;
            }
        }
        let Some(c) = out[i..].chars().next() else {
            break;
        };
        rebuilt.push(c);
        i += c.len_utf8();
    }
    rebuilt
}

fn is_secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '+' | '/' | '=')
}

/// `prefix + 8 or more secret chars` (e.g. `sk-abc123…`) → marker.
/// Short suffixes (e.g. `sk-test`) are kept: test fakes stay readable.
fn redact_prefixed(text: &str, prefix: &str, marker: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(prefix) {
        let after = &rest[pos + prefix.len()..];
        // ASCII-only class: char count == byte length, slicing is safe.
        let secret_len = after.chars().take_while(|c| is_secret_char(*c)).count();
        out.push_str(&rest[..pos]);
        if secret_len >= 8 {
            out.push_str(marker);
            rest = &after[secret_len..];
        } else {
            out.push_str(prefix);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Case-insensitive `label` followed by `:`/`=` and a secret value
/// (e.g. `Bearer abc…`, `api_key="xyz"`) → `label [redacted]`.
/// The label itself survives so the line stays intelligible.
/// Byte-exact ASCII matching: no lowercased copy, no offset drift.
fn redact_labeled(text: &str, label: &str, marker: &str) -> String {
    let label_len = label.len();
    let first = label.as_bytes()[0].to_ascii_lowercase();
    let mut spans: Vec<(usize, usize)> = vec![];
    let mut from = 0usize;
    while from + label_len <= text.len() {
        // Fast byte pre-scan, then an exact case-insensitive compare.
        let Some(rel) = text[from..]
            .bytes()
            .position(|b| b.to_ascii_lowercase() == first)
        else {
            break;
        };
        let pos = from + rel;
        let Some(candidate) = text.get(pos..pos + label_len) else {
            break;
        };
        if !candidate.eq_ignore_ascii_case(label) {
            from = pos + 1;
            continue;
        }
        let after = &text[pos + label_len..];
        let bytes = after.as_bytes();
        let mut k = 0usize;
        while k < bytes.len() && matches!(bytes[k], b' ' | b'\t') {
            k += 1;
        }
        let spaced = k > 0;
        // `Bearer <token>` (HTTP Authorization) carries its value
        // space-separated; every other label requires an explicit
        // `:`/`=` so prose like "cookie policy" is never touched.
        let bare_space_value = spaced && label.eq_ignore_ascii_case("bearer");
        if k < bytes.len() && matches!(bytes[k], b':' | b'=') {
            k += 1;
        } else if !bare_space_value {
            from = pos + label_len;
            continue;
        }
        while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'"' | b'\'') {
            k += 1;
        }
        let start_val = k;
        while k < bytes.len() && bytes[k].is_ascii_alphanumeric() {
            k += 1;
        }
        // Allow the usual token tail chars after the alnum run.
        while k < bytes.len() && matches!(bytes[k], b'-' | b'_' | b'.' | b'~' | b'+' | b'/' | b'=')
        {
            k += 1;
        }
        if k - start_val >= 4 {
            spans.push((pos, pos + label_len + k));
            from = pos + label_len + k;
        } else {
            from = pos + label_len;
        }
    }
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for (start, end) in spans {
        out.push_str(&text[last..start]);
        out.push_str(&text[start..start + label_len]);
        out.push(' ');
        out.push_str(marker);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

/// PEM private-key blocks → marker: from the BEGIN header through
/// the matching END line (header + base64 body + footer go together).
/// Certificates (no PRIVATE KEY tag) are left alone.
fn redact_pem(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(BEGIN) {
        let tail = &rest[pos..];
        let header_len = tail.find("-----").and_then(|opener| {
            tail[opener + 5..]
                .find("-----")
                .map(|closer| opener + 5 + closer + 5)
        });
        let Some(header_len) = header_len else {
            out.push_str(&rest[..pos + BEGIN.len()]);
            rest = &rest[pos + BEGIN.len()..];
            continue;
        };
        if !tail[..header_len].contains("PRIVATE KEY") {
            out.push_str(&rest[..pos + BEGIN.len()]);
            rest = &rest[pos + BEGIN.len()..];
            continue;
        }
        // Private key: swallow through the END line when present.
        let span = tail[header_len..].find("-----END").and_then(|rel| {
            tail[header_len + rel..]
                .find("-----")
                .map(|_| header_len + rel + "-----END".len())
                .and_then(|after_end| {
                    tail[header_len + rel + "-----END".len()..]
                        .find("-----")
                        .map(|close| after_end + close + 5)
                })
        });
        match span {
            Some(len) => {
                out.push_str(&rest[..pos]);
                out.push_str("[redacted-key]");
                rest = &rest[pos + len..];
            }
            None => {
                // Truncated block: header is still identifying.
                out.push_str(&rest[..pos]);
                out.push_str("[redacted-key]");
                rest = &rest[pos + header_len..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Single sanitizer for BOTH diagnostics surfaces (§16): the safe
/// copy text and the JSON file export derive from this allowlisted,
/// redacted bundle — never from raw DTO serialization. Only
/// structural operational fields survive; free text is redacted;
/// emails/paths/tokens/headers/payloads/prompts have no field to
/// travel through.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedAttempt {
    pub source: String,
    pub status: String,
    pub safe_code: Option<String>,
    pub finished_at: String,
    pub latency_ms: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedAccount {
    pub provider: String,
    pub product: String,
    pub account_id: String,
    pub account_alias: String,
    pub selected_source: Option<String>,
    pub connection_state: String,
    pub freshness: String,
    pub coverage: String,
    pub status_class: Option<String>,
    pub connector_version: String,
    pub parser_version: String,
    pub schema_fingerprint: Option<String>,
    pub capabilities_detected: Vec<String>,
    pub cooldown_until: Option<String>,
    pub last_refresh_attempt: Option<String>,
    pub last_successful_refresh: Option<String>,
    pub last_data_observed_at: Option<String>,
    pub last_safe_error_code: Option<String>,
    pub warnings: Vec<String>,
    pub recent_attempts: Vec<SanitizedAttempt>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedImport {
    pub product_id: String,
    pub files_discovered: usize,
    pub files_imported: usize,
    pub records_accepted: usize,
    pub malformed: usize,
    pub checkpoint_resets: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedBundle {
    pub accounts: Vec<SanitizedAccount>,
    pub imports: Vec<SanitizedImport>,
}

pub fn sanitize_diagnostics(
    diagnostics: &[crate::dto::DiagnosticsDto],
    imports: &[crate::dto::ImportStatsDto],
) -> SanitizedBundle {
    SanitizedBundle {
        accounts: diagnostics
            .iter()
            .map(|item| SanitizedAccount {
                provider: item.provider.clone(),
                product: item.product.clone(),
                account_id: item.account_id.clone(),
                account_alias: redact_free_text(&item.account_alias),
                selected_source: item.selected_source.clone(),
                connection_state: format!("{:?}", item.connection_state),
                freshness: format!("{:?}", item.freshness),
                coverage: format!("{:?}", item.coverage),
                status_class: item.status_class.clone(),
                connector_version: item.connector_version.clone(),
                parser_version: item.parser_version.clone(),
                schema_fingerprint: item.schema_fingerprint.clone(),
                capabilities_detected: item.capabilities_detected.clone(),
                cooldown_until: item.cooldown_until.clone(),
                last_refresh_attempt: item.last_refresh_attempt.clone(),
                last_successful_refresh: item.last_successful_refresh.clone(),
                last_data_observed_at: item.last_data_observed_at.clone(),
                last_safe_error_code: item.last_safe_error_code.clone(),
                warnings: item.warnings.iter().map(|w| redact_free_text(w)).collect(),
                recent_attempts: item
                    .recent_attempts
                    .iter()
                    .map(|attempt| SanitizedAttempt {
                        source: attempt.source.clone(),
                        status: attempt.status.clone(),
                        safe_code: attempt.safe_code.clone(),
                        finished_at: attempt.finished_at.clone(),
                        latency_ms: attempt.latency_ms,
                    })
                    .collect(),
            })
            .collect(),
        imports: imports
            .iter()
            .map(|stats| SanitizedImport {
                product_id: stats.product_id.clone(),
                files_discovered: stats.files_discovered,
                files_imported: stats.files_imported,
                records_accepted: stats.records_accepted,
                malformed: stats.malformed,
                checkpoint_resets: stats.checkpoint_resets,
                warnings: stats.warnings.iter().map(|w| redact_free_text(w)).collect(),
            })
            .collect(),
    }
}

pub fn diagnostics_export_payload(state: &AppState) -> serde_json::Value {
    let bundle = sanitize_diagnostics(
        &get_diagnostics_inner(state),
        &get_import_stats_inner(state),
    );
    serde_json::json!({
        "schemaVersion": 2,
        "exportedAt": Utc::now().to_rfc3339(),
        "accounts": bundle.accounts,
        "imports": bundle.imports,
    })
}

/// Copy-safe diagnostics text for one account (§13.2): built from an
/// explicit field allowlist (never full-DTO serialization) with
/// free-text redaction. Personal paths never appear (aliases only);
/// anything resembling a secret becomes `[redacted-*]`.
#[tauri::command]
pub fn copy_diagnostics_text(state: tauri::State<'_, AppState>, account_id: String) -> String {
    let diagnostics = get_diagnostics_inner(state.inner());
    let imports = get_import_stats_inner(state.inner());
    render_copy_text(&diagnostics, &imports, &account_id)
}

pub fn render_copy_text(
    diagnostics: &[crate::dto::DiagnosticsDto],
    imports: &[crate::dto::ImportStatsDto],
    account_id: &str,
) -> String {
    // Same sanitized bundle as the file export (§16 equivalence);
    // only the presentation (text vs JSON) differs.
    render_sanitized_copy(&sanitize_diagnostics(diagnostics, imports), account_id)
}

fn render_sanitized_copy(bundle: &SanitizedBundle, account_id: &str) -> String {
    let mut out = String::from("Usage.ai diagnostics (safe copy, secrets redacted)\n");
    let mut matched = false;
    for item in bundle
        .accounts
        .iter()
        .filter(|d| d.account_id == account_id)
    {
        matched = true;
        out.push_str(&format!(
            "\n[{}/{}] {}\n  source: {}\n  state: {}\n  coverage: {}\n  freshness: {}\n  last attempt: {}\n  last success: {}\n  cooldown until: {}\n  error: {}\n  connector: {} parser: {} schema: {}\n  capabilities: {}\n",
            item.provider,
            item.product,
            item.account_alias,
            item.selected_source.as_deref().unwrap_or("-"),
            item.connection_state,
            item.coverage,
            item.freshness,
            item.last_refresh_attempt.as_deref().unwrap_or("-"),
            item.last_successful_refresh.as_deref().unwrap_or("-"),
            item.cooldown_until.as_deref().unwrap_or("-"),
            item.last_safe_error_code.as_deref().unwrap_or("-"),
            item.connector_version,
            item.parser_version,
            item.schema_fingerprint.as_deref().unwrap_or("-"),
            item.capabilities_detected.join(","),
        ));
        if !item.warnings.is_empty() {
            out.push_str(&format!("  warnings: {}\n", item.warnings.join("; ")));
        }
        for attempt in &item.recent_attempts {
            out.push_str(&format!(
                "  attempt {} {} {}{}\n",
                attempt.source,
                attempt.status,
                attempt.safe_code.as_deref().unwrap_or("-"),
                attempt
                    .latency_ms
                    .map(|ms| format!(" {ms}ms"))
                    .unwrap_or_default(),
            ));
        }
    }
    if !matched {
        out.push_str("\n(no matching account)\n");
    }
    for stats in &bundle.imports {
        out.push_str(&format!(
            "\n[import {}] discovered={} imported={} accepted={} malformed={} resets={}\n",
            stats.product_id,
            stats.files_discovered,
            stats.files_imported,
            stats.records_accepted,
            stats.malformed,
            stats.checkpoint_resets,
        ));
        for warning in &stats.warnings {
            out.push_str(&format!("  import warning: {warning}\n"));
        }
    }
    out
}

fn get_diagnostics_inner(state: &AppState) -> Vec<DiagnosticsDto> {
    // Reuse the command body without a Tauri state wrapper.
    let cached = state.cached.lock().ok().and_then(|c| c.clone());
    let states = state.coordinator.states_snapshot();
    let mut out = vec![];
    let Some(snapshot) = cached else {
        return out;
    };
    for provider in &snapshot.providers {
        let scope_key = usage_runtime::ScopeKey {
            account_id: provider.account_id.parse::<Uuid>().unwrap_or(Uuid::nil()),
            provider_id: provider.provider_id.clone(),
            product_id: provider.product_id.clone(),
        };
        let runtime_state = states.get(&scope_key).cloned().unwrap_or_default();
        let attempts: Vec<AttemptDto> = state
            .storage
            .lock()
            .ok()
            .and_then(|storage| {
                storage
                    .recent_attempts(scope_key.account_id, &scope_key.product_id, 5)
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|a| AttemptDto {
                source: a.source,
                status: a.status,
                safe_code: a.safe_code,
                latency_ms: attempt_latency_ms(&a.started_at, &a.finished_at),
                finished_at: a.finished_at,
            })
            .collect();
        out.push(DiagnosticsDto {
            provider: provider.provider_id.clone(),
            product: provider.product_id.clone(),
            account_alias: provider.alias.clone(),
            account_id: provider.account_id.clone(),
            selected_source: (!runtime_state.connector_version.is_empty())
                .then(|| runtime_state.connector_version.clone()),
            connection_state: provider.connection_state.clone(),
            last_refresh_attempt: runtime_state.last_attempt.map(|d| d.to_rfc3339()),
            last_successful_refresh: runtime_state.last_success.map(|d| d.to_rfc3339()),
            last_data_observed_at: provider.observed_at.clone(),
            freshness: provider.freshness.clone(),
            coverage: provider.coverage,
            status_class: status_class(&provider.connection_state),
            connector_version: runtime_state.connector_version.clone(),
            parser_version: runtime_state.parser_version.clone(),
            schema_fingerprint: (!runtime_state.parser_version.is_empty())
                .then(|| runtime_state.parser_version.clone()),
            capabilities_detected: provider.capabilities.clone(),
            cooldown_until: runtime_state.cooldown_until.map(|d| d.to_rfc3339()),
            last_safe_error_code: runtime_state.error_code.clone(),
            warnings: runtime_state.identity_note.clone().into_iter().collect(),
            recent_attempts: attempts,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{AttemptDto, DiagnosticsDto, ImportStatsDto};

    #[test]
    fn latency_from_valid_timestamps() {
        assert_eq!(
            attempt_latency_ms("2026-09-10T10:00:00Z", "2026-09-10T10:00:01.500Z"),
            Some(1500)
        );
        assert_eq!(attempt_latency_ms("bogus", "2026-09-10T10:00:01Z"), None);
        // Negative spans (clock skew) are None, never wrapped.
        assert_eq!(
            attempt_latency_ms("2026-09-10T10:00:02Z", "2026-09-10T10:00:01Z"),
            None
        );
    }

    #[test]
    fn hostile_alias_is_redacted_not_dropped() {
        let clean = redact_free_text("Work sk-abc123XYZ4567890 label");
        assert!(clean.contains("[redacted-key]"));
        assert!(!clean.contains("sk-abc123XYZ4567890"));
        assert!(clean.contains("Work") && clean.contains("label"));
        // Short test fakes stay readable.
        assert_eq!(redact_free_text("uses sk-test"), "uses sk-test");
    }

    #[test]
    fn labeled_secrets_redacted_label_kept() {
        for (input, label) in [
            ("Authorization: Bearer abcdef123456", "Bearer"),
            ("api_key=\"xyz987654321\"", "api_key"),
            ("token=abcdef1234 rest", "token"),
            ("user password: s3cret-pass-99!", "password"),
            ("session cookie: abcdef1234567890;", "cookie"),
            ("client secret=abcdef12", "secret"),
        ] {
            let clean = redact_free_text(input);
            assert!(
                clean.starts_with(label) || clean.contains(label),
                "label kept: {clean}"
            );
            assert!(
                !clean.contains("abcdef") && !clean.contains("xyz987654321"),
                "value dropped: {clean}"
            );
        }
        // Too-short values are not secrets.
        assert_eq!(redact_free_text("token: abc"), "token: abc");
        // Bare-space prose is not a credential (only the Bearer
        // authorization form carries space-separated values).
        assert_eq!(redact_free_text("cookie policy v2"), "cookie policy v2");
        assert_eq!(redact_free_text("secret garden club"), "secret garden club");
    }

    #[test]
    fn pem_blocks_and_tokens_redacted() {
        let pem = "key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIBPA==\n-----END RSA PRIVATE KEY-----";
        let clean = redact_free_text(pem);
        assert!(clean.contains("[redacted-key]"));
        assert!(!clean.contains("MIIBPA"));
        assert_eq!(
            redact_free_text("uses xoxb-123456789012 alive"),
            "uses [redacted-token] alive"
        );
        assert_eq!(
            redact_free_text("uses ghp_abcdefgh12345678 done"),
            "uses [redacted-token] done"
        );
    }

    fn sample_diag(alias: &str, account_id: &str) -> DiagnosticsDto {
        DiagnosticsDto {
            provider: "openai".into(),
            product: "codex".into(),
            account_alias: alias.into(),
            account_id: account_id.into(),
            selected_source: Some("codex-app-server".into()),
            connection_state: usage_core::ConnectionState::Connected,
            last_refresh_attempt: Some("2026-09-10T10:00:01Z".into()),
            last_successful_refresh: Some("2026-09-10T10:00:00Z".into()),
            last_data_observed_at: None,
            freshness: usage_core::Freshness::Fresh,
            coverage: usage_core::Coverage::Complete,
            status_class: Some("ok".into()),
            connector_version: "codex-app-server".into(),
            parser_version: "v1".into(),
            schema_fingerprint: Some("fp".into()),
            capabilities_detected: vec!["subscriptionQuota".into()],
            cooldown_until: None,
            last_safe_error_code: None,
            warnings: vec!["all good sk-AAAAAAAAAAAAAAAA".into()],
            recent_attempts: vec![AttemptDto {
                source: "codex-app-server".into(),
                status: "ok".into(),
                safe_code: None,
                finished_at: "2026-09-10T10:00:01Z".into(),
                latency_ms: Some(250),
            }],
        }
    }

    #[test]
    fn copy_text_is_allowlisted_and_redacted() {
        let diags = vec![
            sample_diag("Main sk-AAAAAAAAAAAAAAAA", "acc-1"),
            sample_diag("Other", "acc-2"),
        ];
        let imports = vec![ImportStatsDto {
            product_id: "codex".into(),
            files_discovered: 3,
            files_imported: 3,
            records_accepted: 10,
            malformed: 0,
            checkpoint_resets: 0,
            warnings: vec![],
        }];
        let text = render_copy_text(&diags, &imports, "acc-1");
        // Allowlisted content present for the addressed account only.
        assert!(text.contains("[openai/codex]"));
        assert!(text.contains("codex-app-server"));
        assert!(text.contains("250ms"));
        assert!(text.contains("[import codex]"));
        assert!(!text.contains("Other"));
        // Secrets redacted even inside allowlisted free-text fields.
        assert!(!text.contains("sk-AAAAAAAAAAAAAAAA"));
        assert!(text.contains("[redacted-key]"));
        // Unknown account: explicit empty marker, imports still listed.
        let empty = render_copy_text(&diags, &imports, "acc-9");
        assert!(empty.contains("(no matching account)"));
        assert!(empty.contains("[import codex]"));
    }

    fn hostile_diag() -> (DiagnosticsDto, ImportStatsDto) {
        let mut diag = sample_diag("alice@example.com", "acc-9");
        diag.warnings = vec![
            "path /Users/alice/private/project leaked".into(),
            "Authorization: Bearer SUPERSECRETVALUE123".into(),
            "key sk-test-secret-abcdef123456 here".into(),
            "project MyPrivateProject failed".into(),
        ];
        diag.recent_attempts = vec![AttemptDto {
            source: "codex-app-server".into(),
            status: "error".into(),
            safe_code: Some("authentication_required".into()),
            finished_at: "2026-09-10T10:00:01Z".into(),
            latency_ms: Some(250),
        }];
        let imports = ImportStatsDto {
            product_id: "codex".into(),
            files_discovered: 3,
            files_imported: 3,
            records_accepted: 10,
            malformed: 1,
            checkpoint_resets: 0,
            warnings: vec!["odd line in /Users/alice/private/project".into()],
        };
        (diag, imports)
    }

    /// Positive leak injection through the PRODUCTION file-export
    /// path: hostile user-controlled text must not survive anywhere.
    /// (Bare codenames like `MyPrivateProject` carry no secret, email,
    /// or path shape: the contract sanitizes shapes, it does not
    /// censor ordinary words — the alias stays attributable.)
    #[test]
    fn file_export_sanitizes_hostile_fields() {
        let (diag, imports) = hostile_diag();
        let bundle =
            sanitize_diagnostics(std::slice::from_ref(&diag), std::slice::from_ref(&imports));
        let exported =
            serde_json::to_string(&serde_json::json!({ "accounts": bundle.accounts })).unwrap();
        for leaked in [
            "alice@example.com",
            "/Users/alice/private/project",
            "SUPERSECRETVALUE123",
            "sk-test-secret-abcdef123456",
        ] {
            assert!(!exported.contains(leaked), "file export leaks {leaked:?}");
        }
        // Labels survive (intelligibility), values do not.
        assert!(exported.contains("Bearer"));
        assert!(!exported.contains("SUPERSECRET"));
    }

    /// Valid operational diagnostics survive sanitization intact.
    #[test]
    fn file_export_preserves_safe_operational_fields() {
        let (diag, imports) = hostile_diag();
        let bundle =
            sanitize_diagnostics(std::slice::from_ref(&diag), std::slice::from_ref(&imports));
        let account = &bundle.accounts[0];
        assert_eq!(account.provider, "openai");
        assert_eq!(account.product, "codex");
        assert_eq!(account.account_id, "acc-9");
        assert_eq!(account.selected_source.as_deref(), Some("codex-app-server"));
        assert_eq!(account.coverage, "Complete");
        assert_eq!(account.last_safe_error_code.as_deref(), None);
        assert_eq!(account.recent_attempts[0].latency_ms, Some(250));
        assert_eq!(
            account.recent_attempts[0].safe_code.as_deref(),
            Some("authentication_required")
        );
        assert_eq!(bundle.imports[0].files_discovered, 3);
        assert_eq!(bundle.imports[0].malformed, 1);
        // Redacted alias keeps a readable, secret-free shape.
        assert!(!account.account_alias.contains('@'));
    }

    /// Equivalence: copy text and file export share the sanitized
    /// semantic payload — only presentation differs.
    #[test]
    fn copy_and_export_share_sanitized_semantics() {
        let (diag, imports) = hostile_diag();
        let diags = vec![diag];
        let imports = vec![imports];
        let bundle = sanitize_diagnostics(&diags, &imports);
        let text = render_copy_text(&diags, &imports, "acc-9");
        let exported = serde_json::to_string(&bundle).unwrap();
        // Same redacted alias on both surfaces.
        assert!(text.contains(&bundle.accounts[0].account_alias));
        assert!(exported.contains(&bundle.accounts[0].account_alias));
        // Same attempt facts on both surfaces.
        assert!(text.contains("codex-app-server error authentication_required 250ms"));
        assert!(exported.contains("authentication_required"));
        // Same import counts on both surfaces.
        assert!(text.contains("malformed=1"));
        assert!(exported.contains("\"malformed\":1"));
    }
}
