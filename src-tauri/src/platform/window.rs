use tauri::{Manager, PhysicalPosition, Runtime};

pub fn toggle_panel<R: Runtime>(
    app: &tauri::AppHandle<R>,
    position: Option<PhysicalPosition<f64>>,
) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    if let Some(pos) = position {
        position_panel(&window, pos);
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// Clamp the panel into the visible frame of the status-item screen:
/// left/right edge handling and multi-monitor awareness instead of
/// blind offset math.
fn position_panel<R: Runtime>(window: &tauri::WebviewWindow<R>, pos: PhysicalPosition<f64>) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let size = window.outer_size().unwrap_or_default();
    let panel_w = size.width as f64 / scale;
    let panel_h = size.height as f64 / scale;
    let (origin_x, origin_y, frame_w, frame_h) = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            let factor = monitor.scale_factor();
            (
                position.x as f64 / factor,
                position.y as f64 / factor,
                size.width as f64 / factor,
                size.height as f64 / factor,
            )
        })
        .unwrap_or((0.0, 0.0, 4096.0, 4096.0));
    // Anchor under the status item, then clamp into the frame.
    let mut x = pos.x - panel_w + 18.0 / scale;
    let mut y = pos.y + 8.0 / scale;
    x = x.clamp(origin_x, (origin_x + frame_w - panel_w).max(origin_x));
    y = y.clamp(origin_y, (origin_y + frame_h - panel_h).max(origin_y));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}
