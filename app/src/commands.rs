//! Commands invoked by the web UI (`window.__TAURI__.core.invoke`).

use std::sync::atomic::Ordering;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ditto_core::ditto_import::{self, ImportOptions, ImportReport};
use ditto_core::model::{ClipContent, ClipKind, ClipSummary, Group, png_dimensions};
use ditto_core::search::{CompiledSearch, SearchMode, SearchScopes, SearchSpec};
use ditto_core::settings::Settings;
use ditto_core::store::{AddOptions, DuplicatePolicy, ListPage, ListQuery, ListScope, OrderOp, Stats, StickyOp};
use ditto_core::TextTransform;
use ditto_platform::input;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::paste::{self, PasteRequest};
use crate::state::{EVENT_SETTINGS_CHANGED, emit_clips_changed, emit_groups_changed, state};
use crate::{capture, hotkeys, popup, tray};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", B64.encode(bytes))
}

// ---- listing ------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ListRequest {
    pub scope: ListScope,
    pub query: String,
    pub mode: Option<SearchMode>,
    pub scopes: Option<SearchScopes>,
    pub offset: usize,
    pub limit: usize,
}

impl Default for ListRequest {
    fn default() -> Self {
        Self { scope: ListScope::Main, query: String::new(), mode: None, scopes: None, offset: 0, limit: 100 }
    }
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    #[serde(flatten)]
    page: ListPage,
    /// Set when the search pattern is invalid (the list is then unfiltered).
    search_error: Option<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn list_clips(app: AppHandle, request: ListRequest) -> CmdResult<ListResponse> {
    let st = state(&app);
    let settings = st.settings();
    let spec = SearchSpec {
        query: request.query.clone(),
        mode: request.mode.unwrap_or(settings.search_mode),
        scopes: request.scopes.unwrap_or(settings.search_scopes),
    };
    let (search, search_error) = match CompiledSearch::compile(&spec) {
        Ok(s) => (s, None),
        Err(e) => (None, Some(e.to_string())),
    };
    if search_error.is_some() {
        return Ok(ListResponse { page: ListPage { items: Vec::new(), has_more: false }, search_error });
    }
    let query = ListQuery {
        scope: request.scope,
        include_grouped: settings.show_grouped_in_main,
        search: search.as_ref(),
        offset: request.offset,
        limit: request.limit.clamp(1, 1000),
    };
    let page = st.store().list(&query).map_err(err)?;
    Ok(ListResponse { page, search_error: None })
}

#[derive(Debug, Serialize)]
pub struct FormatInfo {
    name: String,
    size: usize,
}

#[derive(Debug, Serialize)]
pub struct ClipDetails {
    summary: ClipSummary,
    text: Option<String>,
    text_truncated: bool,
    html: Option<String>,
    rtf_text: Option<String>,
    image: Option<String>,
    image_size: Option<(u32, u32)>,
    files: Vec<String>,
    formats: Vec<FormatInfo>,
}

const PREVIEW_TEXT_LIMIT: usize = 2_000_000;

#[tauri::command(rename_all = "snake_case")]
pub fn clip_details(app: AppHandle, id: i64) -> CmdResult<ClipDetails> {
    let st = state(&app);
    let store = st.store();
    let summary = store.summary(id).map_err(err)?;
    let content = store.content(id).map_err(err)?;
    let mut text = content.text().map(str::to_string);
    let mut text_truncated = false;
    if let Some(t) = &mut text {
        if t.len() > PREVIEW_TEXT_LIMIT {
            let cut = ditto_core::text::truncate_chars(t, PREVIEW_TEXT_LIMIT).len();
            t.truncate(cut);
            text_truncated = true;
        }
    }
    Ok(ClipDetails {
        text,
        text_truncated,
        html: content.html().map(str::to_string),
        rtf_text: content.rtf().map(ditto_core::text::rtf_to_text),
        image: content.png().map(|p| data_url("image/png", p)),
        image_size: content.png().and_then(png_dimensions),
        files: content.files(),
        formats: content.formats().iter().map(|f| FormatInfo { name: f.name.clone(), size: f.data.len() }).collect(),
        summary,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub fn clip_thumbnail(app: AppHandle, id: i64) -> CmdResult<Option<String>> {
    Ok(state(&app).store().thumbnail(id).map_err(err)?.map(|t| data_url("image/png", &t)))
}

// ---- pasting ------------------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
pub fn paste_clips(app: AppHandle, request: PasteRequest) -> CmdResult<()> {
    paste::paste(&app, request)
}

/// Put the selection on the clipboard without pasting or closing (Ctrl+C in the list).
#[tauri::command(rename_all = "snake_case")]
pub fn copy_clips(app: AppHandle, ids: Vec<i64>, plain_text: bool) -> CmdResult<()> {
    paste::paste(&app, PasteRequest { ids, plain_text, keep_order: true, send_paste: Some(false), ..Default::default() })
}

#[derive(Debug, Serialize)]
pub struct TransformInfo {
    id: TextTransform,
    label: &'static str,
}

#[tauri::command(rename_all = "snake_case")]
pub fn text_transforms() -> Vec<TransformInfo> {
    TextTransform::ALL.iter().map(|&t| TransformInfo { id: t, label: t.label() }).collect()
}

// ---- editing ------------------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
pub fn delete_clips(app: AppHandle, ids: Vec<i64>) -> CmdResult<usize> {
    let st = state(&app);
    let had_hotkey = ids.iter().any(|id| st.store().summary(*id).is_ok_and(|s| s.hotkey.is_some()));
    let n = st.store().delete_clips(&ids).map_err(err)?;
    if had_hotkey {
        hotkeys::register_all(&app);
    }
    emit_clips_changed(&app);
    emit_groups_changed(&app);
    Ok(n)
}

#[tauri::command(rename_all = "snake_case")]
pub fn update_clip_text(app: AppHandle, id: i64, text: String) -> CmdResult<()> {
    let st = state(&app);
    let chars = st.settings().description_chars as usize;
    st.store().update_text(id, &text, chars).map_err(err)?;
    emit_clips_changed(&app);
    Ok(())
}

/// Ditto "New Clip".
#[tauri::command(rename_all = "snake_case")]
pub fn new_clip(app: AppHandle, text: String, group_id: Option<i64>) -> CmdResult<i64> {
    if text.is_empty() {
        return Err("the clip is empty".into());
    }
    let st = state(&app);
    let opts = AddOptions {
        duplicates: DuplicatePolicy::MoveToTop,
        description_chars: st.settings().description_chars as usize,
        group_id,
        count_as_copy: false,
        ..Default::default()
    };
    let id = st.store().add_clip(&ClipContent::from_text(text), &opts).map_err(err)?.id();
    emit_clips_changed(&app);
    emit_groups_changed(&app);
    Ok(id)
}

#[derive(Debug, Deserialize)]
pub struct ClipProperties {
    description: String,
    quick_paste_text: Option<String>,
    never_auto_delete: bool,
    hotkey: Option<String>,
}

/// Ditto "Copy Properties" dialog.
#[tauri::command(rename_all = "snake_case")]
pub fn update_clip_properties(app: AppHandle, id: i64, properties: ClipProperties) -> CmdResult<Vec<String>> {
    let st = state(&app);
    let hotkey = properties.hotkey.as_deref().map(str::trim).filter(|h| !h.is_empty());
    if let Some(h) = hotkey {
        hotkeys::parse(h)?;
    }
    let hotkey_changed;
    {
        let mut store = st.store();
        let before = store.summary(id).map_err(err)?;
        if !properties.description.trim().is_empty() && properties.description != before.description {
            store.set_description(id, &properties.description).map_err(err)?;
        }
        store.set_quick_paste_text(id, properties.quick_paste_text.as_deref()).map_err(err)?;
        store.set_never_auto_delete(id, properties.never_auto_delete).map_err(err)?;
        hotkey_changed = before.hotkey.as_deref() != hotkey;
        if hotkey_changed {
            store.set_hotkey(id, hotkey).map_err(err)?;
        }
    }
    let problems = if hotkey_changed { hotkeys::register_all(&app) } else { Vec::new() };
    emit_clips_changed(&app);
    Ok(problems)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_sticky(app: AppHandle, id: i64, op: StickyOp, scope: ListScope) -> CmdResult<()> {
    state(&app).store().set_sticky(id, op, scope).map_err(err)?;
    emit_clips_changed(&app);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn move_clip(app: AppHandle, id: i64, op: OrderOp, scope: ListScope) -> CmdResult<()> {
    let st = state(&app);
    let include_grouped = st.settings().show_grouped_in_main;
    st.store().move_clip(id, op, scope, include_grouped).map_err(err)?;
    emit_clips_changed(&app);
    Ok(())
}

// ---- groups -------------------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
pub fn list_groups(app: AppHandle) -> CmdResult<Vec<Group>> {
    state(&app).store().groups().map_err(err)
}

#[tauri::command(rename_all = "snake_case")]
pub fn create_group(app: AppHandle, name: String, parent_id: Option<i64>, clip_ids: Option<Vec<i64>>) -> CmdResult<i64> {
    let st = state(&app);
    let id = {
        let mut store = st.store();
        let id = store.create_group(&name, parent_id).map_err(err)?;
        if let Some(ids) = clip_ids.filter(|ids| !ids.is_empty()) {
            store.move_to_group(&ids, Some(id)).map_err(err)?;
        }
        id
    };
    emit_groups_changed(&app);
    emit_clips_changed(&app);
    Ok(id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn rename_group(app: AppHandle, id: i64, name: String) -> CmdResult<()> {
    state(&app).store().rename_group(id, &name).map_err(err)?;
    emit_groups_changed(&app);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn delete_group(app: AppHandle, id: i64, delete_clips: bool) -> CmdResult<()> {
    state(&app).store().delete_group(id, delete_clips).map_err(err)?;
    emit_groups_changed(&app);
    emit_clips_changed(&app);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn move_to_group(app: AppHandle, ids: Vec<i64>, group_id: Option<i64>) -> CmdResult<()> {
    state(&app).store().move_to_group(&ids, group_id).map_err(err)?;
    emit_groups_changed(&app);
    emit_clips_changed(&app);
    Ok(())
}

// ---- settings -----------------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
pub fn get_settings(app: AppHandle) -> Settings {
    state(&app).settings()
}

#[derive(Debug, Serialize)]
pub struct SaveSettingsResult {
    settings: Settings,
    /// Hotkeys that could not be registered, etc.
    warnings: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_settings(app: AppHandle, settings: Settings) -> CmdResult<SaveSettingsResult> {
    for hk in std::iter::once(&settings.hotkey_activate)
        .chain(std::iter::once(&settings.hotkey_text_only_paste))
        .chain(settings.position_hotkeys.iter())
        .filter(|h| !h.trim().is_empty())
    {
        hotkeys::parse(hk)?;
    }
    let st = state(&app);
    let saved = st.set_settings(settings).map_err(err)?;
    let mut warnings = hotkeys::register_all(&app);
    if let Err(e) = apply_autostart(&app, saved.start_on_login) {
        warnings.push(format!("Start on login: {e}"));
    }
    if capture::apply_retention(&mut st.store(), &saved) > 0 {
        emit_clips_changed(&app);
    }
    let _ = app.emit(EVENT_SETTINGS_CHANGED, &saved);
    Ok(SaveSettingsResult { settings: saved, warnings })
}

pub fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    let current = launcher.is_enabled().unwrap_or(false);
    if enabled && !current {
        launcher.enable().map_err(err)?;
    } else if !enabled && current {
        launcher.disable().map_err(err)?;
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn reset_settings(app: AppHandle) -> CmdResult<SaveSettingsResult> {
    save_settings(app, Settings::default())
}

// ---- app / database -----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AppInfo {
    version: &'static str,
    os: &'static str,
    data_dir: String,
    db_path: String,
    portable: bool,
    can_paste: bool,
    capture_paused: bool,
    hotkey_problems: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn app_info(app: AppHandle) -> AppInfo {
    let st = state(&app);
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        data_dir: st.paths.data_dir.display().to_string(),
        db_path: st.paths.db.display().to_string(),
        portable: st.paths.portable,
        can_paste: input::can_send_keystrokes(false),
        capture_paused: st.capture_paused.load(Ordering::Relaxed),
        hotkey_problems: st.lock_hotkey_problems().clone(),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn stats(app: AppHandle) -> CmdResult<Stats> {
    state(&app).store().stats().map_err(err)
}

#[tauri::command(rename_all = "snake_case")]
pub fn compact_database(app: AppHandle) -> CmdResult<Stats> {
    let st = state(&app);
    let mut store = st.store();
    store.compact().map_err(err)?;
    store.stats().map_err(err)
}

#[tauri::command(rename_all = "snake_case")]
pub fn backup_database(app: AppHandle) -> CmdResult<String> {
    let st = state(&app);
    let dir = st.paths.backups_dir();
    std::fs::create_dir_all(&dir).map_err(err)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("history-{stamp}.db"));
    st.store().backup_to(&path).map_err(err)?;
    Ok(path.display().to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn clear_history(app: AppHandle) -> CmdResult<usize> {
    let n = state(&app).store().clear_history().map_err(err)?;
    emit_clips_changed(&app);
    Ok(n)
}

/// Pick a Ditto.db with a file dialog and import it.
#[tauri::command(rename_all = "snake_case")]
pub async fn import_ditto_database(app: AppHandle) -> CmdResult<Option<ImportReport>> {
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Import a Ditto database (Ditto.db)")
        .add_filter("Ditto database", &["db"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(err)?;
    import_from(&app, &path).map(Some)
}

pub fn import_from(app: &AppHandle, path: &std::path::Path) -> CmdResult<ImportReport> {
    let st = state(app);
    let opts = ImportOptions { description_chars: st.settings().description_chars as usize, ..Default::default() };
    let report = ditto_import::import_ditto_db(path, &mut st.store(), &opts).map_err(err)?;
    log::info!("imported {path:?}: {report:?}");
    hotkeys::register_all(app);
    emit_groups_changed(app);
    emit_clips_changed(app);
    Ok(report)
}

/// Tray menu entry: run the import and report the result in a message box.
pub fn import_ditto_with_dialog(app: &AppHandle) {
    let Some(path) = app.dialog().file().set_title("Import a Ditto database (Ditto.db)").add_filter("Ditto database", &["db"]).blocking_pick_file()
    else {
        return;
    };
    let message = match path.into_path().map_err(err).and_then(|p| import_from(app, &p)) {
        Ok(r) => format!(
            "Imported {} clip(s) and {} group(s).\n{} duplicate(s) and {} clip(s) without a supported format were skipped.",
            r.clips_imported, r.groups_imported, r.duplicates_skipped, r.unsupported_skipped
        ),
        Err(e) => format!("Import failed: {e}"),
    };
    app.dialog().message(message).title("Ditto-RS").kind(MessageDialogKind::Info).show(|_| {});
}

/// Ditto "Save To File" / "Export Clip(s) to Text/Image File(s)".
#[tauri::command(rename_all = "snake_case")]
pub async fn save_clip_to_file(app: AppHandle, id: i64) -> CmdResult<Option<String>> {
    let content = state(&app).store().content(id).map_err(err)?;
    let (bytes, ext, filter) = match content.kind() {
        ClipKind::Image => (content.png().unwrap_or_default().to_vec(), "png", "PNG image"),
        _ => match (content.plain_text(), content.html()) {
            (Some(t), _) => (t.into_bytes(), "txt", "Text"),
            (None, Some(h)) => (h.as_bytes().to_vec(), "html", "HTML"),
            _ => return Err("this clip has nothing that can be saved to a file".into()),
        },
    };
    let Some(path) = app.dialog().file().set_file_name(format!("clip-{id}.{ext}")).add_filter(filter, &[ext]).blocking_save_file() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(err)?;
    std::fs::write(&path, bytes).map_err(err)?;
    Ok(Some(path.display().to_string()))
}

// ---- windows ------------------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
pub fn hide_popup(app: AppHandle) {
    popup::hide(&app);
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_settings(app: AppHandle) {
    popup::open_settings(&app);
}

/// The quick-paste window is about to show a native context menu (see `popup::on_window_event`).
#[tauri::command(rename_all = "snake_case")]
pub fn context_menu_opening(app: AppHandle) {
    *state(&app).lock_menu_shown_at() = Some(std::time::Instant::now());
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_capture_paused(app: AppHandle, paused: bool) -> bool {
    tray::set_capture_paused(&app, paused);
    paused
}

/// Ditto "Save Current Clipboard" (works even when capture is paused).
#[tauri::command(rename_all = "snake_case")]
pub async fn save_current_clipboard(app: AppHandle) -> Option<i64> {
    capture::capture_now(&app, true)
}

#[tauri::command(rename_all = "snake_case")]
pub fn toggle_keep_open(app: AppHandle) -> bool {
    let st = state(&app);
    let keep = !st.settings().keep_open_on_blur;
    st.update_settings(|s| s.keep_open_on_blur = keep);
    keep
}

#[tauri::command(rename_all = "snake_case")]
pub fn request_paste_permission() -> bool {
    input::can_send_keystrokes(true)
}

#[tauri::command(rename_all = "snake_case")]
pub fn quit(app: AppHandle) {
    app.exit(0);
}
