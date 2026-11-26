//! Import an existing Ditto database (`Ditto.db`, SQLite) into the Ditto-RS history.
//!
//! Ditto's schema (DatabaseUtilities.cpp, `CreateDB`):
//!
//! ```sql
//! CREATE TABLE Main(lID INTEGER PRIMARY KEY AUTOINCREMENT, lDate INTEGER, mText TEXT,
//!   lShortCut INTEGER, lDontAutoDelete INTEGER, CRC INTEGER, bIsGroup INTEGER,
//!   lParentID INTEGER, QuickPasteText TEXT, clipOrder REAL, clipGroupOrder REAL,
//!   globalShortCut INTEGER, lastPasteDate INTEGER, stickyClipOrder REAL,
//!   stickyClipGroupOrder REAL, MoveToGroupShortCut INTEGER, GlobalMoveToGroupShortCut INTEGER);
//! CREATE TABLE Data(lID INTEGER PRIMARY KEY AUTOINCREMENT, lParentID INTEGER,
//!   strClipBoardFormat TEXT, ooData BLOB);
//! ```
//!
//! `Data.ooData` holds the raw Windows clipboard payloads, which we convert to portable
//! formats: `CF_UNICODETEXT` (UTF-16LE), `CF_TEXT` (ANSI), `HTML Format` (CF_HTML),
//! `Rich Text Format`, `PNG`, `CF_DIB`/`CF_DIBV5` (converted to PNG) and `CF_HDROP` (file list).
//! Older databases may lack some columns; missing ones are treated as NULL.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::model::{ClipContent, ClipKind, formats};
use crate::store::{AddOptions, AddOutcome, DuplicatePolicy, ListScope, StickyOp, Store};
use crate::{Error, Result, imaging, text};

/// Ditto's "not sticky" marker.
const INVALID_STICKY: f64 = -2_147_483_647.0;
const MAX_REPORTED_ERRORS: usize = 20;

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub description_chars: usize,
    pub import_groups: bool,
    pub import_hotkeys: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self { description_chars: 500, import_groups: true, import_hotkeys: true }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    pub clips_imported: usize,
    pub duplicates_skipped: usize,
    pub groups_imported: usize,
    /// Clips with no format we can use (e.g. only private application formats).
    pub unsupported_skipped: usize,
    pub errors: Vec<String>,
}

impl ImportReport {
    fn error(&mut self, e: impl std::fmt::Display) {
        if self.errors.len() < MAX_REPORTED_ERRORS {
            self.errors.push(e.to_string());
        }
    }
}

struct MainRow {
    id: i64,
    date: Option<i64>,
    text: Option<String>,
    shortcut: Option<i64>,
    dont_auto_delete: Option<i64>,
    parent: Option<i64>,
    quick_paste: Option<String>,
    global_shortcut: Option<i64>,
    last_paste: Option<i64>,
    sticky: Option<f64>,
}

pub fn import_ditto_db(path: &Path, store: &mut Store, opts: &ImportOptions) -> Result<ImportReport> {
    let src = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let columns = table_columns(&src, "Main")?;
    if !columns.contains("lid") || table_columns(&src, "Data")?.is_empty() {
        return Err(Error::Invalid(format!("{} is not a Ditto database", path.display())));
    }
    let col = |name: &str| -> String {
        if columns.contains(&name.to_ascii_lowercase()) { name.to_string() } else { format!("NULL AS {name}") }
    };
    let select = format!(
        "SELECT lID, {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM Main",
        col("lDate"),
        col("mText"),
        col("lShortCut"),
        col("lDontAutoDelete"),
        col("lParentID"),
        col("QuickPasteText"),
        col("globalShortCut"),
        col("lastPasteDate"),
        col("stickyClipOrder"),
        col("bIsGroup"),
    );
    let order = if columns.contains("cliporder") { "COALESCE(clipOrder, lID)" } else { "lID" };

    let mut report = ImportReport::default();
    let mut rows: Vec<(MainRow, bool)> = Vec::new();
    {
        let mut stmt = src.prepare(&format!("{select} ORDER BY {order} ASC, lID ASC"))?;
        let mut q = stmt.query([])?;
        while let Some(r) = q.next()? {
            let is_group = r.get::<_, Option<i64>>(10)?.unwrap_or(0) != 0;
            rows.push((
                MainRow {
                    id: r.get(0)?,
                    date: r.get(1)?,
                    text: text_column(r, 2)?,
                    shortcut: r.get(3)?,
                    dont_auto_delete: r.get(4)?,
                    parent: r.get(5)?,
                    quick_paste: text_column(r, 6)?,
                    global_shortcut: r.get(7)?,
                    last_paste: r.get(8)?,
                    sticky: r.get(9)?,
                },
                is_group,
            ));
        }
    }

    // Groups first, parents before children.
    let mut group_map: HashMap<i64, i64> = HashMap::new();
    if opts.import_groups {
        let mut pending: Vec<&MainRow> = rows.iter().filter(|(_, g)| *g).map(|(r, _)| r).collect();
        let group_ids: HashSet<i64> = pending.iter().map(|g| g.id).collect();
        while !pending.is_empty() {
            let before = pending.len();
            let mut i = 0;
            while i < pending.len() {
                let g = pending[i];
                let parent = g.parent.filter(|p| *p > 0 && group_ids.contains(p));
                let ready = match parent {
                    None => Some(None),
                    Some(p) => group_map.get(&p).map(|&mapped| Some(mapped)),
                };
                if let Some(parent) = ready {
                    let name = g.text.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Group {}", g.id));
                    match store.create_group(&name, parent) {
                        Ok(new_id) => {
                            group_map.insert(g.id, new_id);
                            report.groups_imported += 1;
                        }
                        Err(e) => report.error(format!("group {}: {e}", g.id)),
                    }
                    pending.swap_remove(i);
                } else {
                    i += 1;
                }
            }
            if pending.len() == before {
                // A parent cycle: attach the rest to the top level.
                for g in pending.drain(..) {
                    let name = g.text.clone().unwrap_or_else(|| format!("Group {}", g.id));
                    if let Ok(new_id) = store.create_group(&name, None) {
                        group_map.insert(g.id, new_id);
                        report.groups_imported += 1;
                    }
                }
            }
        }
    }

    let mut data_stmt = src.prepare("SELECT strClipBoardFormat, ooData FROM Data WHERE lParentID = ?1 ORDER BY lID")?;
    let mut sticky: Vec<(i64, f64)> = Vec::new();
    for (row, is_group) in &rows {
        if *is_group {
            continue;
        }
        let data: Vec<(String, Vec<u8>)> = data_stmt
            .query_map([row.id], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<Vec<u8>>>(1)?.unwrap_or_default())))?
            .collect::<rusqlite::Result<_>>()?;
        let content = convert_formats(&data, &mut report);
        if content.is_empty() {
            report.unsupported_skipped += 1;
            continue;
        }
        let created = row.date.filter(|d| *d > 0).map(|d| d * 1000);
        let description = match content.kind() {
            ClipKind::Image => None,
            _ => row.text.as_deref().map(|t| text::truncate_chars(t, opts.description_chars).to_string()),
        };
        let add = AddOptions {
            duplicates: DuplicatePolicy::Skip,
            description_chars: opts.description_chars,
            source_app: None,
            group_id: row.parent.and_then(|p| group_map.get(&p).copied()),
            created_at: created,
            last_used_at: row.last_paste.filter(|d| *d > 0).map(|d| d * 1000).or(created),
            description,
            never_auto_delete: row.dont_auto_delete.unwrap_or(0) != 0,
            quick_paste_text: row.quick_paste.clone(),
            count_as_copy: false,
        };
        match store.add_clip(&content, &add) {
            Ok(AddOutcome::Inserted(id)) => {
                report.clips_imported += 1;
                if let Some(order) = row.sticky.filter(|s| *s != INVALID_STICKY) {
                    sticky.push((id, order));
                }
                if opts.import_hotkeys && row.global_shortcut.unwrap_or(0) != 0 {
                    if let Some(hotkey) = row.shortcut.and_then(|s| hotkey_from_windows(s as u32)) {
                        if let Err(e) = store.set_hotkey(id, Some(&hotkey)) {
                            report.error(format!("hotkey of clip {}: {e}", row.id));
                        }
                    }
                }
            }
            Ok(AddOutcome::Duplicate(_)) => report.duplicates_skipped += 1,
            Err(e) => report.error(format!("clip {}: {e}", row.id)),
        }
    }

    // Re-create sticky order: lowest first, so the highest ends on top.
    sticky.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (id, _) in sticky {
        if let Err(e) = store.set_sticky(id, StickyOp::Top, ListScope::Main) {
            report.error(e);
        }
    }
    Ok(report)
}

fn text_column(r: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<String>> {
    use rusqlite::types::ValueRef;
    Ok(match r.get_ref(idx)? {
        ValueRef::Text(t) | ValueRef::Blob(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Null => None,
    })
}

fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.into_iter().map(|n| n.to_ascii_lowercase()).collect())
}

/// Convert Ditto's Windows clipboard payloads to portable formats.
pub fn convert_formats(data: &[(String, Vec<u8>)], report: &mut ImportReport) -> ClipContent {
    let mut content = ClipContent::new();
    let mut ansi_text: Option<String> = None;
    let mut dib: Option<&[u8]> = None;
    for (name, bytes) in data {
        match name.as_str() {
            "CF_UNICODETEXT" => content.set(formats::TEXT, utf16le_to_string(bytes).into_bytes()),
            "CF_TEXT" | "CF_OEMTEXT" => {
                if ansi_text.is_none() {
                    ansi_text = Some(text::decode_cp1252(trim_nul(bytes)));
                }
            }
            "HTML Format" => {
                let html = String::from_utf8_lossy(trim_nul(bytes));
                content.set(formats::HTML, text::strip_cf_html_header(&html).as_bytes().to_vec());
            }
            "Rich Text Format" => content.set(formats::RTF, String::from_utf8_lossy(trim_nul(bytes)).into_owned().into_bytes()),
            "PNG" => content.set(formats::PNG, bytes.clone()),
            "CF_DIBV5" => dib = Some(bytes.as_slice()),
            "CF_DIB" => dib = dib.or(Some(bytes.as_slice())),
            "CF_HDROP" => {
                let files = parse_hdrop(bytes);
                if !files.is_empty() {
                    content.set_files(&files);
                }
            }
            _ => {}
        }
    }
    if !content.has(formats::TEXT) {
        if let Some(t) = ansi_text {
            content.set(formats::TEXT, t.into_bytes());
        }
    }
    if !content.has(formats::PNG) {
        if let Some(d) = dib {
            match imaging::dib_to_png(d) {
                Ok(png) => content.set(formats::PNG, png),
                Err(e) => report.error(format!("image conversion: {e}")),
            }
        }
    }
    content
}

fn trim_nul(b: &[u8]) -> &[u8] {
    let end = b.iter().rposition(|&c| c != 0).map_or(0, |p| p + 1);
    &b[..end]
}

fn utf16le_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// Parse a `DROPFILES` structure followed by a double-NUL-terminated path list.
pub fn parse_hdrop(bytes: &[u8]) -> Vec<String> {
    if bytes.len() < 20 {
        return Vec::new();
    }
    let offset = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let wide = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) != 0;
    let Some(list) = bytes.get(offset..) else { return Vec::new() };
    if wide {
        let units: Vec<u16> = list.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        units.split(|&u| u == 0).take_while(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
    } else {
        list.split(|&b| b == 0).take_while(|s| !s.is_empty()).map(text::decode_cp1252).collect()
    }
}

/// Convert a Ditto/Win32 hotkey value (low byte virtual key, high byte `HOTKEYF_*`
/// modifiers; `HOTKEYF_EXT` is used by Ditto for the Windows key) to our accelerator syntax.
pub fn hotkey_from_windows(value: u32) -> Option<String> {
    let vk = value & 0xff;
    let mods = (value >> 8) & 0xff;
    let key = match vk {
        0x30..=0x39 => format!("Digit{}", vk - 0x30),
        0x41..=0x5a => format!("Key{}", char::from_u32(vk)?),
        0x60..=0x69 => format!("Numpad{}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6f),
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0d => "Enter".into(),
        0x20 => "Space".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "ArrowLeft".into(),
        0x26 => "ArrowUp".into(),
        0x27 => "ArrowRight".into(),
        0x28 => "ArrowDown".into(),
        0x2d => "Insert".into(),
        0x2e => "Delete".into(),
        0xba => "Semicolon".into(),
        0xbb => "Equal".into(),
        0xbc => "Comma".into(),
        0xbd => "Minus".into(),
        0xbe => "Period".into(),
        0xbf => "Slash".into(),
        0xc0 => "Backquote".into(),
        0xdb => "BracketLeft".into(),
        0xdc => "Backslash".into(),
        0xdd => "BracketRight".into(),
        0xde => "Quote".into(),
        _ => return None,
    };
    let mut parts = Vec::new();
    if mods & 0x02 != 0 {
        parts.push("Ctrl");
    }
    if mods & 0x04 != 0 {
        parts.push("Alt");
    }
    if mods & 0x01 != 0 {
        parts.push("Shift");
    }
    if mods & 0x08 != 0 {
        parts.push("Super");
    }
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(&key);
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ListQuery;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
    }

    fn hdrop(files: &[&str]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&20u32.to_le_bytes()); // pFiles
        b.extend_from_slice(&[0; 8]); // pt
        b.extend_from_slice(&0u32.to_le_bytes()); // fNC
        b.extend_from_slice(&1u32.to_le_bytes()); // fWide
        for f in files {
            b.extend(f.encode_utf16().flat_map(u16::to_le_bytes));
            b.extend_from_slice(&[0, 0]);
        }
        b.extend_from_slice(&[0, 0]);
        b
    }

    fn dib_1x1_red() -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0u8; 24]);
        dib.extend_from_slice(&[0, 0, 255, 0]); // BGR + row padding
        dib
    }

    /// A database with Ditto's real schema and a representative set of clips.
    fn make_ditto_db(path: &Path) {
        let db = Connection::open(path).unwrap();
        db.execute_batch(
            "CREATE TABLE Main(lID INTEGER PRIMARY KEY AUTOINCREMENT, lDate INTEGER, mText TEXT, lShortCut INTEGER,
                 lDontAutoDelete INTEGER, CRC INTEGER, bIsGroup INTEGER, lParentID INTEGER, QuickPasteText TEXT,
                 clipOrder REAL, clipGroupOrder REAL, globalShortCut INTEGER, lastPasteDate INTEGER,
                 stickyClipOrder REAL, stickyClipGroupOrder REAL, MoveToGroupShortCut INTEGER, GlobalMoveToGroupShortCut INTEGER);
             CREATE TABLE Data(lID INTEGER PRIMARY KEY AUTOINCREMENT, lParentID INTEGER, strClipBoardFormat TEXT, ooData BLOB);",
        )
        .unwrap();
        let main = |id: i64, text: &str, order: f64, group: bool, parent: i64, extra: (i64, i64, i64, f64, &str)| {
            let (shortcut, global, dont_delete, sticky, qp) = extra;
            db.execute(
                "INSERT INTO Main VALUES (?1, 1700000000, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, 0, ?9, 1700000100, ?10, -2147483647, 0, 0)",
                rusqlite::params![id, text, shortcut, dont_delete, group as i64, parent, qp, order, global, sticky],
            )
            .unwrap();
        };
        let data = |parent: i64, fmt: &str, bytes: Vec<u8>| {
            db.execute("INSERT INTO Data (lParentID, strClipBoardFormat, ooData) VALUES (?1, ?2, ?3)", rusqlite::params![parent, fmt, bytes])
                .unwrap();
        };
        const NONE: (i64, i64, i64, f64, &str) = (0, 0, 0, INVALID_STICKY, "");
        main(1, "Work", 1.0, true, -1, NONE);
        main(2, "Snippets", 2.0, true, 1, NONE);
        // Ctrl (HOTKEYF_CONTROL = 0x02) + '1' (0x31), registered globally.
        main(10, "Hello from Ditto", 10.0, false, -1, ((0x02 << 8) | 0x31, 1, 1, INVALID_STICKY, "hi"));
        data(10, "CF_UNICODETEXT", utf16("Hello from Ditto"));
        data(10, "CF_TEXT", b"Hello from Ditto\0".to_vec());
        main(11, "Bold", 11.0, false, 2, NONE);
        data(11, "CF_UNICODETEXT", utf16("Bold"));
        let frag = "<b>Bold</b>";
        data(11, "HTML Format", format!("Version:0.9\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:-1\r\nEndFragment:-1\r\n<html><body><!--StartFragment-->{frag}<!--EndFragment--></body></html>").into_bytes());
        data(11, "Rich Text Format", b"{\\rtf1 \\b Bold\\b0}\0".to_vec());
        main(12, "CF_DIB", 12.0, false, -1, (0, 0, 0, 5.0, ""));
        data(12, "CF_DIB", dib_1x1_red());
        main(13, "C:\\a.txt", 13.0, false, -1, NONE);
        data(13, "CF_HDROP", hdrop(&["C:\\a.txt", "C:\\dir\\b.txt"]));
        main(14, "private", 14.0, false, -1, NONE);
        data(14, "Some App Private Format", vec![1, 2, 3]);
        main(15, "Ansi only é", 9.0, false, -1, NONE);
        data(15, "CF_TEXT", vec![b'A', b'n', b's', b'i', b' ', b'o', b'n', b'l', b'y', b' ', 0xe9, 0]);
    }

    #[test]
    fn imports_a_ditto_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Ditto.db");
        make_ditto_db(&path);
        let mut store = Store::open_in_memory().unwrap();
        let report = import_ditto_db(&path, &mut store, &ImportOptions::default()).unwrap();
        assert_eq!(report.groups_imported, 2, "{report:?}");
        assert_eq!(report.clips_imported, 5, "{report:?}");
        assert_eq!(report.unsupported_skipped, 1);
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        let page = store.list(&ListQuery { include_grouped: true, limit: 50, ..Default::default() }).unwrap();
        let descs: Vec<&str> = page.items.iter().map(|c| c.description.as_str()).collect();
        // Sticky image first, then by Ditto's clipOrder (newest first).
        assert_eq!(descs, vec!["Image 1 × 1", "C:\\a.txt", "Bold", "Hello from Ditto", "Ansi only é"]);

        let hello = &page.items[3];
        assert_eq!(hello.hotkey.as_deref(), Some("Ctrl+Digit1"));
        assert!(hello.never_auto_delete);
        assert_eq!(hello.quick_paste_text.as_deref(), Some("hi"));
        assert_eq!(hello.created_at, 1_700_000_000_000);
        assert_eq!(hello.last_used_at, 1_700_000_100_000);

        let bold = &page.items[2];
        let content = store.content(bold.id).unwrap();
        assert_eq!(content.html(), Some("<b>Bold</b>"));
        assert_eq!(content.rtf(), Some("{\\rtf1 \\b Bold\\b0}"));
        let groups = store.groups().unwrap();
        let snippets = groups.iter().find(|g| g.name == "Snippets").unwrap();
        let work = groups.iter().find(|g| g.name == "Work").unwrap();
        assert_eq!(snippets.parent_id, Some(work.id));
        assert_eq!(bold.group_id, Some(snippets.id));

        let image = store.content(page.items[0].id).unwrap();
        assert!(image.png().is_some());
        assert!(page.items[0].sticky);
        assert_eq!(store.content(page.items[1].id).unwrap().files(), vec!["C:\\a.txt", "C:\\dir\\b.txt"]);

        // Importing again only finds duplicates.
        let again = import_ditto_db(&path, &mut store, &ImportOptions { import_groups: false, ..Default::default() }).unwrap();
        assert_eq!(again.clips_imported, 0);
        assert_eq!(again.duplicates_skipped, 5);
    }

    #[test]
    fn rejects_other_databases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.db");
        Connection::open(&path).unwrap().execute_batch("CREATE TABLE t(x)").unwrap();
        let mut store = Store::open_in_memory().unwrap();
        assert!(matches!(import_ditto_db(&path, &mut store, &ImportOptions::default()), Err(Error::Invalid(_))));
    }

    #[test]
    fn hotkey_conversion() {
        assert_eq!(hotkey_from_windows(704).as_deref(), Some("Ctrl+Backquote"));
        assert_eq!(hotkey_from_windows((0x01 | 0x08) << 8 | 0x70).as_deref(), Some("Shift+Super+F1"));
        assert_eq!(hotkey_from_windows(0x41).as_deref(), Some("KeyA"));
        assert_eq!(hotkey_from_windows(0xff), None);
    }
}
