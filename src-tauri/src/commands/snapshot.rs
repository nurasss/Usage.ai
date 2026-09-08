use tauri::AppHandle;
use usage_runtime::Trigger;

use crate::appstate::AppState;
use crate::dto::AppSnapshot;
use crate::refresh_flow::{cache_snapshot, run_full_refresh};

#[tauri::command]
pub async fn get_snapshot(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<AppSnapshot, String> {
    // Served errors are explicit: a packaged backend failure never
    // falls back to synthetic demo data (P0-3).
    if let Some(value) = state.cached.lock().map_err(|_| "state_lock")?.clone() {
        return Ok(value);
    }
    if let Ok(storage) = state.storage.lock() {
        if let Ok(Some(json)) = storage.cache("app_snapshot") {
            if let Ok(value) = serde_json::from_str::<AppSnapshot>(&json) {
                *state.cached.lock().map_err(|_| "state_lock")? = Some(value.clone());
                return Ok(value);
            }
        }
    }
    let value = run_full_refresh(state.inner(), &app, Trigger::Scheduled).await;
    if value.providers.is_empty() {
        return Err("refresh_failed".into());
    }
    Ok(value)
}

#[tauri::command]
pub async fn refresh_all(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<AppSnapshot, String> {
    // Manual refresh never bypasses an active per-account cooldown:
    // the coordinator serves stale last-known-good instead.
    let value = run_full_refresh(state.inner(), &app, Trigger::Manual).await;
    if value.providers.is_empty() {
        return Err("refresh_failed".into());
    }
    Ok(value)
}

pub fn cache_snapshot_state(state: &AppState, value: &AppSnapshot) {
    cache_snapshot(state, value);
}
