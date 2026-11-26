//! System tray / menu bar icon (Ditto: the tray icon and its `IDR_MENU` menu).

use std::sync::atomic::Ordering;

use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{EVENT_CAPTURE_CHANGED, state};
use crate::{capture, commands, popup};

const TRAY_ID: &str = "main";

/// Menu items whose state changes at runtime.
struct TrayItems {
    capture: CheckMenuItem<tauri::Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let paused = state(app).capture_paused.load(Ordering::Relaxed);
    let capture = CheckMenuItem::with_id(app, "capture", "Connect To Clipboard", true, !paused, None::<&str>)?;
    app.manage(TrayItems { capture: capture.clone() });
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "show", "Show Quick Paste", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &capture,
            &MenuItem::with_id(app, "save_current", "Save Current Clipboard", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "import", "Import Ditto Database…", true, None::<&str>)?,
            &MenuItem::with_id(app, "options", "Options…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?,
        ],
    )?;

    #[cfg(target_os = "macos")]
    let (icon, template) = (tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?, true);
    #[cfg(not(target_os = "macos"))]
    let (icon, template) = (tauri::image::Image::from_bytes(include_bytes!("../icons/64x64.png"))?, false);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(template)
        .tooltip("Ditto-RS")
        .menu(&menu)
        // Ditto: left click opens the quick-paste window, right click shows the menu.
        // On macOS menu bar items conventionally open their menu on click.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(on_tray_event)
        .build(app)?;
    Ok(())
}

fn on_tray_event(tray: &TrayIcon, event: TrayIconEvent) {
    if cfg!(target_os = "macos") {
        return;
    }
    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
        popup::show(tray.app_handle());
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "show" => popup::show(app),
        "capture" => {
            let paused = !state(app).capture_paused.load(Ordering::Relaxed);
            set_capture_paused(app, paused);
        }
        "save_current" => {
            let app = app.clone();
            std::thread::spawn(move || {
                capture::capture_now(&app, true);
            });
        }
        "import" => {
            let app = app.clone();
            std::thread::spawn(move || commands::import_ditto_with_dialog(&app));
        }
        "options" => popup::open_settings(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Ditto "Connect To Clipboard" / "Disconnect From Clipboard".
pub fn set_capture_paused(app: &AppHandle, paused: bool) {
    state(app).capture_paused.store(paused, Ordering::Relaxed);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(if paused { "Ditto-RS (not saving copies)" } else { "Ditto-RS" }));
    }
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.capture.set_checked(!paused);
    }
    let _ = app.emit(EVENT_CAPTURE_CHANGED, paused);
}
