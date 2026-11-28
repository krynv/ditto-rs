//! Clipboard capture: watch for changes, read them and save them to the history
//! (Ditto: `CClipboardViewer` + `CCopyThread` + `CClip::AddToDB`).

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use ditto_core::settings::Settings;
use ditto_core::store::{AddOptions, DuplicatePolicy, Store};
use ditto_platform::clipboard::{self, ReadOptions, ReadOutcome};
use ditto_platform::focus;
use tauri::AppHandle;

use crate::state::{emit_clips_changed, state};

/// Delay before reading a change: some apps publish their formats in several steps
/// (Ditto: `ProcessDrawClipboardDelay`, 100 ms).
const SETTLE_DELAY: Duration = Duration::from_millis(120);

pub fn start(app: &AppHandle) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<()>();
    let watcher = clipboard::watch(move || {
        let _ = tx.send(());
    })?;
    *state(app).lock_watcher() = Some(watcher);

    let handle = app.clone();
    std::thread::Builder::new().name("clip-capture".into()).spawn(move || {
        while rx.recv().is_ok() {
            std::thread::sleep(SETTLE_DELAY);
            while rx.try_recv().is_ok() {}
            capture_now(&handle, false);
        }
    })?;

    // Hourly sweep so "expire after N days" also applies while nothing is copied.
    let handle = app.clone();
    std::thread::Builder::new().name("retention".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3600));
        let st = state(&handle);
        let settings = st.settings();
        if apply_retention(&mut st.store(), &settings) > 0 {
            emit_clips_changed(&handle);
        }
    })?;
    Ok(())
}

/// Read the clipboard and save it. `forced` ignores the pause switch and app filters
/// ("Save Current Clipboard"). Returns the id of the saved (or matching existing) clip.
pub fn capture_now(app: &AppHandle, forced: bool) -> Option<i64> {
    let st = state(app);
    if !forced && st.capture_paused.load(Ordering::Relaxed) {
        return None;
    }
    let settings = st.settings();
    let source_app = focus::foreground_app_name();
    if !forced && !settings.accepts_app(source_app.as_deref()) {
        log::info!("not saving a copy made in {source_app:?} (include/exclude list)");
        return None;
    }
    match clipboard::read(&ReadOptions::from_settings(&settings)) {
        Ok(ReadOutcome::Content(content)) => {
            let opts = AddOptions {
                duplicates: if settings.allow_duplicates { DuplicatePolicy::Allow } else { DuplicatePolicy::MoveToTop },
                description_chars: settings.description_chars as usize,
                source_app,
                ..Default::default()
            };
            let result = {
                let mut store = st.store();
                let result = store.add_clip(&content, &opts);
                if result.is_ok() {
                    apply_retention(&mut store, &settings);
                }
                result
            };
            match result {
                Ok(outcome) => {
                    log::debug!("saved clip: {outcome:?}");
                    emit_clips_changed(app);
                    // A new image is read in the background so it can be found by its text.
                    crate::image_text::wake(app);
                    Some(outcome.id())
                }
                Err(e) => {
                    log::warn!("could not save clip: {e}");
                    None
                }
            }
        }
        Ok(ReadOutcome::OwnWrite) => {
            log::debug!("ignoring our own clipboard write");
            None
        }
        Ok(ReadOutcome::Concealed) => {
            log::info!("not saving a copy marked as private by its app");
            None
        }
        Ok(ReadOutcome::TooLarge(size)) => {
            log::info!("not saving a {size}-byte clip (limit {} bytes)", settings.max_clip_bytes);
            None
        }
        Ok(ReadOutcome::Empty) => None,
        Err(e) => {
            log::warn!("could not read the clipboard: {e}");
            None
        }
    }
}

/// Ditto's `RemoveOldEntries` with the user's limits. Returns the number of deleted clips.
pub fn apply_retention(store: &mut Store, settings: &Settings) -> usize {
    let max = settings.limit_entries.then_some(settings.max_entries);
    let days = settings.expire_entries.then_some(settings.expire_days);
    if max.is_none() && days.is_none() {
        return 0;
    }
    match store.enforce_retention(max, days) {
        Ok(n) => {
            if n > 0 {
                log::info!("auto-deleted {n} old clip(s)");
            }
            n
        }
        Err(e) => {
            log::warn!("auto-delete failed: {e}");
            0
        }
    }
}
