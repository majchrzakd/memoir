//! One sync round: pull what changed in the cloud, merge it with local
//! changes, push local changes, clean up. Safe to interrupt at any point:
//! every step is idempotent, and the next round picks up where this one
//! stopped.
//!
//! Cloud layout: `objects/<object id>.<revision id>.mmr` (one file per
//! revision, never overwritten; see [`RevisionFile`]) and
//! `blobs/<blob id>.mmr` (attachments, uploaded as stored locally).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ulid::Ulid;

use crate::blobs::{BlobError, BlobStore, is_valid_id};
use crate::crypto::{Keyring, envelope};
use crate::remote::{Area, ProviderError, RemoteFile, StorageProvider};
use crate::store::{ObjectId, RevisionRow, Store, StoreError, SyncObject};
use crate::sync::payload::{Clock, ObjectPayload, REVISION_FORMAT, RevisionFile, merge};

/// Access to this device's data. The engine never holds it across network
/// calls, so the UI stays responsive while syncing.
pub trait Local {
    fn with<R>(&self, f: impl FnOnce(&mut Store, &BlobStore) -> R) -> Result<R>;
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Blob(#[from] BlobError),
    #[error("Memoir is locked")]
    Locked,
}

impl SyncError {
    /// Worth retrying later without user action.
    pub fn is_transient(&self) -> bool {
        matches!(self, SyncError::Provider(e) if e.is_transient())
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// A revision replaced by a newer one is deleted after this long, so
    /// devices that were offline can still merge against it.
    pub revision_grace_ms: i64,
    /// An attachment no longer referenced anywhere is deleted after this long.
    pub blob_grace_ms: i64,
    /// How often to list the cloud fully instead of following the change feed.
    pub full_list_every_ms: i64,
    /// How often to clean up.
    pub cleanup_every_ms: i64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        const HOUR: i64 = 3_600_000;
        Self {
            revision_grace_ms: 48 * HOUR,
            blob_grace_ms: 7 * 24 * HOUR,
            full_list_every_ms: 24 * HOUR,
            cleanup_every_ms: HOUR,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub downloaded: usize,
    pub uploaded: usize,
    pub conflicts: usize,
    pub cleaned: usize,
    /// Local objects changed by this round (the UI reloads them).
    pub changed: Vec<ObjectId>,
}

/// Keys in the store's `remote_state`.
mod key {
    pub const CURSOR: &str = "sync.cursor";
    pub const LISTED_AT: &str = "sync.listed_at";
    pub const CLEANED_AT: &str = "sync.cleaned_at";
}

pub struct SyncEngine<P> {
    provider: Arc<P>,
    keys: Keyring,
    config: SyncConfig,
    now: Box<dyn Fn() -> i64 + Send + Sync>,
}

struct Prepared {
    payload: ObjectPayload,
    parents: Vec<String>,
    version: i64,
    blobs: Vec<(String, Vec<u8>)>,
}

impl<P: StorageProvider> SyncEngine<P> {
    pub fn new(provider: Arc<P>, keys: Keyring, config: SyncConfig) -> Self {
        Self {
            provider,
            keys,
            config,
            now: Box::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64)
            }),
        }
    }

    /// Replaces the clock (tests).
    pub fn with_clock(mut self, now: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.now = Box::new(now);
        self
    }

    pub async fn sync(&self, local: &impl Local) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        self.pull(local, &mut report).await?;
        local.with(|store, _| self.merge_all(store, &mut report))??;
        self.fetch_attachments(local).await?;
        self.push(local, &mut report).await?;
        self.clean_up(local, &mut report).await?;
        Ok(report)
    }

    /// Brings this device up to date with the cloud without writing to it:
    /// pull, merge, download attachments. For reading a copy of the cloud
    /// folder (the recovery tool).
    pub async fn pull_only(&self, local: &impl Local) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        self.pull(local, &mut report).await?;
        local.with(|store, _| self.merge_all(store, &mut report))??;
        self.fetch_attachments(local).await?;
        Ok(report)
    }

    // ---- Pull ---------------------------------------------------------

    async fn pull(&self, local: &impl Local, report: &mut SyncReport) -> Result<()> {
        let now = (self.now)();
        let (cursor, listed_at) = local.with(|s, _| -> Result<_> {
            Ok((
                s.remote_value(key::CURSOR)?,
                s.remote_value(key::LISTED_AT)?
                    .and_then(|v| v.parse::<i64>().ok()),
            ))
        })??;
        let full = match (&cursor, listed_at) {
            (Some(_), Some(at)) => now - at >= self.config.full_list_every_ms,
            _ => true,
        };

        if full {
            // Cursor first: changes during the listing then show up next time.
            let cursor = self.provider.cursor().await?;
            let mut files = self.provider.list(Area::Objects).await?;
            files.extend(self.provider.list(Area::Blobs).await?);
            local.with(|s, _| -> Result<()> {
                let present: HashSet<String> = files.iter().map(|f| f.id.clone()).collect();
                for f in &files {
                    index_file(s, f)?;
                }
                s.retain_remote_files(&present)?;
                s.set_remote_values(&[(key::CURSOR, &cursor), (key::LISTED_AT, &now.to_string())])?;
                Ok(())
            })??;
        } else {
            let changes = self
                .provider
                .changes(cursor.as_deref().unwrap_or_default())
                .await?;
            local.with(|s, _| -> Result<()> {
                for f in &changes.files {
                    index_file(s, f)?;
                }
                for removed in &changes.removed {
                    s.forget_remote_file(removed)?;
                }
                s.set_remote_values(&[(key::CURSOR, &changes.cursor)])?;
                Ok(())
            })??;
        }

        let todo = local.with(|s, _| s.revisions_to_download())??;
        for (rev_id, object_id, file_id) in todo {
            match self.provider.download(&file_id).await {
                Ok(bytes) => {
                    let file = self.open_revision(&object_id, &rev_id, &bytes);
                    if file.is_none() {
                        log::warn!("revision {object_id}.{rev_id} is unreadable; ignoring it");
                    }
                    local.with(|s, _| match &file {
                        Some(f) => s.set_revision_content(&rev_id, &f.parents, Some(&f.object)),
                        None => s.set_revision_content(&rev_id, &[], None),
                    })??;
                    report.downloaded += 1;
                }
                Err(ProviderError::NotFound(_)) => {
                    local.with(|s, _| s.forget_remote_file(&file_id))??
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn open_revision(
        &self,
        object_id: &ObjectId,
        rev_id: &str,
        bytes: &[u8],
    ) -> Option<RevisionFile> {
        let plain = envelope::open(&self.keys, &context(object_id, rev_id), bytes).ok()?;
        let file: RevisionFile = serde_json::from_slice(&plain).ok()?;
        (file.v == REVISION_FORMAT && file.object.id == *object_id).then_some(file)
    }

    // ---- Merge --------------------------------------------------------

    fn merge_all(&self, store: &mut Store, report: &mut SyncReport) -> Result<()> {
        for id in store.objects_needing_merge()? {
            self.merge_object(store, &id, report)?;
        }
        self.break_cycles(store, report)?;
        self.rescue_orphans(store, report)
    }

    fn merge_object(
        &self,
        store: &mut Store,
        id: &ObjectId,
        report: &mut SyncReport,
    ) -> Result<()> {
        let revisions = store.revisions_of(id)?;
        let by_id: HashMap<&str, &RevisionRow> =
            revisions.iter().map(|r| (r.rev_id.as_str(), r)).collect();
        let replaced: HashSet<&str> = revisions
            .iter()
            .flat_map(|r| r.parents.iter().flatten())
            .map(String::as_str)
            .collect();
        // A revision whose file is gone was cleaned up, which happens only to
        // superseded ones (even if this device never saw what replaced it).
        let mut heads: Vec<&RevisionRow> = revisions
            .iter()
            .filter(|r| {
                r.payload.is_some() && r.file_id.is_some() && !replaced.contains(r.rev_id.as_str())
            })
            .collect();
        heads.sort_by(|a, b| a.rev_id.cmp(&b.rev_id));
        let Some(first) = heads.first() else {
            return Ok(());
        };

        // Combine concurrent heads, always in the same order on every device.
        let mut state = first.payload.clone().expect("heads have payloads");
        let mut merged: Vec<&str> = vec![&first.rev_id];
        let mut copies = Vec::new();
        for head in &heads[1..] {
            let base = common_ancestor(&by_id, &merged, &head.rev_id);
            let m = merge(
                base.as_ref(),
                &state,
                head.payload.as_ref().expect("heads have payloads"),
            );
            state = m.payload;
            copies.extend(m.conflict_copy);
            merged.push(&head.rev_id);
        }

        // Then local changes not yet pushed, against what they were based on.
        let sync = store.sync_object(id)?;
        let version = store.outbox_version(id)?;
        let current = store.payload(id)?;
        let result = match version {
            None => state.clone(),
            Some(v) => {
                let mine = match &current {
                    Some(p) => Some(p.clone()),
                    None => store.tombstone(id, v)?,
                };
                match mine {
                    Some(mine) => {
                        let m = merge(sync.base.as_ref(), &mine, &state);
                        copies.extend(m.conflict_copy);
                        m.payload
                    }
                    None => state.clone(),
                }
            }
        };

        // A new revision is needed only if no head already says this.
        let push = !heads.iter().any(|h| h.payload.as_ref() == Some(&result));
        let synced = SyncObject {
            heads: heads.iter().map(|h| h.rev_id.clone()).collect(),
            base: Some(if push { state } else { result.clone() }),
        };
        let changed = match &current {
            Some(c) => *c != result,
            None => !result.deleted,
        };
        store.apply_synced(&result, &synced, push, version)?;
        if changed {
            report.changed.push(*id);
        }

        for copy in copies {
            if store.payload(&copy.id)?.is_none() && store.sync_object(&copy.id)?.heads.is_empty() {
                store.apply_synced(&copy, &SyncObject::default(), true, None)?;
                report.changed.push(copy.id);
                report.conflicts += 1;
            }
        }
        Ok(())
    }

    /// Concurrent moves can make folders contain each other. Every device
    /// breaks such a cycle the same way: the folder moved most recently goes
    /// to the top level.
    fn break_cycles(&self, store: &mut Store, report: &mut SyncReport) -> Result<()> {
        for cycle in store.folder_cycles()? {
            let mut members = Vec::new();
            for id in &cycle {
                if let Some(p) = store.payload(id)? {
                    members.push(p);
                }
            }
            let Some(mut loser) = members
                .into_iter()
                .max_by(|a, b| (&a.clocks.parent, a.id).cmp(&(&b.clocks.parent, b.id)))
            else {
                continue;
            };
            loser.parent_id = None;
            // Same value and clock on every device, later than the move.
            loser.clocks.parent = Clock {
                at: loser.clocks.parent.at,
                device: "~cycle".into(),
            };
            let sync = store.sync_object(&loser.id)?;
            let version = store.outbox_version(&loser.id)?;
            store.apply_synced(&loser, &sync, true, version)?;
            report.changed.push(loser.id);
        }
        Ok(())
    }

    /// A note (or folder, or reply) whose parent was deleted on another
    /// device moves to the top level, the same way on every device.
    fn rescue_orphans(&self, store: &mut Store, report: &mut SyncReport) -> Result<()> {
        for id in store.orphans()? {
            let Some(mut orphan) = store.payload(&id)? else {
                continue;
            };
            orphan.parent_id = None;
            orphan.clocks.parent = Clock {
                at: orphan.clocks.parent.at,
                device: "~orphan".into(),
            };
            let sync = store.sync_object(&id)?;
            let version = store.outbox_version(&id)?;
            store.apply_synced(&orphan, &sync, true, version)?;
            report.changed.push(id);
        }
        Ok(())
    }

    // ---- Attachments ----------------------------------------------------

    /// Downloads attachments that notes refer to but this device lacks.
    async fn fetch_attachments(&self, local: &impl Local) -> Result<()> {
        let missing = local.with(|s, blobs| -> Result<Vec<(String, String)>> {
            let mut out = Vec::new();
            for id in s.blobs_in_notes()? {
                if !blobs.contains(&id)
                    && let Some(file) = s.remote_blob(&id)?
                {
                    out.push((id, file));
                }
            }
            Ok(out)
        })??;
        for (id, file) in missing {
            match self.provider.download(&file).await {
                Ok(bytes) => match local.with(|_, blobs| blobs.import(&id, &bytes))? {
                    Ok(()) => {}
                    Err(BlobError::Crypto(e)) => {
                        log::warn!("attachment {id} from the cloud is unreadable: {e}");
                    }
                    Err(e) => return Err(e.into()),
                },
                Err(ProviderError::NotFound(_)) => {
                    local.with(|s, _| s.forget_remote_file(&file))??
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    // ---- Push ---------------------------------------------------------

    async fn push(&self, local: &impl Local, report: &mut SyncReport) -> Result<()> {
        let queue = local.with(|s, _| -> Result<_> {
            s.enqueue_never_synced()?;
            Ok(s.outbox()?)
        })??;
        for entry in queue {
            let prepared = local.with(|s, blobs| prepare(s, blobs, &entry.id))??;
            let Some(p) = prepared else { continue };

            // Attachments first, so a revision never refers to a missing one.
            for (blob_id, bytes) in p.blobs {
                let file = self
                    .provider
                    .upload(Area::Blobs, &format!("{blob_id}.mmr"), bytes)
                    .await?;
                local.with(|s, _| s.record_remote_blob(&blob_id, &file.id, file.modified))??;
            }

            let rev_id = Ulid::generate().to_string();
            let content = RevisionFile {
                v: REVISION_FORMAT,
                parents: p.parents.clone(),
                object: p.payload.clone(),
            };
            let sealed = envelope::seal(
                &self.keys,
                &context(&p.payload.id, &rev_id),
                &serde_json::to_vec(&content).expect("revision serializes"),
            );
            let file = self
                .provider
                .upload(
                    Area::Objects,
                    &format!("{}.{rev_id}.mmr", p.payload.id),
                    sealed,
                )
                .await?;
            local.with(|s, _| {
                s.complete_push(
                    &p.payload,
                    &rev_id,
                    &file.id,
                    file.modified,
                    &p.parents,
                    p.version,
                )
            })??;
            report.uploaded += 1;
        }
        Ok(())
    }

    // ---- Clean-up -----------------------------------------------------

    async fn clean_up(&self, local: &impl Local, report: &mut SyncReport) -> Result<()> {
        let now = (self.now)();
        let (last, idle) = local.with(|s, _| -> Result<_> {
            Ok((
                s.remote_value(key::CLEANED_AT)?
                    .and_then(|v| v.parse::<i64>().ok()),
                s.pending_changes()? == 0,
            ))
        })??;
        if last.is_some_and(|at| now - at < self.config.cleanup_every_ms) {
            return Ok(());
        }

        let superseded =
            local.with(|s, _| s.superseded_revisions(now - self.config.revision_grace_ms))??;
        for (rev_id, file_id) in superseded {
            match self.provider.delete(&file_id).await {
                Ok(()) | Err(ProviderError::NotFound(_)) => {}
                Err(e) => return Err(e.into()),
            }
            local.with(|s, _| s.trim_revision(&rev_id))??;
            report.cleaned += 1;
        }

        // Unreferenced attachments, only when nothing local is unsynced (a
        // pending note might be about to reference one).
        if idle {
            let (referenced, all) = local
                .with(|s, _| -> Result<_> { Ok((s.referenced_blobs()?, s.all_blobs()?)) })??;
            for (id, seen_at, file) in all {
                if referenced.contains(&id) || now - seen_at < self.config.blob_grace_ms {
                    continue;
                }
                if let Some(file) = file {
                    match self.provider.delete(&file).await {
                        Ok(()) | Err(ProviderError::NotFound(_)) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                local.with(|s, blobs| -> Result<()> {
                    blobs.remove(&id)?;
                    s.forget_blob(&id)?;
                    Ok(())
                })??;
                report.cleaned += 1;
            }
        }
        local.with(|s, _| s.set_remote_values(&[(key::CLEANED_AT, &now.to_string())]))??;
        Ok(())
    }
}

/// What to upload for a queued object (None: nothing to do).
fn prepare(store: &mut Store, blobs: &BlobStore, id: &ObjectId) -> Result<Option<Prepared>> {
    let Some(version) = store.outbox_version(id)? else {
        return Ok(None);
    };
    let payload = match store.payload(id)? {
        Some(p) => p,
        None => match store.tombstone(id, version)? {
            Some(t) => t,
            None => {
                // Created and deleted without ever being synced.
                store.drop_from_outbox(id, version)?;
                return Ok(None);
            }
        },
    };
    let mut uploads = Vec::new();
    if !payload.deleted {
        for a in &payload.attachments {
            if store.remote_blob(&a.id)?.is_none() {
                match blobs.encrypted(&a.id) {
                    Ok(bytes) => uploads.push((a.id.clone(), bytes)),
                    // Referenced but not on this device (yet): another device has it.
                    Err(BlobError::NotFound(_)) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Ok(Some(Prepared {
        parents: store.sync_object(id)?.heads,
        payload,
        version,
        blobs: uploads,
    }))
}

/// Records a cloud file in the local index.
fn index_file(store: &mut Store, file: &RemoteFile) -> Result<()> {
    let Some(stem) = file.name.strip_suffix(".mmr") else {
        return Ok(());
    };
    match file.area {
        Area::Objects => {
            if let Some((object, rev)) = stem.split_once('.')
                && let Ok(object) = object.parse::<ObjectId>()
                && rev.parse::<Ulid>().is_ok()
            {
                store.record_remote_revision(&object, rev, &file.id, file.modified)?;
            }
        }
        Area::Blobs if is_valid_id(stem) => {
            store.record_remote_blob(stem, &file.id, file.modified)?
        }
        _ => {}
    }
    Ok(())
}

/// Binds a revision file to its object and revision id.
fn context(object_id: &ObjectId, rev_id: &str) -> Vec<u8> {
    format!("object:{object_id}:{rev_id}").into_bytes()
}

/// The payload of the best common ancestor of the `merged` heads and `head`
/// (None if unknown, e.g. already cleaned up).
fn common_ancestor(
    by_id: &HashMap<&str, &RevisionRow>,
    merged: &[&str],
    head: &str,
) -> Option<ObjectPayload> {
    let ancestors = |starts: &[&str]| -> HashSet<String> {
        let mut seen = HashSet::new();
        let mut queue: VecDeque<String> = starts.iter().map(|s| (*s).to_owned()).collect();
        while let Some(rev) = queue.pop_front() {
            if !seen.insert(rev.clone()) {
                continue;
            }
            if let Some(parents) = by_id.get(rev.as_str()).and_then(|r| r.parents.as_ref()) {
                queue.extend(parents.iter().cloned());
            }
        }
        seen
    };
    let ours = ancestors(merged);
    let theirs = ancestors(&[head]);
    let common: Vec<&String> = ours.intersection(&theirs).collect();
    // Lowest common ancestors: not an ancestor of another common one.
    let mut lowest: Vec<&String> = common
        .iter()
        .filter(|c| {
            !common
                .iter()
                .any(|o| o != *c && ancestors(&[o.as_str()]).contains(c.as_str()))
        })
        .copied()
        .collect();
    lowest.sort();
    lowest
        .iter()
        .rev()
        .find_map(|rev| by_id.get(rev.as_str()).and_then(|r| r.payload.clone()))
}
