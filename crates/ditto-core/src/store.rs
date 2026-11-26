//! The SQLite clip history.
//!
//! The schema mirrors Ditto's `Main`/`Data` tables (DatabaseUtilities.cpp) with clearer
//! names: `clips` ≈ `Main`, `clip_data` ≈ `Data`, and groups get their own table instead of
//! `Main.bIsGroup` rows. Ordering works like Ditto: a `REAL` order key where the largest
//! value is the top of the list (`clipOrder`/`clipGroupOrder`), and nullable sticky keys
//! (`stickyClipOrder`/`stickyClipGroupOrder`; Ditto uses `-2147483647` for "not sticky").

use std::path::Path;
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::model::{ClipContent, ClipKind, ClipSummary, Group, formats};
use crate::search::{CompiledSearch, SearchFields};
use crate::{Error, Result, imaging, now_ms};

pub const SCHEMA_VERSION: i64 = 1;
/// Thumbnail bounds (pixels). The list draws them at half size for HiDPI screens.
pub const THUMB_MAX_W: u32 = 480;
pub const THUMB_MAX_H: u32 = 160;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS groups (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    parent_id  INTEGER REFERENCES groups(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS clips (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at         INTEGER NOT NULL,
    last_used_at       INTEGER NOT NULL,
    description        TEXT    NOT NULL,
    kind               TEXT    NOT NULL,
    formats            TEXT    NOT NULL,
    hash               INTEGER NOT NULL,
    size_bytes         INTEGER NOT NULL,
    source_app         TEXT,
    clip_order         REAL    NOT NULL,
    group_id           INTEGER REFERENCES groups(id) ON DELETE SET NULL,
    group_order        REAL    NOT NULL DEFAULT 0,
    sticky_order       REAL,
    group_sticky_order REAL,
    never_auto_delete  INTEGER NOT NULL DEFAULT 0,
    quick_paste_text   TEXT,
    hotkey             TEXT,
    thumbnail          BLOB
);
CREATE TABLE IF NOT EXISTS clip_data (
    clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
    format  TEXT    NOT NULL,
    data    BLOB    NOT NULL,
    PRIMARY KEY (clip_id, format)
);
CREATE INDEX IF NOT EXISTS clips_main_order  ON clips(sticky_order DESC, clip_order DESC);
CREATE INDEX IF NOT EXISTS clips_group_order ON clips(group_id, group_sticky_order DESC, group_order DESC);
CREATE INDEX IF NOT EXISTS clips_hash        ON clips(hash);
CREATE INDEX IF NOT EXISTS clips_hotkey      ON clips(hotkey) WHERE hotkey IS NOT NULL;
CREATE INDEX IF NOT EXISTS groups_parent     ON groups(parent_id);
"#;

/// Clips that auto-delete may remove (Ditto `RemoveOldEntries`: no shortcut, not "never
/// auto delete", not in a group, not sticky).
const UNPROTECTED: &str = "hotkey IS NULL AND never_auto_delete = 0 AND group_id IS NULL \
                           AND sticky_order IS NULL AND group_sticky_order IS NULL";

const SUMMARY_COLUMNS: &str = "c.id, c.description, c.kind, c.created_at, c.last_used_at, c.group_id, \
     c.sticky_order IS NOT NULL, c.group_sticky_order IS NOT NULL, c.never_auto_delete, \
     c.quick_paste_text, c.hotkey, c.source_app, c.size_bytes, c.formats, c.thumbnail IS NOT NULL";

/// What happens when a copy is identical to a clip that is already saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DuplicatePolicy {
    /// Move the existing clip to the top (Ditto's default).
    #[default]
    MoveToTop,
    /// Save duplicates, except an immediate repeat of the newest clip (Ditto `AllowDuplicates`).
    Allow,
    /// Leave the existing clip untouched (imports).
    Skip,
}

#[derive(Debug, Clone)]
pub struct AddOptions {
    pub duplicates: DuplicatePolicy,
    pub description_chars: usize,
    pub source_app: Option<String>,
    pub group_id: Option<i64>,
    /// Defaults to now.
    pub created_at: Option<i64>,
    /// Defaults to `created_at`.
    pub last_used_at: Option<i64>,
    /// Overrides the generated description.
    pub description: Option<String>,
    pub never_auto_delete: bool,
    pub quick_paste_text: Option<String>,
    /// Count this add in the copy statistics.
    pub count_as_copy: bool,
}

impl Default for AddOptions {
    fn default() -> Self {
        Self {
            duplicates: DuplicatePolicy::MoveToTop,
            description_chars: 500,
            source_app: None,
            group_id: None,
            created_at: None,
            last_used_at: None,
            description: None,
            never_auto_delete: false,
            quick_paste_text: None,
            count_as_copy: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    Inserted(i64),
    /// The content already existed under this id.
    Duplicate(i64),
}

impl AddOutcome {
    pub fn id(self) -> i64 {
        match self {
            AddOutcome::Inserted(id) | AddOutcome::Duplicate(id) => id,
        }
    }
}

/// Which list is being looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum ListScope {
    #[default]
    Main,
    Group(i64),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ListQuery<'a> {
    pub scope: ListScope,
    /// In the main list, also show clips that belong to a group.
    pub include_grouped: bool,
    pub search: Option<&'a CompiledSearch>,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListPage {
    pub items: Vec<ClipSummary>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StickyOp {
    /// "Make Top Sticky Clip".
    Top,
    /// "Make Last Sticky Clip".
    Last,
    /// "Replace Top Sticky Clip": become the top sticky clip and un-stick the previous one.
    ReplaceTop,
    /// "Remove Sticky Setting".
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderOp {
    Top,
    Up,
    Down,
    Last,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub clips: i64,
    pub groups: i64,
    pub data_bytes: i64,
    pub db_bytes: i64,
    pub total_copies: i64,
    pub total_pastes: i64,
    /// When the statistics started (ms since the epoch).
    pub since: i64,
    pub oldest_clip: Option<i64>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        let version: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0)).optional()?;
        match version.as_deref().map(str::parse::<i64>) {
            None => {
                conn.execute("INSERT INTO meta(key, value) VALUES ('schema_version', ?1)", [SCHEMA_VERSION.to_string()])?;
                conn.execute("INSERT OR IGNORE INTO meta(key, value) VALUES ('stats_since', ?1)", [now_ms().to_string()])?;
            }
            Some(Ok(v)) if v > SCHEMA_VERSION => {
                return Err(Error::Invalid(format!(
                    "the database was created by a newer version (schema {v}, supported {SCHEMA_VERSION})"
                )));
            }
            _ => {}
        }
        Ok(Self { conn })
    }

    /// Raw connection access for advanced callers (backups, tests).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // ---- capture -------------------------------------------------------------------

    pub fn add_clip(&mut self, content: &ClipContent, opts: &AddOptions) -> Result<AddOutcome> {
        if content.is_empty() {
            return Err(Error::Invalid("nothing to save: the clip is empty".into()));
        }
        let hash = content.content_hash();
        if opts.count_as_copy {
            bump_counter(&self.conn, "total_copies")?;
        }

        match opts.duplicates {
            DuplicatePolicy::MoveToTop | DuplicatePolicy::Skip => {
                if let Some(id) = find_duplicate(&self.conn, hash, content)? {
                    if opts.duplicates == DuplicatePolicy::MoveToTop {
                        let order = next_order(&self.conn, None)?;
                        self.conn.execute("UPDATE clips SET clip_order = ?1 WHERE id = ?2", params![order, id])?;
                    }
                    return Ok(AddOutcome::Duplicate(id));
                }
            }
            DuplicatePolicy::Allow => {
                let top: Option<(i64, i64)> = self
                    .conn
                    .query_row("SELECT id, hash FROM clips ORDER BY clip_order DESC LIMIT 1", [], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()?;
                if let Some((id, top_hash)) = top {
                    if top_hash == hash && load_content(&self.conn, id)? == *content {
                        return Ok(AddOutcome::Duplicate(id));
                    }
                }
            }
        }

        let thumbnail = content.png().and_then(|png| match imaging::thumbnail_png(png, THUMB_MAX_W, THUMB_MAX_H) {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("could not create a thumbnail: {e}");
                None
            }
        });
        let created = opts.created_at.unwrap_or_else(now_ms);
        let last_used = opts.last_used_at.unwrap_or(created);
        let description = opts
            .description
            .clone()
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| content.description(opts.description_chars));
        let quick_paste = opts.quick_paste_text.clone().filter(|q| !q.trim().is_empty());

        let tx = self.conn.transaction()?;
        let order = next_order(&tx, None)?;
        let group_order = match opts.group_id {
            Some(g) => next_order(&tx, Some(g))?,
            None => 0.0,
        };
        tx.execute(
            "INSERT INTO clips (created_at, last_used_at, description, kind, formats, hash, size_bytes, source_app,
                                clip_order, group_id, group_order, never_auto_delete, quick_paste_text, thumbnail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                created,
                last_used,
                description,
                content.kind().as_str(),
                content.format_names().join(","),
                hash,
                content.total_size() as i64,
                opts.source_app,
                order,
                opts.group_id,
                group_order,
                opts.never_auto_delete,
                quick_paste,
                thumbnail,
            ],
        )?;
        let id = tx.last_insert_rowid();
        insert_formats(&tx, id, content)?;
        tx.commit()?;
        Ok(AddOutcome::Inserted(id))
    }

    // ---- reading -------------------------------------------------------------------

    pub fn list(&self, q: &ListQuery<'_>) -> Result<ListPage> {
        let limit = q.limit.max(1);
        let needs_text = q.search.is_some_and(CompiledSearch::needs_full_text);
        let mut sql = format!("SELECT {SUMMARY_COLUMNS}");
        if needs_text {
            sql.push_str(", (SELECT d.data FROM clip_data d WHERE d.clip_id = c.id AND d.format = 'text')");
        }
        sql.push_str(" FROM clips c");
        let mut args: Vec<Value> = Vec::new();
        match q.scope {
            ListScope::Main => {
                if !q.include_grouped {
                    sql.push_str(" WHERE c.group_id IS NULL");
                }
                sql.push_str(" ORDER BY c.sticky_order DESC, c.clip_order DESC");
            }
            ListScope::Group(g) => {
                sql.push_str(" WHERE c.group_id = ? ORDER BY c.group_sticky_order DESC, c.group_order DESC");
                args.push(Value::Integer(g));
            }
        }

        let Some(search) = q.search else {
            sql.push_str(" LIMIT ? OFFSET ?");
            args.push(Value::Integer(limit as i64 + 1));
            args.push(Value::Integer(q.offset as i64));
            let mut stmt = self.conn.prepare(&sql)?;
            let mut items = stmt.query_map(params_from_iter(args), summary_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            let has_more = items.len() > limit;
            items.truncate(limit);
            return Ok(ListPage { items, has_more });
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(args))?;
        let mut items = Vec::new();
        let mut skipped = 0usize;
        while let Some(row) = rows.next()? {
            let summary = summary_from_row(row)?;
            let full_text: Option<Vec<u8>> = if needs_text { row.get(15)? } else { None };
            let full_text = full_text.as_deref().map(String::from_utf8_lossy);
            let fields = SearchFields {
                description: &summary.description,
                quick_paste: summary.quick_paste_text.as_deref(),
                full_text: full_text.as_deref(),
            };
            if !search.matches(&fields) {
                continue;
            }
            if skipped < q.offset {
                skipped += 1;
                continue;
            }
            items.push(summary);
            if items.len() > limit {
                break;
            }
        }
        let has_more = items.len() > limit;
        items.truncate(limit);
        Ok(ListPage { items, has_more })
    }

    /// The id at `position` (0-based) of a list, for "paste position N" hotkeys.
    pub fn clip_at(&self, scope: ListScope, include_grouped: bool, position: usize) -> Result<Option<i64>> {
        let page = self.list(&ListQuery { scope, include_grouped, search: None, offset: position, limit: 1 })?;
        Ok(page.items.first().map(|c| c.id))
    }

    pub fn summary(&self, id: i64) -> Result<ClipSummary> {
        self.conn
            .query_row(&format!("SELECT {SUMMARY_COLUMNS} FROM clips c WHERE c.id = ?1"), [id], summary_from_row)
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("clip {id}")))
    }

    pub fn content(&self, id: i64) -> Result<ClipContent> {
        self.ensure_exists(id)?;
        load_content(&self.conn, id)
    }

    pub fn thumbnail(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT thumbnail FROM clips WHERE id = ?1", [id], |r| r.get::<_, Option<Vec<u8>>>(0))
            .optional()?
            .flatten())
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0))?)
    }

    fn ensure_exists(&self, id: i64) -> Result<()> {
        let found: Option<i64> = self.conn.query_row("SELECT id FROM clips WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        found.map(|_| ()).ok_or_else(|| Error::NotFound(format!("clip {id}")))
    }

    // ---- editing -------------------------------------------------------------------

    /// Record a paste (Ditto `lastPasteDate`) and optionally move the clip to the top
    /// (Ditto `UpdateTimeOnPaste`).
    pub fn mark_pasted(&mut self, id: i64, move_to_top: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE clips SET last_used_at = ?1 WHERE id = ?2", params![now_ms(), id])?;
        if move_to_top {
            let order = next_order(&tx, None)?;
            tx.execute("UPDATE clips SET clip_order = ?1 WHERE id = ?2", params![order, id])?;
            let group: Option<i64> =
                tx.query_row("SELECT group_id FROM clips WHERE id = ?1", [id], |r| r.get(0)).optional()?.flatten();
            if let Some(g) = group {
                let go = next_order(&tx, Some(g))?;
                tx.execute("UPDATE clips SET group_order = ?1 WHERE id = ?2", params![go, id])?;
            }
        }
        bump_counter(&tx, "total_pastes")?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_clips(&mut self, ids: &[i64]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        Ok(self.conn.execute(&format!("DELETE FROM clips WHERE id IN ({placeholders})"), params_from_iter(ids))?)
    }

    /// Replace a clip's text (Ditto "Edit Clip"). Rich formats are dropped because they
    /// would no longer match the edited text.
    pub fn update_text(&mut self, id: i64, text: &str, description_chars: usize) -> Result<()> {
        let mut content = self.content(id)?;
        content.set(formats::TEXT, text.as_bytes().to_vec());
        content.remove(formats::HTML);
        content.remove(formats::RTF);
        self.replace_content(id, &content, description_chars)
    }

    pub fn replace_content(&mut self, id: i64, content: &ClipContent, description_chars: usize) -> Result<()> {
        if content.is_empty() {
            return Err(Error::Invalid("a clip cannot be empty".into()));
        }
        self.ensure_exists(id)?;
        let thumbnail = content.png().and_then(|p| imaging::thumbnail_png(p, THUMB_MAX_W, THUMB_MAX_H).ok());
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM clip_data WHERE clip_id = ?1", [id])?;
        insert_formats(&tx, id, content)?;
        tx.execute(
            "UPDATE clips SET description = ?1, kind = ?2, formats = ?3, hash = ?4, size_bytes = ?5, thumbnail = ?6
             WHERE id = ?7",
            params![
                content.description(description_chars),
                content.kind().as_str(),
                content.format_names().join(","),
                content.content_hash(),
                content.total_size() as i64,
                thumbnail,
                id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Ditto "Item Title".
    pub fn set_description(&mut self, id: i64, description: &str) -> Result<()> {
        self.update_column(id, "description", Value::Text(description.to_string()))
    }

    pub fn set_never_auto_delete(&mut self, id: i64, never: bool) -> Result<()> {
        self.update_column(id, "never_auto_delete", Value::Integer(never.into()))
    }

    pub fn set_quick_paste_text(&mut self, id: i64, text: Option<&str>) -> Result<()> {
        let v = text.map(str::trim).filter(|t| !t.is_empty()).map_or(Value::Null, |t| Value::Text(t.to_string()));
        self.update_column(id, "quick_paste_text", v)
    }

    /// Assign a global hotkey to a clip; a hotkey belongs to at most one clip.
    pub fn set_hotkey(&mut self, id: i64, hotkey: Option<&str>) -> Result<()> {
        self.ensure_exists(id)?;
        let hotkey = hotkey.map(str::trim).filter(|h| !h.is_empty());
        let tx = self.conn.transaction()?;
        if let Some(h) = hotkey {
            tx.execute("UPDATE clips SET hotkey = NULL WHERE hotkey = ?1 AND id <> ?2", params![h, id])?;
        }
        tx.execute("UPDATE clips SET hotkey = ?1 WHERE id = ?2", params![hotkey, id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn hotkey_clips(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare("SELECT id, hotkey FROM clips WHERE hotkey IS NOT NULL ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn update_column(&mut self, id: i64, column: &str, value: Value) -> Result<()> {
        let n = self.conn.execute(&format!("UPDATE clips SET {column} = ?1 WHERE id = ?2"), params![value, id])?;
        if n == 0 {
            return Err(Error::NotFound(format!("clip {id}")));
        }
        Ok(())
    }

    pub fn set_sticky(&mut self, id: i64, op: StickyOp, scope: ListScope) -> Result<()> {
        self.ensure_exists(id)?;
        let (col, filter, args) = sticky_scope(scope);
        let tx = self.conn.transaction()?;
        let max_sql = format!("SELECT MAX({col}) FROM clips WHERE {col} IS NOT NULL{filter}");
        let min_sql = format!("SELECT MIN({col}) FROM clips WHERE {col} IS NOT NULL{filter}");
        let value: Option<f64> = match op {
            StickyOp::Top => Some(tx.query_row(&max_sql, params_from_iter(&args), |r| r.get::<_, Option<f64>>(0))?.unwrap_or(0.0) + 1.0),
            StickyOp::Last => Some(tx.query_row(&min_sql, params_from_iter(&args), |r| r.get::<_, Option<f64>>(0))?.unwrap_or(2.0) - 1.0),
            StickyOp::ReplaceTop => {
                let prev: Option<i64> = tx
                    .query_row(
                        &format!("SELECT id FROM clips WHERE {col} IS NOT NULL{filter} ORDER BY {col} DESC LIMIT 1"),
                        params_from_iter(&args),
                        |r| r.get(0),
                    )
                    .optional()?;
                let top = tx.query_row(&max_sql, params_from_iter(&args), |r| r.get::<_, Option<f64>>(0))?.unwrap_or(0.0) + 1.0;
                if let Some(prev) = prev.filter(|&p| p != id) {
                    tx.execute(&format!("UPDATE clips SET {col} = NULL WHERE id = ?1"), [prev])?;
                }
                Some(top)
            }
            StickyOp::Remove => None,
        };
        tx.execute(&format!("UPDATE clips SET {col} = ?1 WHERE id = ?2"), params![value, id])?;
        tx.commit()?;
        Ok(())
    }

    /// Ditto's "Clip Order" menu, applied within the list given by `scope`.
    pub fn move_clip(&mut self, id: i64, op: OrderOp, scope: ListScope, include_grouped: bool) -> Result<()> {
        self.ensure_exists(id)?;
        let (sticky_col, _, _) = sticky_scope(scope);
        let tx = self.conn.transaction()?;
        let sticky: Option<f64> =
            tx.query_row(&format!("SELECT {sticky_col} FROM clips WHERE id = ?1"), [id], |r| r.get(0))?;
        let (col, current, mut filter, mut args) = match sticky {
            Some(v) => (sticky_col, v, format!("{sticky_col} IS NOT NULL"), Vec::<Value>::new()),
            None => {
                let col = if matches!(scope, ListScope::Group(_)) { "group_order" } else { "clip_order" };
                let v: f64 = tx.query_row(&format!("SELECT {col} FROM clips WHERE id = ?1"), [id], |r| r.get(0))?;
                (col, v, format!("{sticky_col} IS NULL"), Vec::new())
            }
        };
        match scope {
            ListScope::Main if !include_grouped => filter.push_str(" AND group_id IS NULL"),
            ListScope::Main => {}
            ListScope::Group(g) => {
                filter.push_str(" AND group_id = ?");
                args.push(Value::Integer(g));
            }
        }
        let new_value = match op {
            OrderOp::Top => {
                let max: Option<f64> = tx.query_row(&format!("SELECT MAX({col}) FROM clips WHERE {filter}"), params_from_iter(&args), |r| r.get(0))?;
                Some(max.unwrap_or(current) + 1.0)
            }
            OrderOp::Last => {
                let min: Option<f64> = tx.query_row(&format!("SELECT MIN({col}) FROM clips WHERE {filter}"), params_from_iter(&args), |r| r.get(0))?;
                Some(min.unwrap_or(current) - 1.0)
            }
            OrderOp::Up | OrderOp::Down => {
                let (cmp, dir) = if op == OrderOp::Up { (">", "ASC") } else { ("<", "DESC") };
                let mut neighbour_args = args.clone();
                neighbour_args.push(Value::Real(current));
                let neighbour: Option<(i64, f64)> = tx
                    .query_row(
                        &format!("SELECT id, {col} FROM clips WHERE {filter} AND {col} {cmp} ? ORDER BY {col} {dir} LIMIT 1"),
                        params_from_iter(&neighbour_args),
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                match neighbour {
                    Some((other, other_value)) => {
                        tx.execute(&format!("UPDATE clips SET {col} = ?1 WHERE id = ?2"), params![current, other])?;
                        Some(other_value)
                    }
                    None => None,
                }
            }
        };
        if let Some(v) = new_value {
            tx.execute(&format!("UPDATE clips SET {col} = ?1 WHERE id = ?2"), params![v, id])?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- groups --------------------------------------------------------------------

    pub fn create_group(&mut self, name: &str, parent: Option<i64>) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("a group needs a name".into()));
        }
        if let Some(p) = parent {
            self.ensure_group(p)?;
        }
        self.conn.execute(
            "INSERT INTO groups (name, parent_id, created_at) VALUES (?1, ?2, ?3)",
            params![name, parent, now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_group(&mut self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("a group needs a name".into()));
        }
        let n = self.conn.execute("UPDATE groups SET name = ?1 WHERE id = ?2", params![name, id])?;
        if n == 0 {
            return Err(Error::NotFound(format!("group {id}")));
        }
        Ok(())
    }

    /// Delete a group and its sub-groups. Their clips are deleted too when `delete_clips`,
    /// otherwise they go back to the main list.
    pub fn delete_group(&mut self, id: i64, delete_clips: bool) -> Result<()> {
        self.ensure_group(id)?;
        let tx = self.conn.transaction()?;
        let descendants = "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL SELECT g.id FROM groups g JOIN sub ON g.parent_id = sub.id)";
        if delete_clips {
            tx.execute(&format!("{descendants} DELETE FROM clips WHERE group_id IN (SELECT id FROM sub)"), [id])?;
        } else {
            tx.execute(
                &format!("{descendants} UPDATE clips SET group_id = NULL, group_order = 0, group_sticky_order = NULL WHERE group_id IN (SELECT id FROM sub)"),
                [id],
            )?;
        }
        tx.execute("DELETE FROM groups WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn groups(&self) -> Result<Vec<Group>> {
        let mut stmt = self.conn.prepare(
            "SELECT g.id, g.name, g.parent_id, (SELECT COUNT(*) FROM clips c WHERE c.group_id = g.id)
             FROM groups g ORDER BY g.name COLLATE NOCASE, g.id",
        )?;
        let rows = stmt.query_map([], |r| Ok(Group { id: r.get(0)?, name: r.get(1)?, parent_id: r.get(2)?, clip_count: r.get(3)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Move clips into a group (`None` = back to the main list), keeping their relative order.
    pub fn move_to_group(&mut self, ids: &[i64], group: Option<i64>) -> Result<()> {
        if let Some(g) = group {
            self.ensure_group(g)?;
        }
        let tx = self.conn.transaction()?;
        // Insert oldest first so the newest ends up on top of the group.
        let mut ordered: Vec<(i64, f64)> = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(order) = tx.query_row("SELECT clip_order FROM clips WHERE id = ?1", [id], |r| r.get::<_, f64>(0)).optional()? {
                ordered.push((id, order));
            }
        }
        ordered.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (id, _) in ordered {
            let group_order = match group {
                Some(g) => next_order(&tx, Some(g))?,
                None => 0.0,
            };
            tx.execute(
                "UPDATE clips SET group_id = ?1, group_order = ?2, group_sticky_order = NULL WHERE id = ?3",
                params![group, group_order, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn ensure_group(&self, id: i64) -> Result<()> {
        let found: Option<i64> = self.conn.query_row("SELECT id FROM groups WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        found.map(|_| ()).ok_or_else(|| Error::NotFound(format!("group {id}")))
    }

    // ---- maintenance ---------------------------------------------------------------

    /// Ditto's `RemoveOldEntries`: keep at most `max_entries` clips and drop clips unused
    /// for `expire_days`, never touching protected clips. Returns how many were deleted.
    pub fn enforce_retention(&mut self, max_entries: Option<u32>, expire_days: Option<u32>) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut deleted = 0;
        if let Some(max) = max_entries {
            deleted += tx.execute(
                &format!(
                    "DELETE FROM clips WHERE id IN (SELECT id FROM clips ORDER BY clip_order DESC LIMIT -1 OFFSET ?1) AND {UNPROTECTED}"
                ),
                [i64::from(max)],
            )?;
        }
        if let Some(days) = expire_days.filter(|d| *d > 0) {
            let cutoff = now_ms() - i64::from(days) * 86_400_000;
            deleted += tx.execute(&format!("DELETE FROM clips WHERE last_used_at < ?1 AND {UNPROTECTED}"), [cutoff])?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    /// Delete every clip that auto-delete could delete (keeps grouped, sticky, hotkey and
    /// "never auto delete" clips). Returns the number of deleted clips.
    pub fn clear_history(&mut self) -> Result<usize> {
        Ok(self.conn.execute(&format!("DELETE FROM clips WHERE {UNPROTECTED}"), [])?)
    }

    pub fn stats(&self) -> Result<Stats> {
        let counter = |key: &str| -> Result<i64> {
            Ok(self
                .conn
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get::<_, String>(0))
                .optional()?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0))
        };
        let page_count: i64 = self.conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
        let page_size: i64 = self.conn.pragma_query_value(None, "page_size", |r| r.get(0))?;
        Ok(Stats {
            clips: self.count()?,
            groups: self.conn.query_row("SELECT COUNT(*) FROM groups", [], |r| r.get(0))?,
            data_bytes: self.conn.query_row("SELECT COALESCE(SUM(size_bytes), 0) FROM clips", [], |r| r.get(0))?,
            db_bytes: page_count * page_size,
            total_copies: counter("total_copies")?,
            total_pastes: counter("total_pastes")?,
            since: counter("stats_since")?,
            oldest_clip: self.conn.query_row("SELECT MIN(created_at) FROM clips", [], |r| r.get(0))?,
        })
    }

    /// Ditto "Compact Database".
    pub fn compact(&mut self) -> Result<()> {
        self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }

    /// Ditto "Backup Database": write a consistent copy to `path` (must not exist).
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(Error::Invalid(format!("{} already exists", path.display())));
        }
        self.conn.execute("VACUUM INTO ?1", [path.to_string_lossy().into_owned()])?;
        Ok(())
    }
}

fn sticky_scope(scope: ListScope) -> (&'static str, String, Vec<Value>) {
    match scope {
        ListScope::Main => ("sticky_order", String::new(), Vec::new()),
        ListScope::Group(g) => ("group_sticky_order", " AND group_id = ?".to_string(), vec![Value::Integer(g)]),
    }
}

fn summary_from_row(r: &Row<'_>) -> rusqlite::Result<ClipSummary> {
    let formats: String = r.get(13)?;
    Ok(ClipSummary {
        id: r.get(0)?,
        description: r.get(1)?,
        kind: ClipKind::parse(&r.get::<_, String>(2)?),
        created_at: r.get(3)?,
        last_used_at: r.get(4)?,
        group_id: r.get(5)?,
        sticky: r.get(6)?,
        group_sticky: r.get(7)?,
        never_auto_delete: r.get(8)?,
        quick_paste_text: r.get(9)?,
        hotkey: r.get(10)?,
        source_app: r.get(11)?,
        size_bytes: r.get(12)?,
        formats: formats.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect(),
        has_thumbnail: r.get(14)?,
    })
}

fn insert_formats(conn: &Connection, id: i64, content: &ClipContent) -> Result<()> {
    let mut stmt = conn.prepare("INSERT INTO clip_data (clip_id, format, data) VALUES (?1, ?2, ?3)")?;
    for f in content.formats() {
        stmt.execute(params![id, f.name, f.data])?;
    }
    Ok(())
}

fn load_content(conn: &Connection, id: i64) -> Result<ClipContent> {
    let mut stmt = conn.prepare("SELECT format, data FROM clip_data WHERE clip_id = ?1")?;
    let mut rows = stmt.query([id])?;
    let mut content = ClipContent::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        content.set(&name, row.get(1)?);
    }
    Ok(content)
}

fn find_duplicate(conn: &Connection, hash: i64, content: &ClipContent) -> Result<Option<i64>> {
    let mut stmt = conn.prepare("SELECT id FROM clips WHERE hash = ?1 ORDER BY clip_order DESC")?;
    let ids: Vec<i64> = stmt.query_map([hash], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for id in ids {
        if load_content(conn, id)? == *content {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Ditto `CClip::GetNewOrder`: one more than the current maximum.
fn next_order(conn: &Connection, group: Option<i64>) -> Result<f64> {
    let max: Option<f64> = match group {
        None => conn.query_row("SELECT MAX(clip_order) FROM clips", [], |r| r.get(0))?,
        Some(g) => conn.query_row("SELECT MAX(group_order) FROM clips WHERE group_id = ?1", [g], |r| r.get(0))?,
    };
    Ok(max.unwrap_or(0.0) + 1.0)
}

fn bump_counter(conn: &Connection, key: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
        [key],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{SearchMode, SearchScopes, SearchSpec};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn add(s: &mut Store, text: &str) -> i64 {
        s.add_clip(&ClipContent::from_text(text), &AddOptions::default()).unwrap().id()
    }

    fn main_ids(s: &Store) -> Vec<i64> {
        s.list(&ListQuery { include_grouped: true, limit: 1000, ..Default::default() })
            .unwrap()
            .items
            .iter()
            .map(|c| c.id)
            .collect()
    }

    fn texts(s: &Store, q: &ListQuery<'_>) -> Vec<String> {
        s.list(q).unwrap().items.into_iter().map(|c| c.description).collect()
    }

    #[test]
    fn newest_first_and_duplicates_move_to_top() {
        let mut s = store();
        let a = add(&mut s, "a");
        let b = add(&mut s, "b");
        assert_eq!(main_ids(&s), vec![b, a]);
        let again = s.add_clip(&ClipContent::from_text("a"), &AddOptions::default()).unwrap();
        assert_eq!(again, AddOutcome::Duplicate(a));
        assert_eq!(main_ids(&s), vec![a, b]);
        assert_eq!(s.count().unwrap(), 2);

        let skip = AddOptions { duplicates: DuplicatePolicy::Skip, ..Default::default() };
        assert_eq!(s.add_clip(&ClipContent::from_text("b"), &skip).unwrap(), AddOutcome::Duplicate(b));
        assert_eq!(main_ids(&s), vec![a, b], "skip leaves the order alone");
    }

    #[test]
    fn allow_duplicates_still_collapses_immediate_repeats() {
        let mut s = store();
        let allow = AddOptions { duplicates: DuplicatePolicy::Allow, ..Default::default() };
        let a1 = s.add_clip(&ClipContent::from_text("a"), &allow).unwrap();
        assert_eq!(s.add_clip(&ClipContent::from_text("a"), &allow).unwrap(), AddOutcome::Duplicate(a1.id()));
        s.add_clip(&ClipContent::from_text("b"), &allow).unwrap();
        assert!(matches!(s.add_clip(&ClipContent::from_text("a"), &allow).unwrap(), AddOutcome::Inserted(_)));
        assert_eq!(s.count().unwrap(), 3);
    }

    #[test]
    fn rejects_empty_clips_and_roundtrips_content() {
        let mut s = store();
        assert!(s.add_clip(&ClipContent::new(), &AddOptions::default()).is_err());
        let content = ClipContent::from_text("hi").with(formats::HTML, b"<b>hi</b>".to_vec());
        let id = s.add_clip(&content, &AddOptions::default()).unwrap().id();
        assert_eq!(s.content(id).unwrap(), content);
        let summary = s.summary(id).unwrap();
        assert_eq!(summary.formats, vec!["html", "text"]);
        assert_eq!(summary.kind, ClipKind::Text);
        assert!(matches!(s.content(999), Err(Error::NotFound(_))));
    }

    #[test]
    fn images_get_thumbnails() {
        let mut s = store();
        let png = crate::model::tests::tiny_png();
        let id = s.add_clip(&ClipContent::from_png(png), &AddOptions::default()).unwrap().id();
        let summary = s.summary(id).unwrap();
        assert_eq!(summary.kind, ClipKind::Image);
        assert!(summary.has_thumbnail);
        assert_eq!(summary.description, "Image 2 × 3");
        assert!(s.thumbnail(id).unwrap().is_some());
    }

    #[test]
    fn pagination_and_search() {
        let mut s = store();
        for i in 0..25 {
            add(&mut s, &format!("item {i} {}", if i % 2 == 0 { "even" } else { "odd" }));
        }
        let page = s.list(&ListQuery { limit: 10, ..Default::default() }).unwrap();
        assert_eq!(page.items.len(), 10);
        assert!(page.has_more);
        assert_eq!(page.items[0].description, "item 24 even");
        let last = s.list(&ListQuery { limit: 10, offset: 20, ..Default::default() }).unwrap();
        assert_eq!(last.items.len(), 5);
        assert!(!last.has_more);

        let search = CompiledSearch::compile(&SearchSpec { query: "odd".into(), ..Default::default() }).unwrap().unwrap();
        let q = ListQuery { search: Some(&search), limit: 5, ..Default::default() };
        let page = s.list(&q).unwrap();
        assert_eq!(page.items.len(), 5);
        assert!(page.has_more);
        assert!(page.items.iter().all(|c| c.description.contains("odd")));
        let q2 = ListQuery { search: Some(&search), limit: 5, offset: 10, ..Default::default() };
        let rest = s.list(&q2).unwrap();
        assert_eq!(rest.items.len(), 2);
        assert!(!rest.has_more);
    }

    #[test]
    fn full_text_search_looks_past_the_description() {
        let mut s = store();
        let long = format!("{}needle", "x".repeat(600));
        s.add_clip(&ClipContent::from_text(long), &AddOptions::default()).unwrap();
        let desc_only = CompiledSearch::compile(&SearchSpec { query: "needle".into(), ..Default::default() }).unwrap().unwrap();
        assert!(s.list(&ListQuery { search: Some(&desc_only), limit: 10, ..Default::default() }).unwrap().items.is_empty());
        let full = CompiledSearch::compile(&SearchSpec {
            query: "needle".into(),
            mode: SearchMode::Wildcard,
            scopes: SearchScopes { description: true, full_text: true, quick_paste: false },
        })
        .unwrap()
        .unwrap();
        assert_eq!(s.list(&ListQuery { search: Some(&full), limit: 10, ..Default::default() }).unwrap().items.len(), 1);
    }

    #[test]
    fn quick_paste_text_is_searchable() {
        let mut s = store();
        let id = add(&mut s, "Best regards, Alex");
        s.set_quick_paste_text(id, Some("sig")).unwrap();
        add(&mut s, "other");
        let search = CompiledSearch::compile(&SearchSpec { query: "/q sig".into(), ..Default::default() }).unwrap().unwrap();
        let q = ListQuery { search: Some(&search), limit: 10, ..Default::default() };
        assert_eq!(texts(&s, &q), vec!["Best regards, Alex"]);
        s.set_quick_paste_text(id, Some("  ")).unwrap();
        assert_eq!(s.summary(id).unwrap().quick_paste_text, None);
    }

    #[test]
    fn sticky_clips_stay_on_top() {
        let mut s = store();
        let a = add(&mut s, "a");
        let b = add(&mut s, "b");
        let c = add(&mut s, "c");
        s.set_sticky(a, StickyOp::Top, ListScope::Main).unwrap();
        assert_eq!(main_ids(&s), vec![a, c, b]);
        let d = add(&mut s, "d");
        assert_eq!(main_ids(&s), vec![a, d, c, b], "new copies go below sticky clips");
        s.set_sticky(b, StickyOp::Last, ListScope::Main).unwrap();
        assert_eq!(main_ids(&s), vec![a, b, d, c]);
        s.set_sticky(c, StickyOp::Top, ListScope::Main).unwrap();
        assert_eq!(main_ids(&s), vec![c, a, b, d]);
        s.set_sticky(d, StickyOp::ReplaceTop, ListScope::Main).unwrap();
        assert_eq!(main_ids(&s), vec![d, a, b, c]);
        assert!(!s.summary(c).unwrap().sticky);
        s.set_sticky(a, StickyOp::Remove, ListScope::Main).unwrap();
        assert_eq!(main_ids(&s), vec![d, b, c, a]);
    }

    #[test]
    fn clip_order_operations() {
        let mut s = store();
        let a = add(&mut s, "a");
        let b = add(&mut s, "b");
        let c = add(&mut s, "c");
        // list: c b a
        s.move_clip(a, OrderOp::Up, ListScope::Main, true).unwrap();
        assert_eq!(main_ids(&s), vec![c, a, b]);
        s.move_clip(c, OrderOp::Down, ListScope::Main, true).unwrap();
        assert_eq!(main_ids(&s), vec![a, c, b]);
        s.move_clip(b, OrderOp::Top, ListScope::Main, true).unwrap();
        assert_eq!(main_ids(&s), vec![b, a, c]);
        s.move_clip(b, OrderOp::Last, ListScope::Main, true).unwrap();
        assert_eq!(main_ids(&s), vec![a, c, b]);
        // Moving the top clip up is a no-op.
        s.move_clip(a, OrderOp::Up, ListScope::Main, true).unwrap();
        assert_eq!(main_ids(&s), vec![a, c, b]);
    }

    #[test]
    fn pasting_moves_to_top_and_counts() {
        let mut s = store();
        let a = add(&mut s, "a");
        let b = add(&mut s, "b");
        s.mark_pasted(a, true).unwrap();
        assert_eq!(main_ids(&s), vec![a, b]);
        s.mark_pasted(b, false).unwrap();
        assert_eq!(main_ids(&s), vec![a, b]);
        let stats = s.stats().unwrap();
        assert_eq!(stats.total_pastes, 2);
        assert_eq!(stats.total_copies, 2);
        assert_eq!(stats.clips, 2);
        assert!(stats.since > 0);
    }

    #[test]
    fn groups_and_scoped_lists() {
        let mut s = store();
        let work = s.create_group("Work", None).unwrap();
        let sub = s.create_group("Snippets", Some(work)).unwrap();
        assert!(s.create_group("  ", None).is_err());
        assert!(s.create_group("x", Some(999)).is_err());
        let a = add(&mut s, "a");
        let b = add(&mut s, "b");
        let c = add(&mut s, "c");
        s.move_to_group(&[a, b], Some(work)).unwrap();
        s.move_to_group(&[c], Some(sub)).unwrap();

        let in_work = |s: &Store| -> Vec<i64> {
            s.list(&ListQuery { scope: ListScope::Group(work), limit: 10, ..Default::default() }).unwrap().items.iter().map(|c| c.id).collect()
        };
        assert_eq!(in_work(&s), vec![b, a]);
        // Main list with and without grouped clips.
        assert_eq!(main_ids(&s).len(), 3);
        assert!(s.list(&ListQuery { include_grouped: false, limit: 10, ..Default::default() }).unwrap().items.is_empty());

        // Group-scoped sticky and ordering.
        s.set_sticky(a, StickyOp::Top, ListScope::Group(work)).unwrap();
        assert_eq!(in_work(&s), vec![a, b]);
        assert!(s.summary(a).unwrap().group_sticky);
        assert!(!s.summary(a).unwrap().sticky);

        let groups = s.groups().unwrap();
        assert_eq!(groups.iter().map(|g| (g.name.as_str(), g.clip_count)).collect::<Vec<_>>(), vec![("Snippets", 1), ("Work", 2)]);
        s.rename_group(sub, "Code").unwrap();

        // Deleting a group keeps (or deletes) its clips, including sub-group clips.
        s.delete_group(work, false).unwrap();
        assert!(s.groups().unwrap().is_empty());
        assert_eq!(s.count().unwrap(), 3);
        assert!(s.summary(c).unwrap().group_id.is_none());

        let g = s.create_group("Temp", None).unwrap();
        s.move_to_group(&[a], Some(g)).unwrap();
        s.delete_group(g, true).unwrap();
        assert_eq!(s.count().unwrap(), 2);
    }

    #[test]
    fn retention_respects_protected_clips() {
        let mut s = store();
        let ids: Vec<i64> = (0..10).map(|i| add(&mut s, &format!("clip {i}"))).collect();
        s.set_never_auto_delete(ids[0], true).unwrap();
        s.set_hotkey(ids[1], Some("Ctrl+Alt+1")).unwrap();
        s.set_sticky(ids[2], StickyOp::Top, ListScope::Main).unwrap();
        let g = s.create_group("keep", None).unwrap();
        s.move_to_group(&[ids[3]], Some(g)).unwrap();

        let deleted = s.enforce_retention(Some(3), None).unwrap();
        let remaining = main_ids(&s);
        // Top three (by clip order: 9, 8, 7) plus the four protected old clips survive.
        assert_eq!(deleted, 3);
        for keep in [ids[9], ids[8], ids[7], ids[0], ids[1], ids[2], ids[3]] {
            assert!(remaining.contains(&keep), "{keep} should survive");
        }

        // Expiry looks at last use.
        s.connection().execute("UPDATE clips SET last_used_at = 0", []).unwrap();
        let deleted = s.enforce_retention(None, Some(5)).unwrap();
        assert_eq!(deleted, 3);
        assert_eq!(s.count().unwrap(), 4);
        assert_eq!(s.clear_history().unwrap(), 0, "only protected clips are left");
    }

    #[test]
    fn hotkeys_are_unique() {
        let mut s = store();
        let a = add(&mut s, "a");
        let b = add(&mut s, "b");
        s.set_hotkey(a, Some("Ctrl+1")).unwrap();
        s.set_hotkey(b, Some("Ctrl+1")).unwrap();
        assert_eq!(s.hotkey_clips().unwrap(), vec![(b, "Ctrl+1".to_string())]);
        s.set_hotkey(b, None).unwrap();
        assert!(s.hotkey_clips().unwrap().is_empty());
    }

    #[test]
    fn editing_text_drops_rich_formats() {
        let mut s = store();
        let content = ClipContent::from_text("old").with(formats::HTML, b"<b>old</b>".to_vec());
        let id = s.add_clip(&content, &AddOptions::default()).unwrap().id();
        s.update_text(id, "new text", 500).unwrap();
        let c = s.content(id).unwrap();
        assert_eq!(c.text(), Some("new text"));
        assert!(c.html().is_none());
        assert_eq!(s.summary(id).unwrap().description, "new text");
        // The edited clip de-duplicates against its new content.
        assert_eq!(s.add_clip(&ClipContent::from_text("new text"), &AddOptions::default()).unwrap(), AddOutcome::Duplicate(id));
    }

    #[test]
    fn persists_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/history.db");
        {
            let mut s = Store::open(&path).unwrap();
            add(&mut s, "persisted");
            s.backup_to(&dir.path().join("backup.db")).unwrap();
            assert!(s.backup_to(&dir.path().join("backup.db")).is_err());
            s.compact().unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.count().unwrap(), 1);
        let backup = Store::open(&dir.path().join("backup.db")).unwrap();
        assert_eq!(backup.count().unwrap(), 1);
    }
}
