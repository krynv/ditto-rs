//! Shared application state (managed by Tauri).

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::Instant;

use ditto_core::settings::Settings;
use ditto_core::store::Store;
use ditto_platform::clipboard::Watcher;
use ditto_platform::focus::ForeignWindow;
use tauri::{AppHandle, Emitter, Manager};

use crate::hotkeys::HotkeyAction;
use crate::paths::Paths;

pub const EVENT_CLIPS_CHANGED: &str = "clips-changed";
pub const EVENT_GROUPS_CHANGED: &str = "groups-changed";
pub const EVENT_SETTINGS_CHANGED: &str = "settings-changed";
pub const EVENT_CAPTURE_CHANGED: &str = "capture-changed";
pub const EVENT_POPUP_SHOWN: &str = "popup-shown";
pub const EVENT_HOTKEY_AGAIN: &str = "popup-hotkey-again";

pub struct AppState {
    pub paths: Paths,
    store: Mutex<Store>,
    settings: RwLock<Settings>,
    /// "Connect To Clipboard" unchecked.
    pub capture_paused: AtomicBool,
    /// The window that had focus before the quick-paste window opened.
    pub previous_window: Mutex<Option<ForeignWindow>>,
    /// Registered global shortcut id → action.
    pub hotkeys: Mutex<HashMap<u32, HotkeyAction>>,
    /// Problems from the last hotkey registration (shown in the UI).
    pub hotkey_problems: Mutex<Vec<String>>,
    pub watcher: Mutex<Option<Watcher>>,
    /// macOS: we asked for the Accessibility permission once this session.
    pub accessibility_prompted: AtomicBool,
    pub popup_shown_at: Mutex<Option<Instant>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppState {
    pub fn new(paths: Paths, store: Store, settings: Settings) -> Self {
        Self {
            paths,
            store: Mutex::new(store),
            settings: RwLock::new(settings),
            capture_paused: AtomicBool::new(false),
            previous_window: Mutex::new(None),
            hotkeys: Mutex::new(HashMap::new()),
            hotkey_problems: Mutex::new(Vec::new()),
            watcher: Mutex::new(None),
            accessibility_prompted: AtomicBool::new(false),
            popup_shown_at: Mutex::new(None),
        }
    }

    pub fn store(&self) -> MutexGuard<'_, Store> {
        lock(&self.store)
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Replace the settings and persist them.
    pub fn set_settings(&self, mut settings: Settings) -> ditto_core::Result<Settings> {
        settings.normalize();
        settings.save(&self.paths.settings)?;
        *self.settings.write().unwrap_or_else(|p| p.into_inner()) = settings.clone();
        Ok(settings)
    }

    /// Change a few fields and persist (errors are logged).
    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) {
        let mut settings = self.settings();
        f(&mut settings);
        if let Err(e) = self.set_settings(settings) {
            log::warn!("could not save settings: {e}");
        }
    }

    pub fn lock_previous_window(&self) -> MutexGuard<'_, Option<ForeignWindow>> {
        lock(&self.previous_window)
    }

    pub fn lock_hotkeys(&self) -> MutexGuard<'_, HashMap<u32, HotkeyAction>> {
        lock(&self.hotkeys)
    }

    pub fn lock_hotkey_problems(&self) -> MutexGuard<'_, Vec<String>> {
        lock(&self.hotkey_problems)
    }

    pub fn lock_watcher(&self) -> MutexGuard<'_, Option<Watcher>> {
        lock(&self.watcher)
    }

    pub fn lock_popup_shown_at(&self) -> MutexGuard<'_, Option<Instant>> {
        lock(&self.popup_shown_at)
    }
}

/// Tell every window to reload its list.
pub fn emit_clips_changed(app: &AppHandle) {
    let _ = app.emit(EVENT_CLIPS_CHANGED, ());
}

pub fn emit_groups_changed(app: &AppHandle) {
    let _ = app.emit(EVENT_GROUPS_CHANGED, ());
}

pub fn state(app: &AppHandle) -> tauri::State<'_, AppState> {
    app.state::<AppState>()
}
