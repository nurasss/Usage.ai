use usage_host::OnlineState;
use usage_providers::descriptor::{all_descriptors, ProductDescriptorDto};

use crate::appstate::AppState;
use crate::dto::AppInfo;

#[tauri::command]
pub fn get_descriptors() -> Vec<ProductDescriptorDto> {
    all_descriptors()
        .iter()
        .map(ProductDescriptorDto::from)
        .collect()
}

#[tauri::command]
pub fn get_storage_status(
    state: tauri::State<'_, AppState>,
) -> Result<usage_storage::StorageStatus, String> {
    state
        .storage
        .lock()
        .map_err(|_| "storage_lock".to_string())?
        .storage_status()
        .map_err(|_| "storage_status_unavailable".to_string())
}

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        update_channel: None,
        updates_enabled: false,
    }
}

/// Platform-observed connectivity feed. The runtime never hard-codes
/// `offline`; it reasons over the last observation (Unknown until the
/// first report, in which case local sources simply keep working).
#[tauri::command]
pub fn report_online_state(online: bool, state: tauri::State<'_, AppState>) {
    state.network.set(if online {
        OnlineState::Online
    } else {
        OnlineState::Offline
    });
}
