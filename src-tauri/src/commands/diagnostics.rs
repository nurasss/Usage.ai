use chrono::Utc;
use uuid::Uuid;

use crate::appstate::AppState;
use crate::dto::{reclassify_legacy_claude_dto, status_class, AttemptDto, DiagnosticsDto};
use usage_host::{NetworkHost, OnlineState};

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

/// Single sanitizer for BOTH diagnostics surfaces (§16, v3
/// allowlist): the safe copy text and the JSON file export derive
/// from this bundle — never from raw DTO serialization. Only
/// structural operational fields survive. There is deliberately NO
/// free-text field: account aliases, warnings, identity notes, import
/// warnings, prompts, paths, emails, tokens all have no field to
/// travel through, so no redaction pass can miss them. A field that
/// cannot prove structural safety is dropped, not regex-cleaned.
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
        "schemaVersion": 3,
        "exportedAt": Utc::now().to_rfc3339(),
        "offline": matches!(state.network.state(), OnlineState::Offline),
        "accounts": bundle.accounts,
        "imports": bundle.imports,
    })
}

/// Copy-safe diagnostics text for one account (§13.2, v3
/// allowlist): renders the same sanitized bundle as the file export.
/// No free-form text survives — aliases, warnings, identity notes,
/// prompts, paths, emails and tokens have no field to travel through.
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
    let mut out = String::from("Usage.ai diagnostics (allowlisted fields only)\n");
    let mut matched = false;
    for item in bundle
        .accounts
        .iter()
        .filter(|d| d.account_id == account_id)
    {
        matched = true;
        out.push_str(&format!(
            "\n[{}/{}]\n  account: {}\n  source: {}\n  state: {}\n  coverage: {}\n  freshness: {}\n  last attempt: {}\n  last success: {}\n  cooldown until: {}\n  error: {}\n  connector: {} parser: {} schema: {}\n  capabilities: {}\n",
            item.provider,
            item.product,
            item.account_id,
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
    }
    out
}

fn get_diagnostics_inner(state: &AppState) -> Vec<DiagnosticsDto> {
    // Reuse the command body without a Tauri state wrapper.
    let cached = state.cached.lock().ok().and_then(|c| c.clone());
    let states = state.coordinator.states_snapshot();
    let mut out = vec![];
    let Some(mut snapshot) = cached else {
        return out;
    };
    // Diagnostics can be requested before/without the snapshot IPC command;
    // apply the same legacy trust guard to the direct cache read.
    for provider in &mut snapshot.providers {
        reclassify_legacy_claude_dto(provider);
    }
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

    /// Sanctioned malicious-input fixture (§20): the exact six
    /// required injection strings plus extras, loaded from the
    /// allowlisted fixture file (never inlined, so repo hygiene and
    /// production proof stay separate concerns).
    const INJECTION_FIXTURE: &str = include_str!("../../fixtures/diagnostics-pii-injection.json");

    fn injection_values() -> Vec<String> {
        let fixture: serde_json::Value = serde_json::from_str(INJECTION_FIXTURE).unwrap();
        let object = fixture.as_object().unwrap();
        object
            .iter()
            .filter(|(key, _)| !key.starts_with('_'))
            .map(|(_, value)| value.as_str().unwrap().to_string())
            .collect()
    }

    /// Production-output tripwire (§25): the serialized export must
    /// contain none of the forbidden injected values and all required
    /// structural markers. Returns the violations (empty = clean), so
    /// CI fails closed on any leak.
    fn tripwire_check(
        serialized: &str,
        forbidden: &[String],
        required: &[&str],
    ) -> Result<(), Vec<String>> {
        let mut violations = vec![];
        for value in forbidden {
            if serialized.contains(value) {
                violations.push(format!("leaked forbidden value: {value:?}"));
            }
        }
        for marker in required {
            if !serialized.contains(marker) {
                violations.push(format!("missing required marker: {marker:?}"));
            }
        }
        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }

    fn hostile_diag() -> (DiagnosticsDto, ImportStatsDto) {
        let fixture: serde_json::Value = serde_json::from_str(INJECTION_FIXTURE).unwrap();
        let get = |key: &str| {
            fixture
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap()
                .to_string()
        };
        let mut diag = sample_diag(&get("alias"), "acc-9");
        // Identity notes travel inside warnings in the DTO (mismatch
        // notes); both must vanish from export surfaces.
        diag.warnings = vec![
            get("identity_note"),
            get("warning_path"),
            get("warning_bearer"),
            get("warning_key"),
            get("warning_project"),
            get("warning_workspace"),
            get("warning_client_secret"),
            get("warning_account"),
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
            warnings: vec![get("warning_path")],
        };
        (diag, imports)
    }

    /// §21: the REAL production export bytes for hostile input carry
    /// none of the injected strings. Asserts on serialized output,
    /// never on an intermediate object.
    #[test]
    fn pii_tripwire_production_export_bytes_are_clean() {
        let (diag, imports) = hostile_diag();
        let bundle =
            sanitize_diagnostics(std::slice::from_ref(&diag), std::slice::from_ref(&imports));
        let exported = serde_json::to_string(&serde_json::json!({
            "schemaVersion": 3,
            "accounts": bundle.accounts,
            "imports": bundle.imports,
        }))
        .unwrap();
        let forbidden = injection_values();
        assert!(
            forbidden.len() >= 6,
            "fixture must carry the six required injections"
        );
        tripwire_check(
            &exported,
            &forbidden,
            &[
                "\"schemaVersion\":3",
                "\"provider\":\"openai\"",
                "\"product\":\"codex\"",
                "\"accountId\":\"acc-9\"",
                "\"selectedSource\":\"codex-app-server\"",
                "\"coverage\":\"Complete\"",
                "\"latencyMs\":250",
                "\"authentication_required\"",
                "\"filesDiscovered\":3",
                "\"malformed\":1",
            ],
        )
        .expect("production export must be clean and structural");
    }

    /// §26 negative control: the tripwire MUST reject an intentionally
    /// unsafe payload (simulating a raw-DTO leak). This proves the
    /// check can actually fail — a passing suite alone proves nothing.
    #[test]
    fn pii_tripwire_negative_control_rejects_unsafe_payload() {
        let forbidden = injection_values();
        let unsafe_payload = serde_json::json!({
            "schemaVersion": 3,
            "accounts": [{
                "provider": "openai",
                "accountId": "acc-9",
                "accountAlias": forbidden[0],
                "warnings": [forbidden[4]],
            }],
        });
        let serialized = serde_json::to_string(&unsafe_payload).unwrap();
        let verdict = tripwire_check(&serialized, &forbidden, &["\"schemaVersion\":3"]);
        assert!(verdict.is_err(), "tripwire must reject the unsafe payload");
        let violations = verdict.expect_err("must fail");
        assert!(
            violations.iter().any(|v| v.contains(&forbidden[0])),
            "rejection names the leak: {violations:?}"
        );
        assert!(
            violations.iter().any(|v| v.contains(&forbidden[4])),
            "rejection names the leak: {violations:?}"
        );
    }

    /// §22: sanitization preserves useful safe diagnostics (privacy
    /// must not turn the export into `{}`).
    #[test]
    fn pii_tripwire_safe_fields_preserved() {
        let (diag, imports) = hostile_diag();
        let bundle =
            sanitize_diagnostics(std::slice::from_ref(&diag), std::slice::from_ref(&imports));
        let account = &bundle.accounts[0];
        assert_eq!(account.provider, "openai");
        assert_eq!(account.product, "codex");
        assert_eq!(account.account_id, "acc-9");
        assert_eq!(account.selected_source.as_deref(), Some("codex-app-server"));
        assert_eq!(account.coverage, "Complete");
        assert_eq!(account.connection_state, "Connected");
        assert_eq!(account.freshness, "Fresh");
        assert_eq!(account.connector_version, "codex-app-server");
        assert_eq!(account.parser_version, "v1");
        assert_eq!(account.schema_fingerprint.as_deref(), Some("fp"));
        assert_eq!(
            account.capabilities_detected,
            vec!["subscriptionQuota".to_string()]
        );
        assert_eq!(account.last_safe_error_code, None);
        assert_eq!(account.recent_attempts[0].latency_ms, Some(250));
        assert_eq!(
            account.recent_attempts[0].safe_code.as_deref(),
            Some("authentication_required")
        );
        assert_eq!(account.recent_attempts[0].status, "error");
        assert_eq!(bundle.imports[0].files_discovered, 3);
        assert_eq!(bundle.imports[0].records_accepted, 10);
        assert_eq!(bundle.imports[0].malformed, 1);
    }

    /// §23: copy text and file export share the sanitized semantic
    /// payload — only presentation differs. No alias, no warnings, no
    /// identity notes on either surface.
    #[test]
    fn pii_tripwire_copy_export_equivalence() {
        let (diag, imports) = hostile_diag();
        let diags = vec![diag];
        let imports = vec![imports];
        let bundle = sanitize_diagnostics(&diags, &imports);
        let text = render_copy_text(&diags, &imports, "acc-9");
        let exported = serde_json::to_string(&bundle).unwrap();
        let forbidden = injection_values();
        for value in &forbidden {
            assert!(!text.contains(value), "copy leaks {value:?}");
            assert!(!exported.contains(value), "export leaks {value:?}");
        }
        // Same structural facts on both surfaces.
        assert!(text.contains("[openai/codex]"));
        assert!(text.contains("account: acc-9"));
        assert!(text.contains("codex-app-server error authentication_required 250ms"));
        assert!(exported.contains("\"accountId\":\"acc-9\""));
        assert!(exported.contains("\"selectedSource\":\"codex-app-server\""));
        assert!(text.contains("malformed=1"));
        assert!(exported.contains("\"malformed\":1"));
        // Free-form fields are gone from both (not redacted: absent).
        assert!(!text.contains("warnings"));
        assert!(!exported.contains("\"warnings\""));
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
    fn copy_text_is_allowlisted_no_free_text() {
        let diags = vec![sample_diag("Main", "acc-1"), sample_diag("Other", "acc-2")];
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
        assert!(text.contains("account: acc-1"));
        assert!(text.contains("codex-app-server"));
        assert!(text.contains("250ms"));
        assert!(text.contains("[import codex]"));
        assert!(!text.contains("Other"));
        assert!(!text.contains("acc-2"));
        // No free-form text survives on either surface: aliases and
        // warnings are dropped, not redacted.
        assert!(!text.contains("Main"));
        assert!(!text.contains("warnings"));
        // Unknown account: explicit empty marker, imports still listed.
        let empty = render_copy_text(&diags, &imports, "acc-9");
        assert!(empty.contains("(no matching account)"));
        assert!(empty.contains("[import codex]"));
    }
}
