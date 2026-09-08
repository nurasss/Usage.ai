use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::appstate::AppState;
use crate::dto::AppSettings;
use crate::platform::window::toggle_panel;

pub fn read_settings(state: &AppState) -> AppSettings {
    std::fs::read_to_string(&state.settings_path)
        .ok()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> AppSettings {
    read_settings(state.inner())
}

#[tauri::command]
pub fn save_settings(
    settings: AppSettings,
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    if !matches!(
        settings.refresh_interval_minutes,
        None | Some(1 | 5 | 10 | 15 | 30 | 60)
    ) || settings.retention_days < 30
        || settings.quota_warning_percent > 100
    {
        return Err("invalid_settings".into());
    }
    app.global_shortcut()
        .unregister_all()
        .map_err(|_| "shortcut_update_failed")?;
    let shortcut = settings.global_shortcut.clone();
    if let Err(error) = app
        .global_shortcut()
        .on_shortcut(shortcut.as_str(), |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_panel(app, None);
            }
        })
    {
        let _ = app
            .global_shortcut()
            .on_shortcut("Ctrl+Alt+U", |app, _, event| {
                if event.state() == ShortcutState::Pressed {
                    toggle_panel(app, None);
                }
            });
        tracing::warn!(error=%error,"shortcut_conflict");
        return Err("shortcut_unavailable".into());
    }
    if settings.launch_at_login {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|_| "autostart_update_failed")?;
    let json = serde_json::to_vec_pretty(&settings).map_err(|_| "settings_encode_failed")?;
    let temporary = state.settings_path.with_extension("json.tmp");
    std::fs::write(&temporary, json).map_err(|_| "settings_write_failed")?;
    set_user_only_permissions(&temporary);
    std::fs::rename(&temporary, &state.settings_path).map_err(|_| "settings_commit_failed")?;
    Ok(())
}

/// Own config files carry no secrets, but user-only permissions are
/// still the expected default on this platform.
pub fn set_user_only_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}
