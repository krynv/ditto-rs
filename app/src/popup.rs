//! The quick-paste window (Ditto: `CQPasteWnd`) and the options window.

use std::time::{Duration, Instant};

use ditto_core::settings::{PopupPosition, Settings};
use ditto_platform::focus;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, Monitor, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    Window, WindowEvent,
};

use crate::state::{EVENT_HOTKEY_AGAIN, EVENT_POPUP_SHOWN, state};

pub const POPUP: &str = "popup";
pub const SETTINGS: &str = "settings";

/// Ignore focus loss this soon after showing (some window managers bounce focus).
const BLUR_GRACE: Duration = Duration::from_millis(250);
/// Opening a native context menu on Linux/X11 takes a keyboard grab, which GTK reports to the
/// window as losing focus. Ignore focus loss this soon after a menu opens so the window doesn't
/// hide underneath its own menu. (macOS and Windows keep the window focused while a menu is open.)
const MENU_BLUR_GRACE: Duration = Duration::from_millis(1000);

/// Window sizes at 100 % "Interface size", in logical pixels (width, height).
const POPUP_MIN: (f64, f64) = (260.0, 200.0);
const SETTINGS_SIZE: (f64, f64) = (760.0, 620.0);
const SETTINGS_MIN: (f64, f64) = (600.0, 460.0);

/// How much both windows are zoomed ("Interface size"); 1.0 where the web view cannot zoom.
pub fn zoom_factor(settings: &Settings) -> f64 {
    if ditto_platform::webview_zoom_supported() { f64::from(settings.ui_zoom) / 100.0 } else { 1.0 }
}

/// Zoom a window's page: text, pictures, icons and spacing together, like zooming in a browser.
fn apply_zoom(window: &WebviewWindow, zoom: f64) {
    if !ditto_platform::webview_zoom_supported() {
        return;
    }
    if let Err(e) = window.set_zoom(zoom) {
        log::warn!("could not set the interface size of the {} window: {e}", window.label());
    }
}

/// The size of a window at `zoom`: its size at 100 % (`base`) scaled, at least `min` scaled,
/// and no larger than the screen. Returns (size, minimum size) in logical pixels.
///
/// The quick-paste window remembers its size at 100 %, not its size on screen, so that making
/// the interface larger and smaller again brings back exactly the window there was, even when
/// the screen was too small for it in between.
fn scaled_size(base: (f64, f64), min: (f64, f64), zoom: f64, screen: Option<(f64, f64)>) -> ((f64, f64), (f64, f64)) {
    let fit = |(w, h): (f64, f64)| match screen {
        Some((sw, sh)) => (w.min(sw), h.min(sh)),
        None => (w, h),
    };
    let min = fit((min.0 * zoom, min.1 * zoom));
    let (w, h) = fit((base.0 * zoom, base.1 * zoom));
    ((w.max(min.0).round(), h.max(min.1).round()), min)
}

/// The 100 % size to remember for a window that is now `actual` at `zoom` and was laid out
/// as `expected`: the same as before, unless the user has resized the window.
fn base_after(actual: (f64, f64), expected: (f64, f64), base: (u32, u32), zoom: f64) -> (u32, u32) {
    let resized = (actual.0 - expected.0).abs() > 1.5 || (actual.1 - expected.1).abs() > 1.5;
    if resized { ((actual.0 / zoom).round() as u32, (actual.1 / zoom).round() as u32) } else { base }
}

fn popup_base(settings: &Settings) -> (f64, f64) {
    (f64::from(settings.window_width), f64::from(settings.window_height))
}

/// A monitor's usable area in logical pixels.
fn screen_of(monitor: &Monitor) -> (f64, f64) {
    let size = monitor.work_area().size.to_logical::<f64>(monitor.scale_factor());
    (size.width, size.height)
}

fn logical_inner_size(window: &WebviewWindow) -> Option<(f64, f64)> {
    let size = window.inner_size().ok()?.to_logical::<f64>(window.scale_factor().ok()?);
    Some((size.width, size.height))
}

pub fn create_popup(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(w) = app.get_webview_window(POPUP) {
        return Ok(w);
    }
    let s = state(app).settings();
    let zoom = zoom_factor(&s);
    // Sized again for the screen it opens on, each time it is shown (`position`).
    let screen = app.primary_monitor().ok().flatten().as_ref().map(screen_of);
    let (size, min) = scaled_size(popup_base(&s), POPUP_MIN, zoom, screen);
    let window = WebviewWindowBuilder::new(app, POPUP, WebviewUrl::App("index.html".into()))
        .title("Ditto-RS")
        .inner_size(size.0, size.1)
        .min_inner_size(min.0, min.1)
        .decorations(false)
        .resizable(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .shadow(true)
        .visible_on_all_workspaces(true)
        .build()?;
    apply_zoom(&window, zoom);
    Ok(window)
}

/// Give the quick-paste window its size for the saved 100 % size, the interface size and
/// the screen it is on (or about to open on). Returns that size in logical pixels.
fn size_popup(window: &WebviewWindow, settings: &Settings, monitor: Option<&Monitor>) -> (f64, f64) {
    let (size, min) = scaled_size(popup_base(settings), POPUP_MIN, zoom_factor(settings), monitor.map(screen_of));
    let _ = window.set_min_size(Some(LogicalSize::new(min.0, min.1)));
    let already = logical_inner_size(window).is_some_and(|(w, h)| (w - size.0).abs() <= 0.5 && (h - size.1).abs() <= 0.5);
    if !already {
        let _ = window.set_size(LogicalSize::new(size.0, size.1));
    }
    size
}

/// Move a window that is `size` (logical, outer) back inside its screen if it sticks out.
/// The size is passed in, not read back, because some window systems resize asynchronously.
fn keep_on_screen(window: &WebviewWindow, monitor: &Monitor, size: (f64, f64)) {
    let Ok(position) = window.outer_position() else { return };
    let area = monitor.work_area();
    let (ax, ay) = (area.position.x, area.position.y);
    let (aw, ah) = (area.size.width as i32, area.size.height as i32);
    let scale = monitor.scale_factor();
    let (w, h) = ((size.0 * scale).round() as i32, (size.1 * scale).round() as i32);
    let x = position.x.clamp(ax, (ax + aw - w).max(ax));
    let y = position.y.clamp(ay, (ay + ah - h).max(ay));
    if (x, y) != (position.x, position.y) {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

/// "Interface size" changed from `old_percent`: zoom both windows and resize them to match,
/// so a larger interface shows as much as before instead of less.
pub fn zoom_changed(app: &AppHandle, old_percent: u32) {
    if !ditto_platform::webview_zoom_supported() {
        return;
    }
    let st = state(app);
    let mut settings = st.settings();
    let zoom = zoom_factor(&settings);
    let old_zoom = f64::from(old_percent.max(1)) / 100.0;

    if let Some(window) = app.get_webview_window(POPUP) {
        let monitor = window.current_monitor().ok().flatten();
        // If the window is open and was resized by hand, that has not been saved yet.
        if window.is_visible().unwrap_or(false) {
            if let Some(actual) = logical_inner_size(&window) {
                let base = (settings.window_width, settings.window_height);
                let (expected, _) = scaled_size(popup_base(&settings), POPUP_MIN, old_zoom, monitor.as_ref().map(screen_of));
                let (w, h) = base_after(actual, expected, base, old_zoom);
                if (w, h) != base {
                    st.update_settings(|s| {
                        s.window_width = w;
                        s.window_height = h;
                    });
                    settings = st.settings();
                }
            }
        }
        apply_zoom(&window, zoom);
        let size = size_popup(&window, &settings, monitor.as_ref());
        log::debug!("interface size {old_percent}% → {}%: quick-paste window is {} × {}", settings.ui_zoom, size.0, size.1);
        if let Some(monitor) = &monitor {
            keep_on_screen(&window, monitor, size);
        }
    }

    // The options window has no remembered size: scale the one it has.
    if let Some(window) = app.get_webview_window(SETTINGS) {
        apply_zoom(&window, zoom);
        let monitor = window.current_monitor().ok().flatten();
        if let (Some(inner), Ok(outer), Ok(scale)) = (logical_inner_size(&window), window.outer_size(), window.scale_factor()) {
            let at_100 = (inner.0 / old_zoom, inner.1 / old_zoom);
            let (size, min) = scaled_size(at_100, SETTINGS_MIN, zoom, monitor.as_ref().map(screen_of));
            let _ = window.set_min_size(Some(LogicalSize::new(min.0, min.1)));
            let _ = window.set_size(LogicalSize::new(size.0, size.1));
            if let Some(monitor) = &monitor {
                // Outer size = inner size plus the title bar and borders, which do not zoom.
                let outer = outer.to_logical::<f64>(scale);
                keep_on_screen(&window, monitor, (size.0 + outer.width - inner.0, size.1 + outer.height - inner.1));
            }
        }
    }
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
    let cursor = app.cursor_position().ok();
    let monitor = cursor
        .and_then(|c| app.monitor_from_point(c.x, c.y).ok().flatten())
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    // The window has no frame, so this is also its outer size.
    let size = size_popup(window, settings, Some(&monitor));
    let scale = monitor.scale_factor();
    let (w, h) = ((size.0 * scale).round() as i32, (size.1 * scale).round() as i32);
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

/// Hide the quick-paste window, remembering its position and, if it was resized, its size.
pub fn hide(app: &AppHandle) {
    let Some(window) = app.get_webview_window(POPUP) else { return };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let st = state(app);
    let current = st.settings();
    let zoom = zoom_factor(&current);
    let position = window.outer_position().ok().map(|p| (p.x, p.y));
    let actual = logical_inner_size(&window);
    let screen = window.current_monitor().ok().flatten().as_ref().map(screen_of);
    let _ = window.hide();

    let base = (current.window_width, current.window_height);
    let (expected, _) = scaled_size(popup_base(&current), POPUP_MIN, zoom, screen);
    let (w, h) = actual.map_or(base, |actual| base_after(actual, expected, base, zoom));
    if position != current.last_position || (w, h) != base {
        st.update_settings(|s| {
            if position.is_some() {
                s.last_position = position;
            }
            s.window_width = w;
            s.window_height = h;
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
            let menu_just_opened = st.lock_menu_shown_at().is_some_and(|t| t.elapsed() < MENU_BLUR_GRACE);
            if !recently_shown && !menu_just_opened && !st.settings().keep_open_on_blur {
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
    let zoom = zoom_factor(&state(app).settings());
    // Larger at a larger interface size, but never larger than the screen (less its title bar).
    let screen = app.primary_monitor().ok().flatten().as_ref().map(screen_of).map(|(w, h)| (w, h - 40.0));
    let ((width, height), (min_width, min_height)) = scaled_size(SETTINGS_SIZE, SETTINGS_MIN, zoom, screen);
    let built = WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App("settings.html".into()))
        .title("Ditto-RS Options")
        .inner_size(width, height)
        .min_inner_size(min_width, min_height)
        .center()
        .focused(true)
        .build();
    match built {
        Ok(w) => {
            apply_zoom(&w, zoom);
            let _ = w.set_focus();
        }
        Err(e) => log::error!("cannot open the options window: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Option<(f64, f64)> = Some((1512.0, 944.0));

    #[test]
    fn windows_scale_with_the_interface_size() {
        assert_eq!(scaled_size((420.0, 520.0), POPUP_MIN, 1.0, SCREEN), ((420.0, 520.0), (260.0, 200.0)));
        assert_eq!(scaled_size((420.0, 520.0), POPUP_MIN, 1.5, SCREEN), ((630.0, 780.0), (390.0, 300.0)));
        assert_eq!(scaled_size((420.0, 520.0), POPUP_MIN, 0.75, SCREEN), ((315.0, 390.0), (195.0, 150.0)));
        // Never larger than the screen, and the minimum gives way too.
        assert_eq!(scaled_size((657.0, 667.0), POPUP_MIN, 1.5, SCREEN), ((986.0, 944.0), (390.0, 300.0)));
        assert_eq!(scaled_size((420.0, 520.0), POPUP_MIN, 3.0, Some((700.0, 500.0))), ((700.0, 500.0), (700.0, 500.0)));
        // A saved size below the minimum is brought up to it.
        assert_eq!(scaled_size((100.0, 100.0), POPUP_MIN, 2.0, None), ((520.0, 400.0), (520.0, 400.0)));
        assert_eq!(scaled_size(SETTINGS_SIZE, SETTINGS_MIN, 2.0, Some((1512.0, 904.0))), ((1512.0, 904.0), (1200.0, 904.0)));
    }

    #[test]
    fn zooming_in_and_out_again_keeps_the_window_size() {
        // The size at 100 % is what is remembered, so a window that had to be cut down to the
        // screen at 150 % and 200 % comes back unchanged.
        let base = (657, 667);
        for zoom in [1.5, 2.0, 3.0, 0.75] {
            let (shown, _) = scaled_size((657.0, 667.0), POPUP_MIN, zoom, SCREEN);
            assert_eq!(base_after(shown, shown, base, zoom), base, "at {zoom}");
        }
        assert_eq!(scaled_size((657.0, 667.0), POPUP_MIN, 1.0, SCREEN).0, (657.0, 667.0));
    }

    #[test]
    fn a_window_resized_by_hand_is_remembered_at_its_100_percent_size() {
        let (expected, _) = scaled_size((420.0, 520.0), POPUP_MIN, 1.5, SCREEN);
        assert_eq!(expected, (630.0, 780.0));
        // Dragged to 900 × 600 at 150 %: 600 × 400 at 100 %.
        assert_eq!(base_after((900.0, 600.0), expected, (420, 520), 1.5), (600, 400));
        // Rounding noise from the window system is not a resize.
        assert_eq!(base_after((630.5, 779.0), expected, (420, 520), 1.5), (420, 520));
        assert_eq!(base_after((500.0, 520.0), (420.0, 520.0), (420, 520), 1.0), (500, 520));
    }
}
