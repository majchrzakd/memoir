//! The store's side of sync: local objects as revision payloads, applying
//! merged results, and bookkeeping of what exists in the cloud. Changes made
//! here come from sync, so they don't record new change clocks.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};

use super::{Kind, ObjectId, Result, Store, StoreError, index_note, parse_tags, snapshot_if_due};
use crate::sync::payload::{AttachmentRef, Clock, Clocks, ObjectPayload, attachment_ids};

/// A local object waiting to be pushed. `version` grows on every change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    pub id: ObjectId,
    pub version: i64,
}

/// A revision known to exist (or to have existed) in the cloud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionRow {
    pub rev_id: String,
    pub file_id: Option<String>,
    pub modified: i64,
    /// None until downloaded.
    pub parents: Option<Vec<String>>,
    /// None until downloaded, if unreadable, or after cleanup.
    pub payload: Option<ObjectPayload>,
}

/// What the local state of an object is based on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncObject {
    pub heads: Vec<String>,
    pub base: Option<ObjectPayload>,
}

impl Store {
    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn outbox(&self) -> Result<Vec<OutboxEntry>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT object_id, queued_at FROM outbox ORDER BY queued_at")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(OutboxEntry {
                    id: r.get(0)?,
                    version: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Queues every object that was never synced and isn't queued (notes
    /// written before sync existed, or after the sync state was reset).
    pub fn enqueue_never_synced(&mut self) -> Result<usize> {
        let now = self.now();
        let n = self.conn.execute(
            "INSERT INTO outbox (object_id, queued_at)
             SELECT id, ?1 FROM objects
             WHERE id NOT IN (SELECT object_id FROM sync_objects)
               AND id NOT IN (SELECT object_id FROM outbox)",
            [now],
        )?;
        Ok(n)
    }

    /// Every object on this device (including trashed ones and replies).
    pub fn all_object_ids(&self) -> Result<Vec<ObjectId>> {
        let mut stmt = self.conn.prepare("SELECT id FROM objects ORDER BY id")?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn outbox_version(&self, id: &ObjectId) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT queued_at FROM outbox WHERE object_id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The local state of an object as a payload; None if it doesn't exist.
    pub fn payload(&self, id: &ObjectId) -> Result<Option<ObjectPayload>> {
        let row = self
            .conn
            .query_row(
                "SELECT o.kind, o.parent_id, o.title, o.sort_key, coalesce(b.body, ''), o.tags,
                        o.created_at, o.updated_at, o.trashed_at, o.later_at, o.due_date, o.clocks
                 FROM objects o LEFT JOIN note_bodies b ON b.seq = o.seq WHERE o.id = ?1",
                [id],
                |r| {
                    Ok((
                        Kind::from_code(r.get(0)?)?,
                        r.get::<_, Option<ObjectId>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, Option<i64>>(8)?,
                        r.get::<_, Option<i64>>(9)?,
                        r.get::<_, Option<String>>(10)?,
                        r.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            kind,
            parent_id,
            title,
            sort_key,
            body,
            tags,
            created_at,
            updated_at,
            trashed_at,
            later_at,
            due_date,
            clocks,
        )) = row
        else {
            return Ok(None);
        };
        let attachments = self.attachment_refs(&body)?;
        Ok(Some(ObjectPayload {
            id: *id,
            kind,
            deleted: false,
            parent_id,
            sort_key,
            title,
            body,
            tags: parse_tags(&tags),
            created_at,
            updated_at,
            trashed_at,
            later_at,
            due_date,
            attachments,
            clocks: serde_json::from_str::<Clocks>(&clocks).unwrap_or_default(),
        }))
    }

    fn attachment_refs(&self, body: &str) -> Result<Vec<AttachmentRef>> {
        let mut refs = Vec::new();
        for id in attachment_ids(body) {
            if let Some(meta) = self.blob(&id)? {
                refs.push(AttachmentRef {
                    id: meta.id,
                    size: meta.size,
                    mime: meta.mime,
                });
            }
        }
        Ok(refs)
    }

    /// A tombstone for an object deleted on this device, built from what was
    /// last synced. None if it was never synced (nothing to delete remotely).
    pub fn tombstone(&self, id: &ObjectId, version: i64) -> Result<Option<ObjectPayload>> {
        let sync = self.sync_object(id)?;
        if sync.heads.is_empty() {
            return Ok(None);
        }
        let Some(mut payload) = sync.base else {
            return Ok(None);
        };
        payload.deleted = true;
        payload.clocks.deleted = Clock {
            at: version,
            device: self.device.clone(),
        };
        Ok(Some(payload))
    }

    pub fn sync_object(&self, id: &ObjectId) -> Result<SyncObject> {
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT heads, base FROM sync_objects WHERE object_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            None => SyncObject::default(),
            Some((heads, base)) => SyncObject {
                heads: from_json(&heads)?,
                base: base.as_deref().map(from_json).transpose()?,
            },
        })
    }

    // ---- Cloud index --------------------------------------------------

    /// Records a revision file seen in the cloud. Returns true if it's new.
    pub fn record_remote_revision(
        &mut self,
        object_id: &ObjectId,
        rev_id: &str,
        file_id: &str,
        modified: i64,
    ) -> Result<bool> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO revisions (rev_id, object_id, file_id, modified) VALUES (?1, ?2, ?3, ?4)",
            params![rev_id, object_id, file_id, modified],
        )?;
        if inserted == 0 {
            self.conn.execute(
                "UPDATE revisions SET file_id = ?2, modified = ?3 WHERE rev_id = ?1",
                params![rev_id, file_id, modified],
            )?;
        }
        Ok(inserted > 0)
    }

    pub fn record_remote_blob(
        &mut self,
        blob_id: &str,
        file_id: &str,
        modified: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO remote_blobs (blob_id, file_id, modified) VALUES (?1, ?2, ?3)",
            params![blob_id, file_id, modified],
        )?;
        Ok(())
    }

    /// The cloud file of an attachment, if it's known to be uploaded.
    pub fn remote_blob(&self, blob_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT file_id FROM remote_blobs WHERE blob_id = ?1",
                [blob_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// A cloud file disappeared (deleted by this or another device).
    pub fn forget_remote_file(&mut self, file_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE revisions SET file_id = NULL WHERE file_id = ?1",
            [file_id],
        )?;
        self.conn
            .execute("DELETE FROM remote_blobs WHERE file_id = ?1", [file_id])?;
        Ok(())
    }

    /// Forgets every cloud file not in `present` (after a full listing).
    pub fn retain_remote_files(&mut self, present: &HashSet<String>) -> Result<()> {
        let mut known: Vec<String> = self
            .conn
            .prepare("SELECT file_id FROM revisions WHERE file_id IS NOT NULL")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        known.extend(
            self.conn
                .prepare("SELECT file_id FROM remote_blobs")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
        for file in known.iter().filter(|f| !present.contains(*f)) {
            self.forget_remote_file(file)?;
        }
        Ok(())
    }

    /// Revisions whose file exists but whose content isn't downloaded yet.
    pub fn revisions_to_download(&self) -> Result<Vec<(String, ObjectId, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT rev_id, object_id, file_id FROM revisions
             WHERE file_id IS NOT NULL AND parents IS NULL",
        )?;
        Ok(stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Stores a downloaded revision. `payload` None means unreadable.
    pub fn set_revision_content(
        &mut self,
        rev_id: &str,
        parents: &[String],
        payload: Option<&ObjectPayload>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE revisions SET parents = ?2, payload = ?3 WHERE rev_id = ?1",
            params![rev_id, to_json(parents), payload.map(to_json)],
        )?;
        Ok(())
    }

    pub fn revisions_of(&self, object_id: &ObjectId) -> Result<Vec<RevisionRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT rev_id, file_id, modified, parents, payload FROM revisions WHERE object_id = ?1",
        )?;
        let rows: Vec<RawRevision> = stmt
            .query_map([object_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(rev_id, file_id, modified, parents, payload)| {
                Ok(RevisionRow {
                    rev_id,
                    file_id,
                    modified,
                    parents: parents.as_deref().map(from_json).transpose()?,
                    payload: payload.as_deref().map(from_json).transpose()?,
                })
            })
            .collect()
    }

    /// Objects with a readable head revision (nothing newer is based on it)
    /// that the local state doesn't incorporate yet: these need merging.
    pub fn objects_needing_merge(&self) -> Result<Vec<ObjectId>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT r.object_id FROM revisions r
             WHERE r.payload IS NOT NULL AND r.file_id IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM revisions c, json_each(c.parents) p
                               WHERE c.object_id = r.object_id AND p.value = r.rev_id)
               AND NOT EXISTS (SELECT 1 FROM sync_objects s, json_each(s.heads) h
                               WHERE s.object_id = r.object_id AND h.value = r.rev_id)",
        )?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Attachment ids referenced by current notes and replies.
    pub fn blobs_in_notes(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT body FROM note_bodies")?;
        let bodies = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(bodies.iter().flat_map(|b| attachment_ids(b)).collect())
    }

    /// Folders whose parent chain loops (possible after concurrent moves on
    /// two devices). Each cycle is listed once.
    pub fn folder_cycles(&self) -> Result<Vec<Vec<ObjectId>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, parent_id FROM objects WHERE kind = 1")?;
        let parents: std::collections::HashMap<ObjectId, Option<ObjectId>> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut cycles: Vec<Vec<ObjectId>> = Vec::new();
        let mut done: HashSet<ObjectId> = HashSet::new();
        for start in parents.keys() {
            let mut path: Vec<ObjectId> = Vec::new();
            let mut current = Some(*start);
            while let Some(id) = current {
                if done.contains(&id) {
                    break;
                }
                if let Some(pos) = path.iter().position(|p| *p == id) {
                    let mut cycle = path[pos..].to_vec();
                    cycle.sort();
                    if !cycles.contains(&cycle) {
                        cycles.push(cycle);
                    }
                    break;
                }
                path.push(id);
                current = parents.get(&id).copied().flatten();
            }
            done.extend(path);
        }
        Ok(cycles)
    }

    /// Objects whose parent was deleted (a note edited on one device while
    /// another deleted its folder). Parents not yet downloaded don't count.
    pub fn orphans(&self) -> Result<Vec<ObjectId>> {
        let mut stmt = self.conn.prepare(
            "SELECT o.id FROM objects o
             WHERE o.parent_id IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM objects p WHERE p.id = o.parent_id)
               AND EXISTS (SELECT 1 FROM sync_objects s
                           WHERE s.object_id = o.parent_id AND s.heads != '[]')
             ORDER BY o.id",
        )?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    // ---- Applying sync results ----------------------------------------

    /// Makes `payload` the local state of its object (or deletes the object
    /// for a tombstone) and records what it's based on. With `push`, the
    /// object is queued so a new revision gets uploaded; otherwise a pending
    /// queue entry no newer than `seen_version` is cleared.
    pub fn apply_synced(
        &mut self,
        payload: &ObjectPayload,
        sync: &SyncObject,
        push: bool,
        seen_version: Option<i64>,
    ) -> Result<()> {
        let now = self.now();
        // Text that sync replaces stays in history (same rule as local
        // edits), so another device's change can be undone here.
        let old = match self.note(&payload.id) {
            Ok(note) => Some(note),
            Err(StoreError::NotFound(_) | StoreError::NotANote(_)) => None,
            Err(e) => return Err(e),
        };
        let tx = self.conn.transaction()?;
        if let Some(old) = &old
            && !payload.deleted
            && (old.title != payload.title || old.body != payload.body)
        {
            snapshot_if_due(&tx, old, now)?;
        }
        let seq: Option<i64> = tx
            .query_row("SELECT seq FROM objects WHERE id = ?1", [payload.id], |r| {
                r.get(0)
            })
            .optional()?;
        if payload.deleted {
            if let Some(seq) = seq {
                tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [seq])?;
                tx.execute("DELETE FROM note_bodies WHERE seq = ?1", [seq])?;
                tx.execute("DELETE FROM history WHERE object_id = ?1", [payload.id])?;
                tx.execute("DELETE FROM objects WHERE seq = ?1", [seq])?;
            }
        } else {
            let tags = serde_json::to_string(&payload.tags).expect("tags serialize");
            let clocks = serde_json::to_string(&payload.clocks).expect("clocks serialize");
            tx.execute(
                "INSERT INTO objects (id, kind, parent_id, title, sort_key, tags, created_at,
                                      updated_at, trashed_at, later_at, due_date, clocks)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT (id) DO UPDATE SET
                     kind = excluded.kind, parent_id = excluded.parent_id, title = excluded.title,
                     sort_key = excluded.sort_key, tags = excluded.tags,
                     created_at = excluded.created_at, updated_at = excluded.updated_at,
                     trashed_at = excluded.trashed_at, later_at = excluded.later_at,
                     due_date = excluded.due_date, clocks = excluded.clocks",
                params![
                    payload.id,
                    payload.kind.code(),
                    payload.parent_id,
                    payload.title,
                    payload.sort_key,
                    tags,
                    payload.created_at,
                    payload.updated_at,
                    payload.trashed_at,
                    payload.later_at,
                    payload.due_date,
                    clocks
                ],
            )?;
            if payload.kind == Kind::Note {
                tx.execute(
                    "INSERT INTO note_bodies (seq, body) VALUES ((SELECT seq FROM objects WHERE id = ?1), ?2)
                     ON CONFLICT (seq) DO UPDATE SET body = excluded.body",
                    params![payload.id, payload.body],
                )?;
                index_note(&tx, &payload.id)?;
            }
            for a in &payload.attachments {
                tx.execute(
                    "INSERT OR IGNORE INTO blobs (id, size, mime, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![a.id, a.size as i64, a.mime, now],
                )?;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO sync_objects (object_id, heads, base) VALUES (?1, ?2, ?3)",
            params![
                payload.id,
                to_json(&sync.heads),
                sync.base.as_ref().map(to_json)
            ],
        )?;
        if push {
            super::enqueue(&tx, &payload.id, now)?;
        } else if let Some(version) = seen_version {
            tx.execute(
                "DELETE FROM outbox WHERE object_id = ?1 AND queued_at <= ?2",
                params![payload.id, version],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// After uploading revision `rev_id` of `payload`: records it as the new
    /// base, and clears the queue entry unless the object changed meanwhile.
    pub fn complete_push(
        &mut self,
        payload: &ObjectPayload,
        rev_id: &str,
        file_id: &str,
        modified: i64,
        parents: &[String],
        version: i64,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO revisions (rev_id, object_id, file_id, modified, parents, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![rev_id, payload.id, file_id, modified, to_json(parents), to_json(payload)],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO sync_objects (object_id, heads, base) VALUES (?1, ?2, ?3)",
            params![payload.id, to_json(&[rev_id]), to_json(payload)],
        )?;
        tx.execute(
            "DELETE FROM outbox WHERE object_id = ?1 AND queued_at <= ?2",
            params![payload.id, version],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Drops a queue entry that needs no upload (e.g. created and deleted
    /// before ever being synced).
    pub fn drop_from_outbox(&mut self, id: &ObjectId, version: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM outbox WHERE object_id = ?1 AND queued_at <= ?2",
            params![id, version],
        )?;
        Ok(())
    }

    // ---- Cleanup ------------------------------------------------------

    /// Revision files that a newer, downloaded revision replaced before `cutoff`.
    ///
    /// Ancestors come before their descendants: deleting in this order, an
    /// interrupted clean-up never leaves a revision in the cloud whose
    /// successors are gone (another device would take it for current).
    pub fn superseded_revisions(&self, cutoff: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.rev_id, r.file_id, coalesce(r.parents, '[]') FROM revisions r
             WHERE r.file_id IS NOT NULL AND EXISTS (
                 SELECT 1 FROM revisions c, json_each(c.parents) p
                 WHERE c.object_id = r.object_id AND p.value = r.rev_id AND c.modified < ?1)
             ORDER BY r.rev_id",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let parents: HashMap<&str, Vec<String>> = rows
            .iter()
            .map(|(rev, _, p)| Ok((rev.as_str(), from_json(p)?)))
            .collect::<Result<_>>()?;
        // Depth-first: a revision is listed after all its listed ancestors.
        let mut order = Vec::with_capacity(rows.len());
        let mut done: HashSet<&str> = HashSet::new();
        for (rev, _, _) in &rows {
            let mut stack = vec![(rev.as_str(), false)];
            while let Some((r, expanded)) = stack.pop() {
                if done.contains(r) {
                    continue;
                }
                if expanded {
                    done.insert(r);
                    order.push(r);
                    continue;
                }
                stack.push((r, true));
                for p in parents.get(r).into_iter().flatten() {
                    if let Some((key, _)) = parents.get_key_value(p.as_str())
                        && !done.contains(key)
                    {
                        stack.push((key, false));
                    }
                }
            }
        }
        let files: HashMap<&str, &str> = rows
            .iter()
            .map(|(rev, file, _)| (rev.as_str(), file.as_str()))
            .collect();
        Ok(order
            .into_iter()
            .map(|r| (r.to_owned(), files[r].to_owned()))
            .collect())
    }

    /// After deleting a superseded revision file: drop its content too,
    /// keeping only the ancestry.
    pub fn trim_revision(&mut self, rev_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE revisions SET file_id = NULL, payload = NULL WHERE rev_id = ?1",
            [rev_id],
        )?;
        Ok(())
    }

    /// Attachment ids still referenced by any note, reply, version in
    /// history or revision kept for merging.
    pub fn referenced_blobs(&self) -> Result<HashSet<String>> {
        let mut ids = HashSet::new();
        let mut collect = |sql: &str| -> Result<()> {
            let mut stmt = self.conn.prepare(sql)?;
            let texts = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for text in texts {
                ids.extend(attachment_ids(&text));
            }
            Ok(())
        };
        collect("SELECT body FROM note_bodies")?;
        collect("SELECT body FROM history")?;
        collect("SELECT payload FROM revisions WHERE payload IS NOT NULL")?;
        collect("SELECT base FROM sync_objects WHERE base IS NOT NULL")?;
        Ok(ids)
    }

    /// Attachments known locally or in the cloud, with when they appeared.
    pub fn all_blobs(&self) -> Result<Vec<(String, i64, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT b.id, b.created_at, r.file_id FROM blobs b LEFT JOIN remote_blobs r ON r.blob_id = b.id
             UNION
             SELECT r.blob_id, r.modified, r.file_id FROM remote_blobs r
             WHERE r.blob_id NOT IN (SELECT id FROM blobs)",
        )?;
        Ok(stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn forget_blob(&mut self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM blobs WHERE id = ?1", [id])?;
        self.conn
            .execute("DELETE FROM remote_blobs WHERE blob_id = ?1", [id])?;
        Ok(())
    }
}

/// A `revisions` row as stored: rev id, file id, modified, parents JSON, payload JSON.
type RawRevision = (String, Option<String>, i64, Option<String>, Option<String>);

fn to_json<T: serde::Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).expect("sync state serializes")
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| StoreError::Corrupt(format!("sync state: {e}")))
}
