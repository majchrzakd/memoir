//! Database schema and migrations. `PRAGMA user_version` holds the number of
//! applied migrations; each runs in its own transaction. Never edit a
//! released migration — append a new one.

use rusqlite::Connection;

use super::StoreError;

const MIGRATIONS: &[&str] = &[
    // 1: folders, notes, search, sync outbox, history.
    r#"
    CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value BLOB NOT NULL
    );

    -- Folders (kind 1) and notes (kind 2). `seq` is a stable integer key for
    -- the FTS index (implicit rowids may be renumbered by VACUUM).
    -- parent_id has no foreign key on purpose: sync may deliver a child
    -- before its parent, and orphans are shown at the top level.
    -- Rows stay small (no note bodies) so loading the tree reads little.
    CREATE TABLE objects (
        seq         INTEGER PRIMARY KEY,
        id          TEXT NOT NULL UNIQUE,
        kind        INTEGER NOT NULL CHECK (kind IN (1, 2)),
        parent_id   TEXT,
        title       TEXT NOT NULL,
        sort_key    TEXT NOT NULL,
        tags        TEXT NOT NULL DEFAULT '[]',
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL,
        trashed_at  INTEGER
    );
    CREATE INDEX objects_by_parent ON objects (parent_id, sort_key);
    CREATE INDEX objects_trashed ON objects (trashed_at) WHERE trashed_at IS NOT NULL;

    -- Markdown of each note; seq = objects.seq.
    CREATE TABLE note_bodies (
        seq   INTEGER PRIMARY KEY,
        body  TEXT NOT NULL
    );

    -- Plain text of notes (Markdown stripped); rowid = objects.seq.
    CREATE VIRTUAL TABLE notes_fts USING fts5 (
        title, body, tags,
        tokenize = 'unicode61 remove_diacritics 2'
    );

    -- Objects changed locally and not yet pushed; one row per object.
    -- An id with no row in `objects` is a deletion.
    CREATE TABLE outbox (
        object_id  TEXT PRIMARY KEY,
        queued_at  INTEGER NOT NULL
    );

    -- Earlier versions of notes.
    CREATE TABLE history (
        object_id  TEXT NOT NULL,
        saved_at   INTEGER NOT NULL,
        title      TEXT NOT NULL,
        body       TEXT NOT NULL,
        PRIMARY KEY (object_id, saved_at)
    ) WITHOUT ROWID;
    "#,
    // 2: "Later" list — when a note was marked, and an optional due date
    // (calendar date, YYYY-MM-DD, no time zone).
    r#"
    ALTER TABLE objects ADD COLUMN later_at INTEGER;
    ALTER TABLE objects ADD COLUMN due_date TEXT;
    CREATE INDEX objects_later ON objects (later_at) WHERE later_at IS NOT NULL;
    "#,
    // 3: attachments. The encrypted files live in the blob store; this keeps
    // what the UI needs to show them and, for sync, whether they're uploaded.
    r#"
    CREATE TABLE blobs (
        id           TEXT PRIMARY KEY,
        size         INTEGER NOT NULL,
        mime         TEXT NOT NULL,
        created_at   INTEGER NOT NULL,
        uploaded_at  INTEGER
    );
    "#,
];

pub fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let applied: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let applied = usize::try_from(applied).unwrap_or(usize::MAX);
    if applied > MIGRATIONS.len() {
        return Err(StoreError::NewerSchema(applied));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}
