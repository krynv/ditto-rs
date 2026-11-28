//! User options. Defaults follow Ditto's `CGetSetOptions` (Options.cpp) unless noted.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::search::{SearchMode, SearchScopes};

/// Which clipboard formats are captured (Ditto: Options → Supported Types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureFormats {
    pub text: bool,
    pub html: bool,
    pub rtf: bool,
    pub images: bool,
    pub files: bool,
}

impl Default for CaptureFormats {
    fn default() -> Self {
        Self { text: true, html: true, rtf: true, images: true, files: true }
    }
}

/// Where the quick-paste window opens (Ditto: Popup Positioning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PopupPosition {
    /// Ditto's default is "At Caret"; reading the text caret is not portable, so the
    /// mouse cursor is our default.
    #[default]
    AtCursor,
    AtPreviousPosition,
    CenterOfScreen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Smallest and largest "Interface size", in percent.
pub const UI_ZOOM_RANGE: std::ops::RangeInclusive<u32> = 50..=300;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ---- General -------------------------------------------------------------------
    /// "Start Ditto on System Startup".
    pub start_on_login: bool,
    /// "Maximum Number of Saved Copies" checkbox. Ditto enables it only for the portable
    /// build; we enable it everywhere so the history cannot grow without bound.
    pub limit_entries: bool,
    /// Ditto default: 500.
    pub max_entries: u32,
    /// "Paste Entries Expire after N Days" (off by default, 5 days).
    pub expire_entries: bool,
    pub expire_days: u32,
    /// Keep identical copies as separate clips (Ditto `AllowDuplicates`, off: a repeated copy
    /// moves the existing clip to the top).
    pub allow_duplicates: bool,
    /// Characters kept for the list description (Ditto `DescTextSize` = 500).
    pub description_chars: u32,
    /// Skip clips larger than this many bytes; 0 = no limit (Ditto `MaxClipSizeInBytes` = 0).
    pub max_clip_bytes: u64,
    pub capture: CaptureFormats,
    /// Only save copies made in these apps; `;`-separated, `*` wildcards (Ditto `CopyAppInclude`).
    pub copy_app_include: String,
    /// Never save copies made in these apps (Ditto `CopyAppExclude`).
    pub copy_app_exclude: String,
    /// Honour "do not record" markers set by password managers (Ditto honours
    /// `Clipboard Viewer Ignore` and `ExcludeClipboardContentFromMonitorProcessing`).
    pub respect_privacy_markers: bool,
    /// Recognize the text in image clips (OCR) so they can be found by searching. Not in Ditto.
    pub read_image_text: bool,

    // ---- Keyboard ------------------------------------------------------------------
    /// Global hotkey that opens the quick-paste window (Ditto `DittoHotKey` 704 = Ctrl+`).
    pub hotkey_activate: String,
    /// Global hotkey that pastes the top clip as plain text (unset by default).
    pub hotkey_text_only_paste: String,
    /// Global hotkeys for positions 1–10 ("Global Hot Keys for Last Ten Items Copied").
    pub position_hotkeys: Vec<String>,
    /// Paste after a position hotkey, otherwise only load the clipboard (Ditto: TRUE).
    pub position_hotkeys_send_paste: bool,
    /// Move a clip to the top when it is pasted (Ditto `UpdateTimeOnPaste` = TRUE).
    pub move_to_top_on_paste: bool,
    /// Pressing the activation hotkey while the window is open moves the selection down
    /// (Ditto `MoveSelectionOnOpenHotkey` = TRUE).
    pub hotkey_again_moves_selection: bool,

    // ---- Quick paste window ---------------------------------------------------------
    pub popup_position: PopupPosition,
    /// "Text Lines per Clip" (Ditto `LinesPerRow` = 2).
    pub lines_per_clip: u32,
    /// "Show text for first ten copy hot keys" (Ditto: TRUE).
    pub show_quick_paste_numbers: bool,
    pub show_leading_whitespace: bool,
    pub show_thumbnails: bool,
    pub find_as_you_type: bool,
    pub prompt_on_delete: bool,
    /// "Show Clips That are in Groups in Main List" (Ditto `ShowAllClipsInMainList` = TRUE).
    pub show_grouped_in_main: bool,
    /// Send the paste keystroke after choosing a clip (Ditto `SendPasteMessageAfterSelection`).
    pub paste_after_selection: bool,
    /// Separator between clips of a multi-clip paste (Ditto `MultiPasteSeparator` = `[LF]`).
    pub multi_paste_separator: String,
    /// Paste a multi-selection bottom-up, i.e. oldest first (Ditto `MultiPasteReverse` = TRUE).
    pub multi_paste_reverse: bool,
    /// Keep the window open when it loses focus.
    pub keep_open_on_blur: bool,
    pub theme: Theme,
    /// Size of the clip text in the list, in pixels.
    pub font_size: u32,
    /// "Interface size": how much both windows are zoomed, in percent (text, pictures, icons
    /// and spacing together). Not in Ditto, which relies on the Windows display scale.
    pub ui_zoom: u32,
    /// Size of the quick-paste window at 100 % interface size, in logical pixels.
    pub window_width: u32,
    pub window_height: u32,
    /// Last window position, used by [`PopupPosition::AtPreviousPosition`].
    pub last_position: Option<(i32, i32)>,
    pub search_mode: SearchMode,
    pub search_scopes: SearchScopes,
    /// Delay between giving focus back to the target app and sending the paste keystroke.
    pub paste_delay_ms: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_on_login: false,
            limit_entries: true,
            max_entries: 500,
            expire_entries: false,
            expire_days: 5,
            allow_duplicates: false,
            description_chars: 500,
            max_clip_bytes: 0,
            capture: CaptureFormats::default(),
            copy_app_include: "*".into(),
            copy_app_exclude: String::new(),
            respect_privacy_markers: true,
            read_image_text: true,
            hotkey_activate: "Ctrl+Backquote".into(),
            hotkey_text_only_paste: String::new(),
            position_hotkeys: vec![String::new(); 10],
            position_hotkeys_send_paste: true,
            move_to_top_on_paste: true,
            hotkey_again_moves_selection: true,
            popup_position: PopupPosition::AtCursor,
            lines_per_clip: 2,
            show_quick_paste_numbers: true,
            show_leading_whitespace: false,
            show_thumbnails: true,
            find_as_you_type: true,
            prompt_on_delete: true,
            show_grouped_in_main: true,
            paste_after_selection: true,
            multi_paste_separator: "[LF]".into(),
            multi_paste_reverse: true,
            keep_open_on_blur: false,
            theme: Theme::System,
            font_size: 13,
            ui_zoom: 100,
            window_width: 420,
            window_height: 520,
            last_position: None,
            search_mode: SearchMode::Wildcard,
            search_scopes: SearchScopes::default(),
            paste_delay_ms: 120,
        }
    }
}

impl Settings {
    /// Load settings, falling back to defaults (with a warning) when missing or invalid.
    pub fn load(path: &Path) -> Settings {
        match std::fs::read_to_string(path) {
            Ok(s) => match serde_json::from_str::<Settings>(&s) {
                Ok(mut settings) => {
                    settings.normalize();
                    settings
                }
                Err(e) => {
                    log::warn!("ignoring invalid settings file {}: {e}", path.display());
                    Settings::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                log::warn!("cannot read settings file {}: {e}", path.display());
                Settings::default()
            }
        }
    }

    /// Save atomically (write a temp file, then rename over the old one).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Clamp values into sane ranges.
    pub fn normalize(&mut self) {
        self.position_hotkeys.resize(10, String::new());
        self.lines_per_clip = self.lines_per_clip.clamp(1, 5);
        self.description_chars = self.description_chars.clamp(16, 100_000);
        self.font_size = self.font_size.clamp(9, 24);
        self.ui_zoom = self.ui_zoom.clamp(*UI_ZOOM_RANGE.start(), *UI_ZOOM_RANGE.end());
        self.window_width = self.window_width.clamp(260, 6000);
        self.window_height = self.window_height.clamp(200, 6000);
        self.paste_delay_ms = self.paste_delay_ms.min(2000);
        self.expire_days = self.expire_days.max(1);
    }

    /// Whether a copy made in `app` should be saved (Ditto `CopyAppInclude`/`CopyAppExclude`).
    pub fn accepts_app(&self, app: Option<&str>) -> bool {
        let Some(app) = app.map(str::trim).filter(|a| !a.is_empty()) else {
            return true;
        };
        let matches_any = |list: &str| list.split(';').map(str::trim).filter(|p| !p.is_empty()).any(|p| wildcard_match(p, app));
        let include = self.copy_app_include.trim();
        let included = include.is_empty() || matches_any(include);
        included && !matches_any(&self.copy_app_exclude)
    }
}

/// Case-insensitive `*`/`?` wildcard match of a whole string.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("keepass*", "KeePassXC"));
        assert!(wildcard_match("*.exe", "chrome.exe"));
        assert!(!wildcard_match("c?ome.exe", "chrome.exe"));
        assert!(wildcard_match("ch?ome.exe", "chrome.exe"));
        assert!(!wildcard_match("chrome", "chrome.exe"));
    }

    #[test]
    fn interface_size_is_kept_in_range() {
        assert_eq!(Settings::default().ui_zoom, 100);
        for (given, kept) in [(0, 50), (49, 50), (50, 50), (150, 150), (300, 300), (301, 300), (5000, 300)] {
            let mut s = Settings { ui_zoom: given, ..Default::default() };
            s.normalize();
            assert_eq!(s.ui_zoom, kept, "{given}");
        }
        // The window size is kept at 100 %, so it does not depend on the interface size.
        let mut s = Settings { window_width: 2400, window_height: 3000, ui_zoom: 300, ..Default::default() };
        s.normalize();
        assert_eq!((s.window_width, s.window_height), (2400, 3000));
    }

    #[test]
    fn app_filters() {
        let mut s = Settings::default();
        assert!(s.accepts_app(Some("Safari")));
        assert!(s.accepts_app(None));
        s.copy_app_exclude = "1Password*; keepass*".into();
        assert!(!s.accepts_app(Some("1Password 7")));
        assert!(!s.accepts_app(Some("KeePassXC")));
        assert!(s.accepts_app(Some("Terminal")));
        s.copy_app_include = "Terminal;Code".into();
        assert!(s.accepts_app(Some("code")));
        assert!(!s.accepts_app(Some("Safari")));
    }

    #[test]
    fn roundtrip_and_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let mut s = Settings { max_entries: 42, ..Settings::default() };
        s.position_hotkeys[3] = "Ctrl+Alt+4".into();
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        // Unknown and missing keys are tolerated; out-of-range values are clamped.
        std::fs::write(&path, r#"{"max_entries": 7, "lines_per_clip": 99, "something_new": true}"#).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.max_entries, 7);
        assert_eq!(loaded.lines_per_clip, 5);
        assert_eq!(loaded.hotkey_activate, "Ctrl+Backquote");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }
}
