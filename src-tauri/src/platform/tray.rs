use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use usage_runtime::Trigger;

use crate::appstate::AppState;
use crate::commands::settings::read_settings;
use crate::dto::AppSnapshot;
use crate::platform::window::toggle_panel;
use crate::refresh_flow::{primary_remaining, run_full_refresh};

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let refresh = MenuItemBuilder::with_id("refresh", "Обновить")
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    let settings = MenuItemBuilder::with_id("settings", "Настройки…")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Выйти").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&refresh, &settings, &quit])
        .build()?;
    let mut tray = TrayIconBuilder::with_id("usage-tray")
        .tooltip("Usage.ai")
        .menu(&menu)
        .show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            position,
            ..
        } = event
        {
            toggle_panel(tray.app_handle(), Some(position));
        }
    })
    .on_menu_event(|app, event| match event.id().as_ref() {
        "refresh" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(state) = app.try_state::<AppState>() {
                    run_full_refresh(state.inner(), &app, Trigger::Tray).await;
                }
            });
        }
        "settings" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.emit("open-settings", ());
            }
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .build(app)?;
    Ok(())
}

/// Menu-bar metric (§27.7): `iconMetric` shows the minimum fresh
/// remaining percent as the tray title; `icon` clears it. A failure
/// to update the tray never fails the refresh.
pub fn update_metric(app: &AppHandle, state: &AppState, snapshot: &AppSnapshot) {
    let mode = read_settings(state).menu_bar_mode;
    let title: Option<String> = if mode == "iconMetric" {
        primary_remaining(&snapshot.providers).map(|remaining| format!("{remaining:.0}%"))
    } else {
        None
    };
    if let Some(tray) = app.tray_by_id("usage-tray") {
        let _ = tray.set_title(title.as_deref());
    }
}
