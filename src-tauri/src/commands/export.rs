use chrono::Local;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::appstate::AppState;
use crate::commands::diagnostics::diagnostics_export_payload;
use crate::commands::settings::set_user_only_permissions;

/// Exports use an explicit Save dialog (§29.4): nothing is written
/// silently to Documents/Downloads. Temp files are user-only.
async fn save_with_dialog(
    app: &AppHandle,
    suggested_name: String,
    content: String,
) -> Result<String, String> {
    let file_path = app
        .dialog()
        .file()
        .set_file_name(&suggested_name)
        .blocking_save_file();
    let Some(file_path) = file_path else {
        return Err("export_cancelled".into());
    };
    let path = file_path
        .as_path()
        .ok_or_else(|| "export_destination_unavailable".to_string())?;
    std::fs::write(path, content).map_err(|_| "export_write_failed".to_string())?;
    set_user_only_permissions(path);
    Ok(format!("Экспорт сохранён: {}", path.display()))
}

#[tauri::command]
pub async fn export_history(
    format: String,
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    if !matches!(format.as_str(), "csv" | "json") {
        return Err("unsupported_export_format".into());
    }
    let content = {
        let storage = state
            .storage
            .lock()
            .map_err(|_| "storage_lock".to_string())?;
        match format.as_str() {
            "csv" => storage.export_csv(),
            _ => storage.export_json(),
        }
        .map_err(|_| "export_failed".to_string())?
    };
    save_with_dialog(
        &app,
        format!(
            "Usage.ai-history-{}.{}",
            Local::now().format("%Y-%m-%d-%H%M%S"),
            format
        ),
        content,
    )
    .await
}

#[tauri::command]
pub async fn export_diagnostics(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    // Separate whitelist-only format, never mixed with history export.
    let content = serde_json::to_string_pretty(&diagnostics_export_payload(state.inner()))
        .map_err(|_| "export_failed".to_string())?;
    save_with_dialog(
        &app,
        format!(
            "Usage.ai-diagnostics-{}.json",
            Local::now().format("%Y-%m-%d-%H%M%S")
        ),
        content,
    )
    .await
}
