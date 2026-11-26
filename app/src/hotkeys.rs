//! Global hotkeys (Ditto: `CHotKeys`): open the window, text-only paste, paste positions
//! 1–10 ("Global Hot Keys for Last Ten Items Copied") and per-clip hotkeys.
//!
//! Uses the `global-hotkey` crate directly (0.10 recognises ISO keys such as
//! `IntlBackslash`, which tauri-plugin-global-shortcut's pinned 0.8 rejects). macOS requires
//! the manager to be created and used on the main thread, so every call is routed through
//! `AppHandle::run_on_main_thread` (which runs inline when already on the main thread).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, mpsc};

use ditto_core::store::ListScope;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tauri::{AppHandle, Manager};

use crate::paste::{self, PasteRequest};
use crate::popup;
use crate::state::state;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    ShowPopup,
    TextOnlyPaste,
    /// 0-based position in the main list.
    PastePosition(usize),
    PasteClip(i64),
}

struct HotkeyManager {
    manager: GlobalHotKeyManager,
    registered: Mutex<Vec<HotKey>>,
}

// SAFETY: the manager is created in `setup` and only used inside `on_main_thread`, i.e. on
// the main thread, which is what the platform implementations require.
unsafe impl Send for HotkeyManager {}
unsafe impl Sync for HotkeyManager {}

/// Parse an accelerator such as `Ctrl+Shift+KeyV`, `Super+Backquote` or `Ctrl+IntlBackslash`.
pub fn parse(accelerator: &str) -> Result<HotKey, String> {
    accelerator.trim().parse::<HotKey>().map_err(|e| format!("“{accelerator}”: {e}"))
}

/// Create the OS hotkey manager and route its events. Call from `setup` (main thread).
pub fn init(app: &AppHandle) -> anyhow::Result<()> {
    let manager = GlobalHotKeyManager::new()?;
    app.manage(Arc::new(HotkeyManager { manager, registered: Mutex::new(Vec::new()) }));
    let handle = app.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state() != HotKeyState::Pressed {
            return;
        }
        let app = handle.clone();
        let _ = handle.run_on_main_thread(move || on_hotkey(&app, event.id()));
    }));
    Ok(())
}

/// Run `f` with the manager on the main thread and wait for its result.
fn on_main_thread<T: Send + 'static>(app: &AppHandle, f: impl FnOnce(&HotkeyManager) -> T + Send + 'static) -> Option<T> {
    let manager = app.try_state::<Arc<HotkeyManager>>()?.inner().clone();
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f(&manager));
    })
    .ok()?;
    rx.recv().ok()
}

/// (Re-)register every hotkey from the settings and the clips. Returns problems found
/// (invalid accelerators, combinations taken by another app, duplicates).
pub fn register_all(app: &AppHandle) -> Vec<String> {
    let st = state(app);
    let settings = st.settings();

    let mut wanted: Vec<(String, HotkeyAction, String)> = Vec::new();
    wanted.push((settings.hotkey_activate.clone(), HotkeyAction::ShowPopup, "Activate Ditto".into()));
    wanted.push((settings.hotkey_text_only_paste.clone(), HotkeyAction::TextOnlyPaste, "Text Only Paste".into()));
    for (i, hk) in settings.position_hotkeys.iter().enumerate() {
        wanted.push((hk.clone(), HotkeyAction::PastePosition(i), format!("Position {}", i + 1)));
    }
    match st.store().hotkey_clips() {
        Ok(clips) => wanted.extend(clips.into_iter().map(|(id, hk)| (hk, HotkeyAction::PasteClip(id), format!("Clip {id}")))),
        Err(e) => log::warn!("could not read clip hotkeys: {e}"),
    }

    let mut problems = Vec::new();
    let mut seen = HashSet::new();
    let mut parsed = Vec::new();
    for (accelerator, action, label) in wanted {
        if accelerator.trim().is_empty() {
            continue;
        }
        match parse(&accelerator) {
            Ok(hotkey) if !seen.insert(hotkey.id()) => {
                problems.push(format!("{label}: {accelerator} is already used by another Ditto-RS hotkey"));
            }
            Ok(hotkey) => parsed.push((hotkey, action, label, accelerator)),
            Err(e) => problems.push(format!("{label}: {e}")),
        }
    }

    let outcome = on_main_thread(app, move |m| {
        let mut registered = m.registered.lock().unwrap_or_else(|p| p.into_inner());
        if !registered.is_empty() {
            if let Err(e) = m.manager.unregister_all(&registered) {
                log::warn!("could not unregister hotkeys: {e}");
            }
            registered.clear();
        }
        let mut actions = HashMap::new();
        let mut failures = Vec::new();
        for (hotkey, action, label, accelerator) in parsed {
            match m.manager.register(hotkey) {
                Ok(()) => {
                    registered.push(hotkey);
                    actions.insert(hotkey.id(), action);
                }
                Err(e) => failures.push(format!("{label}: could not register {accelerator} ({e})")),
            }
        }
        (actions, failures)
    });
    let (actions, failures) =
        outcome.unwrap_or_else(|| (HashMap::new(), vec!["global hotkeys are unavailable on this system".into()]));
    problems.extend(failures);
    for p in &problems {
        log::warn!("hotkey: {p}");
    }
    *st.lock_hotkeys() = actions;
    *st.lock_hotkey_problems() = problems.clone();
    problems
}

fn on_hotkey(app: &AppHandle, id: u32) {
    let Some(action) = state(app).lock_hotkeys().get(&id).copied() else { return };
    match action {
        HotkeyAction::ShowPopup => popup::show(app),
        HotkeyAction::TextOnlyPaste => {
            let app = app.clone();
            std::thread::spawn(move || paste::paste_clipboard_as_plain_text(&app));
        }
        HotkeyAction::PastePosition(position) => {
            let st = state(app);
            let settings = st.settings();
            let id = st.store().clip_at(ListScope::Main, settings.show_grouped_in_main, position).ok().flatten();
            if let Some(id) = id {
                run_paste(app, id, Some(settings.position_hotkeys_send_paste));
            }
        }
        HotkeyAction::PasteClip(id) => run_paste(app, id, Some(true)),
    }
}

fn run_paste(app: &AppHandle, id: i64, send_paste: Option<bool>) {
    let req = PasteRequest { ids: vec![id], send_paste, ..Default::default() };
    if let Err(e) = paste::paste(app, req) {
        log::warn!("hotkey paste failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_accelerators_the_ui_records() {
        // The options window stores KeyboardEvent.code names with our modifier names.
        for acc in ["Ctrl+Backquote", "Super+IntlBackslash", "Ctrl+Shift+KeyV", "Alt+Digit1", "Ctrl+Alt+Numpad0", "Shift+F12"] {
            assert!(parse(acc).is_ok(), "{acc} should parse");
        }
        assert!(parse("Ctrl+NoSuchKey").is_err());
        assert_ne!(parse("Super+IntlBackslash").unwrap().id(), parse("Super+Backquote").unwrap().id());
    }
}
