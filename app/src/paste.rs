//! Pasting (Ditto: `CProcessPaste` / `COleClipSource`): build the content, put it on the
//! clipboard, give focus back to the target window and send the paste keystroke.

use std::sync::atomic::Ordering;
use std::time::Duration;

use ditto_core::model::ClipContent;
use ditto_core::settings::Settings;
use ditto_core::store::Store;
use ditto_core::{TextTransform, imaging, transform};
use ditto_platform::clipboard::{self, ReadOptions, ReadOutcome};
use ditto_platform::focus::{self, ForeignWindow};
use ditto_platform::input;
use serde::Deserialize;
use tauri::AppHandle;

use crate::popup;
use crate::state::{emit_clips_changed, state};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PasteRequest {
    /// In list order (top first).
    pub ids: Vec<i64>,
    /// "Paste Plain Text Only" (Shift+Enter).
    pub plain_text: bool,
    /// A Special Paste text transform.
    pub transform: Option<TextTransform>,
    /// "Multiple Images Horizontally/Vertically".
    pub combine_images: Option<Combine>,
    /// "Paste, Don't Change Clip Order".
    pub keep_order: bool,
    /// Override "send paste after selection" (false = only load the clipboard).
    pub send_paste: Option<bool>,
    /// The request comes from the quick-paste window (hide it and restore focus).
    pub from_popup: bool,
}

/// Build what goes on the clipboard.
pub fn build_content(store: &Store, settings: &Settings, req: &PasteRequest) -> Result<ClipContent, String> {
    if req.ids.is_empty() {
        return Err("nothing selected".into());
    }
    let mut contents = req.ids.iter().map(|&id| store.content(id)).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;

    if let Some(direction) = req.combine_images {
        let pngs: Vec<&[u8]> = contents.iter().filter_map(ClipContent::png).collect();
        if pngs.is_empty() {
            return Err("none of the selected clips is an image".into());
        }
        let png = imaging::combine_png(&pngs, direction == Combine::Horizontal).map_err(|e| e.to_string())?;
        return Ok(ClipContent::from_png(png));
    }

    if contents.len() == 1 && !req.plain_text && req.transform.is_none() {
        return Ok(contents.remove(0));
    }

    // Multi-paste: Ditto pastes bottom-up (oldest first) by default.
    if settings.multi_paste_reverse {
        contents.reverse();
    }
    if contents.len() > 1 && req.transform.is_none() && !req.plain_text && contents.iter().all(|c| !c.files().is_empty()) {
        // Like Ditto's CF_HDROP aggregator: one combined file list.
        let files: Vec<String> = contents.iter().flat_map(ClipContent::files).collect();
        let mut merged = ClipContent::from_files(&files);
        merged.set(ditto_core::formats::TEXT, files.join(transform::LINE_ENDING).into_bytes());
        return Ok(merged);
    }
    let texts: Vec<String> = contents.iter().filter_map(ClipContent::plain_text).collect();
    if texts.is_empty() {
        return Err("the selected clips have no text".into());
    }
    let mut text = texts.join(&transform::expand_separator(&settings.multi_paste_separator));
    if let Some(t) = req.transform {
        text = t.apply(&text);
    }
    Ok(ClipContent::from_text(text))
}

pub fn paste(app: &AppHandle, req: PasteRequest) -> Result<(), String> {
    log::debug!("paste request: {req:?}");
    let st = state(app);
    let settings = st.settings();
    let content = build_content(&st.store(), &settings, &req)?;
    clipboard::write(&content).map_err(|e| e.to_string())?;
    {
        let mut store = st.store();
        let move_to_top = settings.move_to_top_on_paste && !req.keep_order;
        for &id in &req.ids {
            if let Err(e) = store.mark_pasted(id, move_to_top) {
                log::warn!("could not update clip {id}: {e}");
            }
        }
    }
    emit_clips_changed(app);

    let target = if req.from_popup { st.lock_previous_window().clone() } else { None };
    if req.from_popup {
        popup::hide(app);
    }
    if req.send_paste.unwrap_or(settings.paste_after_selection) {
        send_paste_to(app, target, settings.paste_delay_ms);
    }
    Ok(())
}

/// Give focus back to `target` (if any) and send the paste keystroke.
///
/// The waiting happens on a helper thread, but focus changes and the keystroke itself are
/// dispatched to the main thread: on macOS, enigo resolves the layout-dependent key code
/// through the Text Input Sources API, which traps when called off the main queue.
fn send_paste_to(app: &AppHandle, target: Option<ForeignWindow>, delay_ms: u64) {
    if !input::can_send_keystrokes(false) {
        // The clip is on the clipboard; ask once for the permission needed to paste it.
        if !state(app).accessibility_prompted.swap(true, Ordering::SeqCst) {
            input::can_send_keystrokes(true);
        }
        log::warn!("the paste keystroke needs the Accessibility permission (System Settings → Privacy & Security)");
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let focus_app = app.clone();
        let focus_target = target.clone();
        let refocus = app.run_on_main_thread(move || {
            let activated = focus_target.as_ref().is_some_and(focus::activate);
            log::debug!("gave focus back to {:?}: {activated}", focus_target.as_ref().map(|t| &t.app_name));
            #[cfg(target_os = "macos")]
            if !activated {
                // Hiding the app hands activation back to the previously active app.
                let _ = focus_app.hide();
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (activated, &focus_app);
        });
        if let Err(e) = refocus {
            log::warn!("could not restore focus: {e}");
        }
        // Activation is asynchronous; do not type into whatever happens to be in front.
        if let Some(t) = &target {
            if !focus::wait_until_foreground(t, Duration::from_millis(800)) {
                log::debug!("{:?} did not come to the front in time", t.app_name);
            }
        }
        if !input::wait_for_modifiers_released(Duration::from_millis(1500)) {
            log::debug!("modifier keys still held; pasting anyway");
        }
        std::thread::sleep(Duration::from_millis(delay_ms));
        let sent = app.run_on_main_thread(|| {
            log::debug!("sending the paste keystroke");
            if let Err(e) = input::send_paste() {
                log::warn!("could not send the paste keystroke: {e}");
            }
        });
        if let Err(e) = sent {
            log::warn!("could not schedule the paste keystroke: {e}");
        }
    });
}

/// "Text Only Paste" global hotkey: paste the current clipboard as plain text.
pub fn paste_clipboard_as_plain_text(app: &AppHandle) {
    let settings = state(app).settings();
    let opts = ReadOptions { images: false, respect_privacy_markers: false, include_own_writes: true, ..ReadOptions::default() };
    let text = match clipboard::read(&opts) {
        Ok(ReadOutcome::Content(c)) => c.plain_text(),
        _ => None,
    };
    let Some(text) = text else { return };
    if let Err(e) = clipboard::write(&ClipContent::from_text(text)) {
        log::warn!("text-only paste failed: {e}");
        return;
    }
    send_paste_to(app, None, settings.paste_delay_ms);
}

#[cfg(test)]
mod tests {
    use ditto_core::formats;
    use ditto_core::store::AddOptions;

    use super::*;

    fn setup() -> (Store, Vec<i64>) {
        let mut store = Store::open_in_memory().unwrap();
        let ids = ["first", "second", "third"]
            .iter()
            .map(|t| store.add_clip(&ClipContent::from_text(*t), &AddOptions::default()).unwrap().id())
            .collect::<Vec<_>>();
        (store, ids)
    }

    #[test]
    fn single_clip_keeps_all_formats() {
        let mut store = Store::open_in_memory().unwrap();
        let rich = ClipContent::from_text("hi").with(formats::HTML, b"<b>hi</b>".to_vec());
        let id = store.add_clip(&rich, &AddOptions::default()).unwrap().id();
        let s = Settings::default();
        let req = PasteRequest { ids: vec![id], ..Default::default() };
        assert_eq!(build_content(&store, &s, &req).unwrap(), rich);
        let plain = PasteRequest { ids: vec![id], plain_text: true, ..Default::default() };
        assert_eq!(build_content(&store, &s, &plain).unwrap(), ClipContent::from_text("hi"));
    }

    #[test]
    fn multi_paste_order_separator_and_transform() {
        let (store, ids) = setup();
        let mut s = Settings::default();
        // UI order is newest first: third, second, first.
        let ui_order = vec![ids[2], ids[1], ids[0]];
        let req = PasteRequest { ids: ui_order.clone(), ..Default::default() };
        let nl = transform::LINE_ENDING;
        assert_eq!(build_content(&store, &s, &req).unwrap().text().unwrap(), format!("first{nl}second{nl}third"));
        s.multi_paste_reverse = false;
        s.multi_paste_separator = ", ".into();
        assert_eq!(build_content(&store, &s, &req).unwrap().text().unwrap(), "third, second, first");
        let upper = PasteRequest { ids: ui_order, transform: Some(TextTransform::UpperCase), ..Default::default() };
        assert_eq!(build_content(&store, &s, &upper).unwrap().text().unwrap(), "THIRD, SECOND, FIRST");
        assert!(build_content(&store, &s, &PasteRequest::default()).is_err());
    }

    #[test]
    fn file_lists_are_merged() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_clip(&ClipContent::from_files(&["/a".into()]), &AddOptions::default()).unwrap().id();
        let b = store.add_clip(&ClipContent::from_files(&["/b".into(), "/c".into()]), &AddOptions::default()).unwrap().id();
        let merged = build_content(&store, &Settings::default(), &PasteRequest { ids: vec![b, a], ..Default::default() }).unwrap();
        assert_eq!(merged.files(), vec!["/a", "/b", "/c"]);
    }
}
