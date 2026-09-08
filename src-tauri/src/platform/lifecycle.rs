use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use usage_runtime::Trigger;

use crate::appstate::AppState;
use crate::commands::settings::read_settings;
use crate::platform::window::toggle_panel;
use crate::refresh_flow::run_full_refresh;

/// Fixed scheduler with coalescing: one current refresh per tick.
/// After sleep/wake there is no replay of missed ticks and therefore
/// no notification storm; the frontend additionally triggers a
/// single refresh when the panel becomes visible again.
pub fn spawn_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        while let Some(state) = app.try_state::<AppState>() {
            if !first {
                let Some(minutes) = read_settings(state.inner()).refresh_interval_minutes else {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    continue;
                };
                tokio::time::sleep(std::time::Duration::from_secs(minutes * 60)).await;
            }
            first = false;
            run_full_refresh(state.inner(), &app, Trigger::Scheduled).await;
        }
    });
}

pub fn register_default_shortcut(app: &tauri::App) {
    if let Err(error) = app
        .global_shortcut()
        .on_shortcut("Ctrl+Alt+U", |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_panel(app, None);
            }
        })
    {
        tracing::warn!(error=%error,"global_shortcut_unavailable");
    }
}
