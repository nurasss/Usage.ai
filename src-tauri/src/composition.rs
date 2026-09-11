use std::sync::{Arc, Mutex};
use tauri::Manager;
use usage_host::ObservedNetwork;
use usage_storage::Storage;

use crate::appstate::AppState;
use crate::platform::{hosts, lifecycle, tray};

/// Application composition: storage, scoped hosts, runtime
/// coordinator and shared UI state. No provider business logic.
pub fn build_state(dir: &std::path::Path) -> Result<AppState, String> {
    let storage = Storage::open(dir.join("usage.db")).map_err(|_| "storage_open_failed")?;
    let storage = Arc::new(Mutex::new(storage));
    let network = ObservedNetwork::default();
    let hosts = hosts::macos_hosts(&network);
    let coordinator = Arc::new(usage_runtime::Coordinator::new(
        hosts.clone(),
        storage.clone(),
    ));
    Ok(AppState {
        storage,
        cached: Mutex::new(None),
        coordinator,
        hosts,
        network,
        planner: Mutex::new(usage_runtime::Planner::default()),
        import_stats: Mutex::new(std::collections::HashMap::new()),
        settings_path: dir.join("settings.json"),
    })
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .without_time()
        .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("usage-lifecycle")
                .on_event(|app_handle, event| {
                    if matches!(
                        event,
                        tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
                    ) {
                        if let Some(state) = app_handle.try_state::<AppState>() {
                            state.coordinator.shutdown();
                        }
                    }
                })
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            app.manage(build_state(&dir)?);
            lifecycle::spawn_scheduler(app.handle().clone());
            lifecycle::register_default_shortcut(app);
            tray::build_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(false) = event {
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::snapshot::get_snapshot,
            crate::commands::snapshot::refresh_all,
            crate::commands::export::export_history,
            crate::commands::export::export_diagnostics,
            crate::commands::diagnostics::get_diagnostics,
            crate::commands::diagnostics::get_import_stats,
            crate::commands::diagnostics::copy_diagnostics_text,
            crate::commands::accounts::list_accounts,
            crate::commands::accounts::add_account,
            crate::commands::accounts::update_account,
            crate::commands::accounts::test_connection,
            crate::commands::accounts::configure_openai_admin_connection,
            crate::commands::accounts::remove_connection_secret,
            crate::commands::accounts::remove_account,
            crate::commands::accounts::scan_candidates,
            crate::commands::accounts::connect_candidate,
            crate::commands::accounts::ignore_candidate,
            crate::commands::budgets::list_budgets,
            crate::commands::budgets::save_budget,
            crate::commands::budgets::delete_budget,
            crate::commands::info::get_descriptors,
            crate::commands::info::get_storage_status,
            crate::commands::info::get_app_info,
            crate::commands::info::report_online_state,
            crate::commands::settings::get_settings,
            crate::commands::settings::save_settings
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Usage.ai");
}
