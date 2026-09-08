use chrono::Utc;
use uuid::Uuid;

use crate::appstate::AppState;
use crate::dto::{status_class, AttemptDto, DiagnosticsDto};

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

pub fn diagnostics_export_payload(state: &AppState) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "exportedAt": Utc::now().to_rfc3339(),
        "diagnostics": get_diagnostics_inner(state),
    })
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
                finished_at: a.finished_at,
            })
            .collect();
        out.push(DiagnosticsDto {
            provider: provider.provider_id.clone(),
            product: provider.product_id.clone(),
            account_alias: provider.alias.clone(),
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
            warnings: vec![],
            recent_attempts: attempts,
        });
    }
    out
}
