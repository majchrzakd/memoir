//! Local store: folders and notes in the encrypted SQLite database.
//!
//! The store is the source of truth for the UI. Every change is one
//! transaction that updates the object, its search index and the sync
//! outbox together, so after a commit the change survives a crash and is
//! guaranteed to be pushed eventually.

mod schema;
pub mod sort_key;
mod text;

use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

use crate::crypto::SecretKey;
use crate::db::open_encrypted;
use crate::vault::{UnlockedVault, VaultFile};

/// Keep at most one history version per this much editing time.
const HISTORY_INTERVAL_MS: i64 = 10 * 60 * 1000;
const HISTORY_MAX_VERSIONS: i64 = 50;

/// Marks the start / end of a matched term in [`SearchHit::snippet`].
pub const MATCH_START: char = '\u{1}';
pub const MATCH_END: char = '\u{2}';

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error("item {0} not found")]
    NotFound(ObjectId),
    #[error("{0} is not a folder")]
    NotAFolder(ObjectId),
    #[error("{0} is not a note")]
    NotANote(ObjectId),
    #[error("a folder can't be moved into itself or one of its subfolders")]
    Cycle,
    #[error("{0} is not in the target folder")]
    NotASibling(ObjectId),
    #[error("folder name can't be empty")]
    EmptyName,
    #[error("{0} is in the trash")]
    InTrash(ObjectId),
    #[error("{0} is not in the trash")]
    NotInTrash(ObjectId),
    #[error("{0:?} is not a valid date (expected YYYY-MM-DD)")]
    InvalidDate(String),
    #[error("no saved version of {0} at {1}")]
    NoSuchVersion(ObjectId, i64),
    #[error("this database was written by a newer version of Memoir (schema {0})")]
    NewerSchema(usize),
    #[error("stored data is corrupt: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Identifier of a folder or note: a ULID, unique across devices.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(Ulid);

impl ObjectId {
    pub fn generate() -> Self {
        Self(Ulid::generate())
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({})", self.0)
    }
}

impl FromStr for ObjectId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ulid::from_str(s).map(Self)
    }
}

impl Serialize for ObjectId {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl ToSql for ObjectId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0.to_string()))
    }
}

impl FromSql for ObjectId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Folder,
    Note,
}

impl Kind {
    fn code(self) -> i64 {
        match self {
            Kind::Folder => 1,
            Kind::Note => 2,
        }
    }

    fn from_code(code: i64) -> rusqlite::Result<Self> {
        match code {
            1 => Ok(Kind::Folder),
            2 => Ok(Kind::Note),
            _ => Err(rusqlite::Error::IntegralValueOutOfRange(2, code)),
        }
    }
}

/// A folder or note as shown in the tree (no note body).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TreeItem {
    pub id: ObjectId,
    pub kind: Kind,
    /// `None` = top level (also used for items whose parent is missing).
    pub parent_id: Option<ObjectId>,
    pub title: String,
    pub sort_key: String,
    /// Last change to the content (title, body, tags) — not moves, trash or
    /// the Later flag.
    pub updated_at: i64,
    pub trashed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub id: ObjectId,
    pub parent_id: Option<ObjectId>,
    pub title: String,
    /// Markdown.
    pub body: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    /// Last change to the content (title, body, tags).
    pub updated_at: i64,
    pub trashed_at: Option<i64>,
    /// When the note was put on the Later list; `None` = not on it.
    pub later_at: Option<i64>,
    /// Optional due date for a Later note, `YYYY-MM-DD`.
    pub due_date: Option<String>,
}

/// A note on the Later list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaterItem {
    pub id: ObjectId,
    pub parent_id: Option<ObjectId>,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub id: ObjectId,
    pub parent_id: Option<ObjectId>,
    pub title: String,
    /// Plain-text excerpt; matches are wrapped in [`MATCH_START`]/[`MATCH_END`].
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEntry {
    /// When this version was last saved; identifies it.
    pub saved_at: i64,
    pub title: String,
    pub body: String,
}

/// Where among its siblings an item goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "lowercase")]
pub enum Position {
    First,
    Last,
    Before(ObjectId),
    After(ObjectId),
}

/// Target folder (`None` = top level) and position within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Placement {
    pub parent: Option<ObjectId>,
    pub position: Position,
}

impl Placement {
    pub fn last_in(parent: Option<ObjectId>) -> Self {
        Self {
            parent,
            position: Position::Last,
        }
    }
}

/// All objects in the trash or inside a trashed folder.
const HIDDEN_CTE: &str = "
    WITH RECURSIVE hidden(id) AS (
        SELECT id FROM objects WHERE trashed_at IS NOT NULL
        UNION
        SELECT o.id FROM objects o JOIN hidden h ON o.parent_id = h.id
    )";

const TREE_COLUMNS: &str = "id, kind, parent_id, title, sort_key, updated_at, trashed_at";

pub struct Store {
    conn: Connection,
    clock: Box<dyn Fn() -> i64 + Send>,
}

impl Store {
    /// Opens (creating and migrating if needed) the local database.
    pub fn open(path: &Path, key: &SecretKey) -> Result<Self> {
        let mut conn = open_encrypted(path, key)?;
        // Every committed change is on disk before the call returns.
        conn.pragma_update(None, "synchronous", "FULL")?;
        schema::migrate(&mut conn)?;
        let store = Self {
            conn,
            clock: Box::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after 1970")
                    .as_millis() as i64
            }),
        };
        store.device_id()?;
        Ok(store)
    }

    #[cfg(test)]
    fn set_clock(&mut self, clock: impl Fn() -> i64 + Send + 'static) {
        self.clock = Box::new(clock);
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Random identifier of this device's copy, created on first open.
    pub fn device_id(&self) -> Result<String> {
        if let Some(id) = self.meta_text("device_id")? {
            return Ok(id);
        }
        let id = Ulid::generate().to_string();
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('device_id', ?1)",
            [&id],
        )?;
        Ok(id)
    }

    // ---- Vault --------------------------------------------------------

    /// Keeps the vault file and master key inside the encrypted database, so
    /// an opened store can sync without asking for the passphrase.
    pub fn save_vault(&mut self, file: &VaultFile, vault: &UnlockedVault) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('vault_json', ?1)",
            [file.to_json()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('master_key', ?1)",
            [&vault.master().as_bytes()[..]],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_vault(&self) -> Result<Option<(VaultFile, UnlockedVault)>> {
        let Some(json) = self.meta_text("vault_json")? else {
            return Ok(None);
        };
        let master: Vec<u8> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'master_key'", [], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| StoreError::Corrupt("vault without master key".into()))?;
        let master = zeroize::Zeroizing::new(master);
        let file = VaultFile::from_json(&json).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let master = SecretKey::from_slice(&master)
            .ok_or_else(|| StoreError::Corrupt("master key has the wrong size".into()))?;
        let vault = UnlockedVault::from_master(&file, master);
        Ok(Some((file, vault)))
    }

    fn meta_text(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    // ---- Tree ---------------------------------------------------------

    /// Every folder and note not in the trash, ordered by parent, then
    /// folders before notes, then position.
    pub fn tree(&self) -> Result<Vec<TreeItem>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "{HIDDEN_CTE}
             SELECT {TREE_COLUMNS} FROM objects
             WHERE id NOT IN hidden
             ORDER BY parent_id, kind, sort_key, id"
        ))?;
        let mut items = stmt
            .query_map([], tree_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Orphans (parent missing, e.g. purged on another device) go to the top.
        let ids: HashSet<ObjectId> = items.iter().map(|i| i.id).collect();
        for item in &mut items {
            if item.parent_id.is_some_and(|p| !ids.contains(&p)) {
                item.parent_id = None;
            }
        }
        Ok(items)
    }

    /// Items moved to the trash, most recent first. Contents of a trashed
    /// folder are not listed separately.
    pub fn trashed_items(&self) -> Result<Vec<TreeItem>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "{HIDDEN_CTE}
             SELECT {TREE_COLUMNS} FROM objects
             WHERE trashed_at IS NOT NULL
               AND (parent_id IS NULL OR parent_id NOT IN hidden)
             ORDER BY trashed_at DESC, id"
        ))?;
        Ok(stmt
            .query_map([], tree_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_folder(&mut self, placement: Placement, name: &str) -> Result<ObjectId> {
        let name = folder_name(name)?;
        self.create(Kind::Folder, placement, &name, "")
    }

    pub fn create_note(
        &mut self,
        placement: Placement,
        title: &str,
        body: &str,
    ) -> Result<ObjectId> {
        self.create(Kind::Note, placement, title, body)
    }

    fn create(
        &mut self,
        kind: Kind,
        placement: Placement,
        title: &str,
        body: &str,
    ) -> Result<ObjectId> {
        let now = self.now();
        let id = ObjectId::generate();
        let tx = self.conn.transaction()?;
        let sort_key = place(&tx, placement, None)?;
        tx.execute(
            "INSERT INTO objects (id, kind, parent_id, title, sort_key, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, kind.code(), placement.parent, title, sort_key, now],
        )?;
        if kind == Kind::Note {
            tx.execute(
                "INSERT INTO note_bodies (seq, body) VALUES (?1, ?2)",
                params![tx.last_insert_rowid(), body],
            )?;
            index_note(&tx, &id)?;
        }
        enqueue(&tx, &id, now)?;
        tx.commit()?;
        Ok(id)
    }

    /// Renames a folder or retitles a note.
    pub fn rename(&mut self, id: &ObjectId, title: &str) -> Result<()> {
        let kind = self.kind(id)?;
        let title = match kind {
            Kind::Folder => folder_name(title)?,
            Kind::Note => title.to_owned(),
        };
        let now = self.now();
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE objects SET title = ?2, updated_at = ?3 WHERE id = ?1 AND title != ?2",
            params![id, title, now],
        )?;
        if changed > 0 {
            if kind == Kind::Note {
                index_note(&tx, id)?;
            }
            enqueue(&tx, id, now)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn move_item(&mut self, id: &ObjectId, placement: Placement) -> Result<()> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        object_kind(&tx, id)?;
        let sort_key = place(&tx, placement, Some(id))?;
        tx.execute(
            "UPDATE objects SET parent_id = ?2, sort_key = ?3 WHERE id = ?1",
            params![id, placement.parent, sort_key],
        )?;
        enqueue(&tx, id, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Moves an item (and, implicitly, a folder's contents) to the trash.
    pub fn move_to_trash(&mut self, id: &ObjectId) -> Result<()> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        object_kind(&tx, id)?;
        let changed = tx.execute(
            "UPDATE objects SET trashed_at = ?2 WHERE id = ?1 AND trashed_at IS NULL",
            params![id, now],
        )?;
        if changed > 0 {
            enqueue(&tx, id, now)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Restores a trashed item to where it was, or to the top level if that
    /// folder is gone or itself in the trash.
    pub fn restore(&mut self, id: &ObjectId) -> Result<()> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let (parent, trashed_at): (Option<ObjectId>, Option<i64>) = tx
            .query_row(
                "SELECT parent_id, trashed_at FROM objects WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound(*id))?;
        if trashed_at.is_none() {
            return Err(StoreError::NotInTrash(*id));
        }

        let parent_ok = match &parent {
            None => true,
            Some(p) => {
                let chain = ancestors(&tx, p)?;
                !chain.is_empty() && chain.iter().all(|(_, trashed)| trashed.is_none())
            }
        };
        if parent_ok {
            tx.execute("UPDATE objects SET trashed_at = NULL WHERE id = ?1", [id])?;
        } else {
            let sort_key = place(&tx, Placement::last_in(None), Some(id))?;
            tx.execute(
                "UPDATE objects SET trashed_at = NULL, parent_id = NULL, sort_key = ?2 WHERE id = ?1",
                params![id, sort_key],
            )?;
        }
        enqueue(&tx, id, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Permanently deletes a trashed item and everything inside it.
    pub fn purge(&mut self, id: &ObjectId) -> Result<()> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let trashed: Option<i64> = tx
            .query_row("SELECT trashed_at FROM objects WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or(StoreError::NotFound(*id))?;
        if trashed.is_none() {
            return Err(StoreError::NotInTrash(*id));
        }
        purge_subtree(&tx, id, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Permanently deletes everything in the trash.
    pub fn empty_trash(&mut self) -> Result<usize> {
        self.purge_trashed_before(i64::MAX)
    }

    /// Permanently deletes items trashed before `cutoff` (unix ms), with
    /// their contents. Returns how many objects were deleted.
    pub fn purge_trashed_before(&mut self, cutoff: i64) -> Result<usize> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let ids: Vec<ObjectId> = tx
            .prepare("SELECT id FROM objects WHERE trashed_at < ?1")?
            .query_map([cutoff], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut purged = 0;
        for id in &ids {
            // An earlier subtree may already have taken this one with it.
            if object_kind(&tx, id).is_ok() {
                purged += purge_subtree(&tx, id, now)?;
            }
        }
        tx.commit()?;
        Ok(purged)
    }

    fn kind(&self, id: &ObjectId) -> Result<Kind> {
        object_kind(&self.conn, id)
    }

    // ---- Notes --------------------------------------------------------

    pub fn note(&self, id: &ObjectId) -> Result<Note> {
        let note = self
            .conn
            .prepare_cached(
                "SELECT o.id, o.parent_id, o.title, coalesce(b.body, ''), o.tags,
                        o.created_at, o.updated_at, o.trashed_at, o.kind, o.later_at, o.due_date
                 FROM objects o LEFT JOIN note_bodies b ON b.seq = o.seq
                 WHERE o.id = ?1",
            )?
            .query_row([id], |r| {
                Ok((
                    Note {
                        id: r.get(0)?,
                        parent_id: r.get(1)?,
                        title: r.get(2)?,
                        body: r.get(3)?,
                        tags: parse_tags(&r.get::<_, String>(4)?),
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                        trashed_at: r.get(7)?,
                        later_at: r.get(9)?,
                        due_date: r.get(10)?,
                    },
                    Kind::from_code(r.get(8)?)?,
                ))
            })
            .optional()?
            .ok_or(StoreError::NotFound(*id))?;
        match note {
            (note, Kind::Note) => Ok(note),
            _ => Err(StoreError::NotANote(*id)),
        }
    }

    /// Saves a note's title and body. Saving unchanged content is a no-op,
    /// so re-saving never creates sync traffic.
    pub fn save_note(&mut self, id: &ObjectId, title: &str, body: &str) -> Result<()> {
        let old = self.note(id)?;
        if old.title == title && old.body == body {
            return Ok(());
        }
        let now = self.now();
        let tx = self.conn.transaction()?;
        snapshot_if_due(&tx, &old, now)?;
        tx.execute(
            "UPDATE objects SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, title, now],
        )?;
        tx.execute(
            "UPDATE note_bodies SET body = ?2 WHERE seq = (SELECT seq FROM objects WHERE id = ?1)",
            params![id, body],
        )?;
        index_note(&tx, id)?;
        enqueue(&tx, id, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Replaces a note's tags (trimmed, empty ones dropped, de-duplicated
    /// ignoring case, order kept).
    pub fn set_tags(&mut self, id: &ObjectId, tags: &[String]) -> Result<()> {
        let old = self.note(id)?;
        let mut seen = HashSet::new();
        let tags: Vec<String> = tags
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty() && seen.insert(t.to_lowercase()))
            .collect();
        if tags == old.tags {
            return Ok(());
        }
        let now = self.now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE objects SET tags = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&tags).expect("tags serialize"),
                now
            ],
        )?;
        index_note(&tx, id)?;
        enqueue(&tx, id, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Puts a note on the Later list (with an optional due date, `YYYY-MM-DD`)
    /// or takes it off (`later = false`, which also clears the due date).
    pub fn set_later(&mut self, id: &ObjectId, later: bool, due_date: Option<&str>) -> Result<()> {
        let old = self.note(id)?;
        let due_date = match (later, due_date) {
            (true, Some(d)) => Some(valid_date(d)?),
            _ => None,
        };
        if old.later_at.is_some() == later && old.due_date == due_date {
            return Ok(());
        }
        let now = self.now();
        // Keep the original "marked at" time when only the due date changes.
        let later_at = later.then(|| old.later_at.unwrap_or(now));
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE objects SET later_at = ?2, due_date = ?3 WHERE id = ?1",
            params![id, later_at, due_date],
        )?;
        enqueue(&tx, id, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Notes on the Later list (not in the trash): those with a due date
    /// first, soonest first; then by creation time, newest first.
    pub fn later_notes(&self) -> Result<Vec<LaterItem>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "{HIDDEN_CTE}
             SELECT id, parent_id, title, created_at, updated_at, due_date FROM objects
             WHERE later_at IS NOT NULL AND kind = 2 AND id NOT IN hidden
             ORDER BY due_date IS NULL, due_date, created_at DESC, id"
        ))?;
        Ok(stmt
            .query_map([], |r| {
                Ok(LaterItem {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    title: r.get(2)?,
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
                    due_date: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Earlier versions of a note, newest first.
    pub fn history(&self, id: &ObjectId) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT saved_at, title, body FROM history WHERE object_id = ?1 ORDER BY saved_at DESC",
        )?;
        Ok(stmt
            .query_map([id], |r| {
                Ok(HistoryEntry {
                    saved_at: r.get(0)?,
                    title: r.get(1)?,
                    body: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Makes an earlier version current again (the current one goes to history).
    pub fn restore_version(&mut self, id: &ObjectId, saved_at: i64) -> Result<()> {
        let version = self
            .history(id)?
            .into_iter()
            .find(|v| v.saved_at == saved_at)
            .ok_or(StoreError::NoSuchVersion(*id, saved_at))?;
        let current = self.note(id)?;
        let tx = self.conn.transaction()?;
        // Always keep what is being replaced, regardless of the interval.
        tx.execute(
            "INSERT OR IGNORE INTO history (object_id, saved_at, title, body) VALUES (?1, ?2, ?3, ?4)",
            params![id, current.updated_at, current.title, current.body],
        )?;
        tx.commit()?;
        self.save_note(id, &version.title, &version.body)
    }

    /// Notes containing every word of `query` (the last one as a prefix),
    /// best matches first. Trashed notes are excluded.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(fts) = text::fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare_cached(&format!(
            "{HIDDEN_CTE}
             SELECT o.id, o.parent_id, o.title,
                    snippet(notes_fts, -1, '{MATCH_START}', '{MATCH_END}', '…', 16)
             FROM notes_fts JOIN objects o ON o.seq = notes_fts.rowid
             WHERE notes_fts MATCH ?1 AND o.id NOT IN hidden
             ORDER BY bm25(notes_fts, 10.0, 1.0, 5.0)
             LIMIT ?2"
        ))?;
        Ok(stmt
            .query_map(params![fts, limit as i64], |r| {
                Ok(SearchHit {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    title: r.get(2)?,
                    snippet: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Number of local changes not yet pushed to the cloud.
    pub fn pending_changes(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))?;
        Ok(n as u64)
    }
}

fn tree_item(r: &Row<'_>) -> rusqlite::Result<TreeItem> {
    Ok(TreeItem {
        id: r.get(0)?,
        kind: Kind::from_code(r.get(1)?)?,
        parent_id: r.get(2)?,
        title: r.get(3)?,
        sort_key: r.get(4)?,
        updated_at: r.get(5)?,
        trashed_at: r.get(6)?,
    })
}

fn folder_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StoreError::EmptyName);
    }
    Ok(name.to_owned())
}

/// Accepts a real calendar date written as `YYYY-MM-DD` (so dates sort as text).
fn valid_date(date: &str) -> Result<String> {
    let invalid = || StoreError::InvalidDate(date.to_owned());
    let parts: Vec<&str> = date.split('-').collect();
    let [y, m, d] = parts[..] else {
        return Err(invalid());
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return Err(invalid());
    }
    let (y, m, d): (u32, u32, u32) = (
        y.parse().map_err(|_| invalid())?,
        m.parse().map_err(|_| invalid())?,
        d.parse().map_err(|_| invalid())?,
    );
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(invalid()),
    };
    if !(1..=days).contains(&d) {
        return Err(invalid());
    }
    Ok(date.to_owned())
}

fn parse_tags(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn object_kind(conn: &Connection, id: &ObjectId) -> Result<Kind> {
    let code: i64 = conn
        .query_row("SELECT kind FROM objects WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or(StoreError::NotFound(*id))?;
    Ok(Kind::from_code(code)?)
}

/// `id` and its ancestors (nearest first) with their `trashed_at`. Stops at a
/// missing parent; a cycle (possible only through corrupt data) is an error.
fn ancestors(conn: &Connection, id: &ObjectId) -> Result<Vec<(ObjectId, Option<i64>)>> {
    let mut stmt =
        conn.prepare_cached("SELECT parent_id, trashed_at FROM objects WHERE id = ?1")?;
    let mut chain = Vec::new();
    let mut current = Some(*id);
    while let Some(cur) = current {
        let Some((parent, trashed)) = stmt
            .query_row([cur], |r| {
                Ok((
                    r.get::<_, Option<ObjectId>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                ))
            })
            .optional()?
        else {
            break;
        };
        if chain.iter().any(|(seen, _)| *seen == cur) {
            return Err(StoreError::Corrupt(format!("folder cycle at {cur}")));
        }
        chain.push((cur, trashed));
        current = parent;
    }
    Ok(chain)
}

/// Computes the sort key for an item placed at `placement`, checking that
/// the target folder exists, is a folder, is not in the trash and — when
/// `moving` an existing item — is not that item or inside it.
fn place(tx: &Transaction<'_>, placement: Placement, moving: Option<&ObjectId>) -> Result<String> {
    if let Some(parent) = &placement.parent {
        if object_kind(tx, parent)? != Kind::Folder {
            return Err(StoreError::NotAFolder(*parent));
        }
        let chain = ancestors(tx, parent)?;
        if chain.iter().any(|(a, _)| Some(a) == moving) {
            return Err(StoreError::Cycle);
        }
        if chain.iter().any(|(_, t)| t.is_some()) {
            return Err(StoreError::InTrash(*parent));
        }
    }

    let mut stmt = tx.prepare_cached(
        "SELECT id, sort_key FROM objects WHERE parent_id IS ?1 ORDER BY sort_key, id",
    )?;
    let siblings: Vec<(ObjectId, String)> = stmt
        .query_map([placement.parent], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter(|s| !matches!(s, Ok((id, _)) if Some(id) == moving))
        .collect::<rusqlite::Result<_>>()?;
    let index_of = |target: &ObjectId| {
        siblings
            .iter()
            .position(|(id, _)| id == target)
            .ok_or(StoreError::NotASibling(*target))
    };

    let (before, after) = match placement.position {
        Position::First => (None, siblings.first()),
        Position::Last => (siblings.last(), None),
        Position::After(s) => {
            let i = index_of(&s)?;
            (Some(&siblings[i]), siblings.get(i + 1))
        }
        Position::Before(s) => {
            let i = index_of(&s)?;
            (i.checked_sub(1).map(|j| &siblings[j]), Some(&siblings[i]))
        }
    };
    let before = before.map(|(_, k)| k.as_str());
    let after = after.map(|(_, k)| k.as_str());
    Ok(match (before, after) {
        // Equal neighbours can only come from concurrent inserts on two
        // devices; there is no key between them, so go just after `before`.
        (Some(b), Some(a)) if b >= a => sort_key::between(Some(b), None),
        _ => sort_key::between(before, after),
    })
}

fn enqueue(tx: &Transaction<'_>, id: &ObjectId, now: i64) -> Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO outbox (object_id, queued_at) VALUES (?1, ?2)",
        params![id, now],
    )?;
    Ok(())
}

/// Rewrites a note's search entry from its current row.
fn index_note(tx: &Transaction<'_>, id: &ObjectId) -> Result<()> {
    let (seq, title, body, tags): (i64, String, String, String) = tx.query_row(
        "SELECT o.seq, o.title, b.body, o.tags
         FROM objects o JOIN note_bodies b ON b.seq = o.seq WHERE o.id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [seq])?;
    tx.execute(
        "INSERT INTO notes_fts (rowid, title, body, tags) VALUES (?1, ?2, ?3, ?4)",
        params![
            seq,
            title,
            text::markdown_to_text(&body),
            parse_tags(&tags).join(" ")
        ],
    )?;
    Ok(())
}

/// Puts the version being replaced into history when it is worth keeping:
/// it sat unchanged for [`HISTORY_INTERVAL_MS`] (an edit after a pause), or
/// the newest kept version is at least that much older (a checkpoint during
/// long continuous editing). Autosaves in between are not kept. Only the
/// newest [`HISTORY_MAX_VERSIONS`] are retained.
fn snapshot_if_due(tx: &Transaction<'_>, old: &Note, now: i64) -> Result<()> {
    if old.title.is_empty() && old.body.is_empty() {
        return Ok(());
    }
    let last: Option<i64> = tx.query_row(
        "SELECT max(saved_at) FROM history WHERE object_id = ?1",
        [old.id],
        |r| r.get(0),
    )?;
    let settled = now - old.updated_at >= HISTORY_INTERVAL_MS;
    let checkpoint = last.is_none_or(|t| old.updated_at - t >= HISTORY_INTERVAL_MS);
    if !settled && !checkpoint {
        return Ok(());
    }
    tx.execute(
        "INSERT OR IGNORE INTO history (object_id, saved_at, title, body) VALUES (?1, ?2, ?3, ?4)",
        params![old.id, old.updated_at, old.title, old.body],
    )?;
    tx.execute(
        "DELETE FROM history WHERE object_id = ?1 AND saved_at NOT IN (
             SELECT saved_at FROM history WHERE object_id = ?1 ORDER BY saved_at DESC LIMIT ?2)",
        params![old.id, HISTORY_MAX_VERSIONS],
    )?;
    Ok(())
}

/// Deletes `id` and all its descendants; queues each deletion for sync.
fn purge_subtree(tx: &Transaction<'_>, id: &ObjectId, now: i64) -> Result<usize> {
    let doomed: Vec<(ObjectId, i64)> = tx
        .prepare(
            "WITH RECURSIVE sub(id) AS (
                 SELECT ?1 UNION SELECT o.id FROM objects o JOIN sub ON o.parent_id = sub.id)
             SELECT o.id, o.seq FROM objects o JOIN sub ON o.id = sub.id",
        )?
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (doomed_id, seq) in &doomed {
        tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [seq])?;
        tx.execute("DELETE FROM note_bodies WHERE seq = ?1", [seq])?;
        tx.execute("DELETE FROM history WHERE object_id = ?1", [doomed_id])?;
        tx.execute("DELETE FROM objects WHERE id = ?1", [doomed_id])?;
        enqueue(tx, doomed_id, now)?;
    }
    Ok(doomed.len())
}

#[cfg(test)]
mod tests;
