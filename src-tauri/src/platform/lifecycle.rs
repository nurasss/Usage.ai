use chrono::Utc;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use usage_core::refresh_policy;
use usage_host::NetworkHost;
use usage_runtime::Trigger;

use crate::appstate::AppState;
use crate::commands::settings::read_settings;
use crate::platform::window::toggle_panel;
use crate::refresh_flow::run_full_refresh;

/// Adaptive scheduler (§14.2): fixed ticks are gone. Each loop picks
/// a tick from policy — fast while any scope is active, configured
/// otherwise, stretched offline — and refreshes only due scopes
/// (idle ones serve silent LKG fills, never failures). A tick gap
/// past WAKE_GAP_MULTIPLE means the machine slept: one catch-up
/// refresh, never a replay of missed ticks (no notification storm);
/// the frontend additionally triggers a single refresh when the
/// panel becomes visible again. Manual mode (no interval) sleeps.
pub fn spawn_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        let mut last_tick = Utc::now();
        while let Some(state) = app.try_state::<AppState>() {
            let settings = read_settings(state.inner());
            let Some(minutes) = settings.refresh_interval_minutes else {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            };
            let configured = std::time::Duration::from_secs(minutes * 60);
            if !first {
                let now = Utc::now();
                if refresh_policy::wake_gap_exceeded(last_tick, configured, now) {
                    last_tick = now;
                    run_full_refresh(state.inner(), &app, Trigger::Wake).await;
                    continue;
                }
                let tick = tick_interval(state.inner(), configured);
                tokio::time::sleep(tick).await;
            }
            first = false;
            last_tick = Utc::now();
            run_full_refresh(state.inner(), &app, Trigger::Scheduled).await;
        }
    });
}

/// Adaptive tick from policy constants: active scopes pull the
/// loop fast; offline stretches everything; otherwise the configured
/// interval rules.
fn tick_interval(state: &AppState, configured: std::time::Duration) -> std::time::Duration {
    use usage_host::OnlineState;
    if state.network.state() == OnlineState::Offline {
        return configured * refresh_policy::OFFLINE_FACTOR as u32;
    }
    let now = Utc::now();
    let active = state
        .coordinator
        .states_snapshot()
        .values()
        .any(|scope_state| {
            scope_state.last_observed.is_some_and(|observed| {
                now.signed_duration_since(observed)
                    .to_std()
                    .is_ok_and(|age| age <= refresh_policy::ACTIVITY_WINDOW)
            })
        });
    if active {
        configured.min(refresh_policy::ACTIVE_CADENCE)
    } else {
        configured
    }
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
