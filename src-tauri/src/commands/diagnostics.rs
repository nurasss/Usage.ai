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
    out
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
        while k < bytes.len() && matches!(bytes[k], b'-' | b'_' | b'.' | b'~' | b'+' | b'/' | b'=') {
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

pub fn diagnostics_export_payload(state: &AppState) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "exportedAt": Utc::now().to_rfc3339(),
        "diagnostics": get_diagnostics_inner(state),
    })
}

/// Copy-safe diagnostics text for one account (§13.2): built from an
/// explicit field allowlist (never full-DTO serialization) with
/// free-text redaction. Personal paths never appear (aliases only);
/// anything resembling a secret becomes `[redacted-*]`.
#[tauri::command]
pub fn copy_diagnostics_text(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> String {
    let diagnostics = get_diagnostics_inner(state.inner());
    let imports = get_import_stats_inner(state.inner());
    render_copy_text(&diagnostics, &imports, &account_id)
}

pub fn render_copy_text(
    diagnostics: &[crate::dto::DiagnosticsDto],
    imports: &[crate::dto::ImportStatsDto],
    account_id: &str,
) -> String {
    let mut out = String::from("Usage.ai diagnostics (safe copy, secrets redacted)\n");
    let mut matched = false;
    for item in diagnostics.iter().filter(|d| d.account_id == account_id) {
        matched = true;
        out.push_str(&format!(
            "\n[{}/{}] {}\n  source: {}\n  state: {:?}\n  coverage: {:?}\n  freshness: {:?}\n  last attempt: {}\n  last success: {}\n  cooldown until: {}\n  error: {}\n  connector: {} parser: {} schema: {}\n  capabilities: {}\n",
            item.provider,
            item.product,
            redact_free_text(&item.account_alias),
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
            let warnings: Vec<_> = item.warnings.iter().map(|w| redact_free_text(w)).collect();
            out.push_str(&format!("  warnings: {}\n", warnings.join("; ")));
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
    for stats in imports {
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
            out.push_str(&format!("  import warning: {}\n", redact_free_text(warning)));
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
            attempt_latency_ms(
                "2026-09-10T10:00:00Z",
                "2026-09-10T10:00:01.500Z"
            ),
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
}
