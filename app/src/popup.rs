//! The quick-paste window (Ditto: `CQPasteWnd`) and the options window.

use std::time::{Duration, Instant};

use ditto_core::settings::{PopupPosition, Settings};
use ditto_platform::focus;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent,
};

use crate::state::{EVENT_HOTKEY_AGAIN, EVENT_POPUP_SHOWN, state};

pub const POPUP: &str = "popup";
pub const SETTINGS: &str = "settings";

/// Ignore focus loss this soon after showing (some window managers bounce focus).
const BLUR_GRACE: Duration = Duration::from_millis(250);

pub fn create_popup(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(w) = app.get_webview_window(POPUP) {
        return Ok(w);
    }
    let s = state(app).settings();
    WebviewWindowBuilder::new(app, POPUP, WebviewUrl::App("index.html".into()))
        .title("Ditto-RS")
        .inner_size(f64::from(s.window_width), f64::from(s.window_height))
        .min_inner_size(260.0, 200.0)
        .decorations(false)
        .resizable(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .shadow(true)
        .visible_on_all_workspaces(true)
        .build()
}

#[derive(Clone, serde::Serialize)]
struct PopupShown {
    /// Nothing was remembered to paste into (e.g. opened from the tray).
    has_target: bool,
    target_app: Option<String>,
}

/// Show the quick-paste window (global hotkey / tray). If it is already open and focused,
/// Ditto moves the selection down instead.
pub fn show(app: &AppHandle) {
    let window = match create_popup(app) {
        Ok(w) => w,
        Err(e) => {
            log::error!("cannot create the quick-paste window: {e}");
            return;
        }
    };
    let st = state(app);
    let settings = st.settings();
    let visible = window.is_visible().unwrap_or(false);
    if visible && window.is_focused().unwrap_or(false) {
        if settings.hotkey_again_moves_selection {
            let _ = window.emit(EVENT_HOTKEY_AGAIN, ());
        }
        return;
    }

    let target = focus::foreground_window().filter(|w| !w.is_own_process());
    log::debug!("showing the quick-paste window; paste target: {:?}", target.as_ref().map(|t| &t.app_name));
    let payload = PopupShown { has_target: target.is_some(), target_app: target.as_ref().and_then(|t| t.app_name.clone()) };
    if target.is_some() || !visible {
        *st.lock_previous_window() = target;
    }

    if !visible {
        position(app, &window, &settings);
    }
    *st.lock_popup_shown_at() = Some(Instant::now());
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit(EVENT_POPUP_SHOWN, payload);
}

/// Place the window per the "Popup Positioning" option, keeping it fully on screen
/// (Ditto: "Ensure Entire Window is Visible").
fn position(app: &AppHandle, window: &WebviewWindow, settings: &Settings) {
    let Ok(size) = window.outer_size() else { return };
    let (w, h) = (size.width as i32, size.height as i32);
    let cursor = app.cursor_position().ok();
    let monitor = cursor
        .and_then(|c| app.monitor_from_point(c.x, c.y).ok().flatten())
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let area = monitor.work_area();
    let (ax, ay) = (area.position.x, area.position.y);
    let (aw, ah) = (area.size.width as i32, area.size.height as i32);

    let (mut x, mut y) = match (settings.popup_position, settings.last_position, cursor) {
        (PopupPosition::AtPreviousPosition, Some((px, py)), _) => (px, py),
        (PopupPosition::CenterOfScreen, _, _) => (ax + (aw - w) / 2, ay + (ah - h) / 2),
        (_, _, Some(c)) => {
            let (cx, cy) = (c.x as i32, c.y as i32);
            // Open below-right of the pointer; flip when that would leave the screen.
            let x = if cx + w <= ax + aw { cx } else { cx - w };
            let y = if cy + h <= ay + ah { cy } else { cy - h };
            (x, y)
        }
        _ => (ax + (aw - w) / 2, ay + (ah - h) / 2),
    };
    x = x.clamp(ax, (ax + aw - w).max(ax));
    y = y.clamp(ay, (ay + ah - h).max(ay));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Hide the quick-paste window, remembering its position and size.
pub fn hide(app: &AppHandle) {
    let Some(window) = app.get_webview_window(POPUP) else { return };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let position = window.outer_position().ok().map(|p| (p.x, p.y));
    let logical = window
        .inner_size()
        .ok()
        .zip(window.scale_factor().ok())
        .map(|(s, scale)| (s.to_logical::<f64>(scale).width.round() as u32, s.to_logical::<f64>(scale).height.round() as u32));
    let _ = window.hide();
    let st = state(app);
    let current = st.settings();
    let changed = position != current.last_position
        || logical.is_some_and(|(w, h)| (w, h) != (current.window_width, current.window_height));
    if changed {
        st.update_settings(|s| {
            if position.is_some() {
                s.last_position = position;
            }
            if let Some((w, h)) = logical {
                s.window_width = w;
                s.window_height = h;
            }
        });
    }
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != POPUP {
        return;
    }
    let app = window.app_handle();
    match event {
        WindowEvent::Focused(false) => {
            let st = state(app);
            let recently_shown = st.lock_popup_shown_at().is_some_and(|t| t.elapsed() < BLUR_GRACE);
            if !recently_shown && !st.settings().keep_open_on_blur {
                hide(app);
            }
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(app);
        }
        _ => {}
    }
}

pub fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    hide(app);
    let built = WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App("settings.html".into()))
        .title("Ditto-RS Options")
        .inner_size(760.0, 620.0)
        .min_inner_size(600.0, 460.0)
        .center()
        .focused(true)
        .build();
    match built {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => log::error!("cannot open the options window: {e}"),
    }
}
